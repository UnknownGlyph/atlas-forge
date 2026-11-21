//! Persistent project versions. Metadata and the native document share one
//! atomic file so a partially committed record never appears in the list.
use crate::{project, recovery};
use map_core::Document;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8; 8] = b"ATLSHST1";
const EXT: &str = "atlaspoint";
const MAX_HEADER: usize = 128 << 10;
static NEXT: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Manual,
    Automatic,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub title: String,
    pub saved_at: u64,
    pub kind: Kind,
    pub board: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store {
    root: PathBuf,
    draft: Option<String>,
}

fn stamp() -> (u64, String) {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let nonce = NEXT.fetch_add(1, Ordering::Relaxed);
    (
        elapsed.as_secs(),
        format!(
            "{:024}-{:010}-{:016}",
            elapsed.as_nanos(),
            std::process::id(),
            nonce
        ),
    )
}

fn valid_key(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 76
        && id
            .as_bytes()
            .iter()
            .all(|c| c.is_ascii_digit() || *c == b'-')
}

impl Store {
    pub fn draft() -> Self {
        Self::from_draft(&stamp().1).expect("generated draft key")
    }

    pub fn draft_in(recovery_directory: &Path) -> Self {
        Self::from_draft_in(recovery_directory, &stamp().1).expect("generated draft key")
    }

    pub fn from_draft(key: &str) -> Option<Self> {
        Self::from_draft_in(&recovery::default_dir(), key)
    }

    pub fn from_draft_in(recovery_directory: &Path, key: &str) -> Option<Self> {
        valid_key(key).then(|| Self {
            root: recovery_directory.join("draft-history").join(key),
            draft: Some(key.to_owned()),
        })
    }

    pub fn for_project(path: &Path) -> Self {
        // Canonicalize aliases so the same saved project shares one history.
        let path = fs::canonicalize(path).unwrap_or_else(|_| path.to_owned());
        let mut sidecar = path.as_os_str().to_os_string();
        sidecar.push(".history");
        Self {
            root: PathBuf::from(sidecar),
            draft: None,
        }
    }

    pub fn directory(&self) -> &Path {
        &self.root
    }

    pub fn draft_key(&self) -> Option<&str> {
        self.draft.as_deref()
    }

    fn path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_key(id) {
            return Err("保存点标识无效".into());
        }
        Ok(self.root.join(format!("{id}.{EXT}")))
    }

    fn lock(&self) -> io::Result<File> {
        fs::create_dir_all(&self.root)?;
        let lock = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".history.lock"))?;
        lock.lock()?;
        Ok(lock)
    }

    fn read_header(file: &mut File) -> Result<Entry, String> {
        let mut prefix = [0u8; 12];
        file.read_exact(&mut prefix).map_err(|e| e.to_string())?;
        if &prefix[..8] != MAGIC {
            return Err("文件不是 Atlas Forge 保存点".into());
        }
        let size = u32::from_le_bytes(prefix[8..].try_into().unwrap()) as usize;
        if size > MAX_HEADER {
            return Err("保存点元数据长度无效".into());
        }
        let mut buf = vec![0; size];
        file.read_exact(&mut buf).map_err(|e| e.to_string())?;
        serde_json::from_slice(&buf).map_err(|e| e.to_string())
    }

    pub fn list(&self) -> Result<Vec<Entry>, String> {
        let listing = match fs::read_dir(&self.root) {
            Ok(listing) => listing,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.to_string()),
        };
        let mut entries = Vec::new();
        for item in listing {
            let file = item.map_err(|e| e.to_string())?.path();
            if file.extension().and_then(|s| s.to_str()) != Some(EXT) {
                continue;
            }
            let entry = File::open(&file)
                .ok()
                .and_then(|mut f| Self::read_header(&mut f).ok());
            if let Some(entry) = entry.filter(|entry| {
                file.file_stem().and_then(|s| s.to_str()) == Some(&entry.id) && valid_key(&entry.id)
            }) {
                entries.push(entry);
            }
        }
        entries.sort_by(|a, b| b.id.cmp(&a.id));
        Ok(entries)
    }

    pub fn capture(
        &self,
        doc: &Document,
        kind: Kind,
        title: &str,
        keep: usize,
    ) -> Result<Entry, String> {
        let (saved_at, id) = stamp();
        let entry = Entry {
            id,
            title: title.trim().to_owned(),
            saved_at,
            kind,
            board: doc.meta.name.clone(),
        };
        let payload = project::to_bytes(doc)?;
        self.put(&entry, &payload, keep)?;
        Ok(entry)
    }

    pub fn capture_encoded(&self, board: &str, payload: &[u8], keep: usize) -> Result<(), String> {
        let (saved_at, id) = stamp();
        self.put(
            &Entry {
                id,
                title: "自动保存".into(),
                saved_at,
                kind: Kind::Automatic,
                board: board.into(),
            },
            payload,
            keep,
        )
    }

    fn put(&self, entry: &Entry, payload: &[u8], keep: usize) -> Result<(), String> {
        let header = serde_json::to_vec(entry).map_err(|e| e.to_string())?;
        if header.len() > MAX_HEADER {
            return Err("保存点名称过长".into());
        }
        let mut bytes = Vec::with_capacity(12 + header.len() + payload.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&(header.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&header);
        bytes.extend_from_slice(payload);
        let _lock = self.lock().map_err(|e| e.to_string())?;
        recovery::write_atomic(&self.path(&entry.id)?, &bytes).map_err(|e| e.to_string())?;
        if entry.kind == Kind::Automatic {
            self.trim_locked(keep)?;
        }
        Ok(())
    }

    fn trim_locked(&self, keep: usize) -> Result<(), String> {
        for item in self
            .list()?
            .into_iter()
            .filter(|entry| entry.kind == Kind::Automatic)
            .skip(keep)
        {
            fs::remove_file(self.path(&item.id)?).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn trim(&self, keep: usize) -> Result<(), String> {
        if !self.root.exists() {
            return Ok(());
        }
        let _lock = self.lock().map_err(|e| e.to_string())?;
        self.trim_locked(keep)
    }

    pub fn load(&self, id: &str) -> Result<Document, String> {
        let path = self.path(id)?;
        let mut file = File::open(&path).map_err(|e| e.to_string())?;
        let entry = Self::read_header(&mut file)?;
        if entry.id != id {
            return Err("保存点标识与文件不一致".into());
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        project::from_bytes(&bytes)
    }

    pub fn delete(&self, id: &str) -> Result<(), String> {
        let path = self.path(id)?;
        let _lock = self.lock().map_err(|e| e.to_string())?;
        fs::remove_file(path).map_err(|e| e.to_string())
    }

    /// Attach all draft versions to the first saved file. Move no records
    /// until the destination is complete so a failure is recoverable.
    pub fn copy_draft_to(&self, destination: &Self, keep: usize) -> Result<(), String> {
        if self.draft.is_none() || self.root == destination.root {
            return Ok(());
        }
        let entries = self.list()?;
        if entries.is_empty() {
            return Ok(());
        }
        let _lock = destination.lock().map_err(|e| e.to_string())?;
        for entry in entries {
            let out = destination.path(&entry.id)?;
            if out.exists() {
                continue;
            }
            let bytes = fs::read(self.path(&entry.id)?).map_err(|e| e.to_string())?;
            recovery::write_atomic(&out, &bytes).map_err(|e| e.to_string())?;
        }
        destination.trim_locked(keep)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_and_migration_preserve_named_versions_and_order() {
        let root = std::env::temp_dir().join(format!("atlas-version-test-{}", stamp().1));
        let draft = Store {
            root: root.join("draft"),
            draft: Some("1-2".into()),
        };
        let saved = Store::for_project(&root.join("世界.atlas"));
        let mut doc = Document::new("起稿", 9650.0);
        let manual = draft.capture(&doc, Kind::Manual, "第一条海岸", 2).unwrap();
        for i in 1..=4 {
            doc.meta.name = format!("修订 {i}");
            draft.capture(&doc, Kind::Automatic, "自动保存", 2).unwrap();
        }
        assert_eq!(
            draft
                .list()
                .unwrap()
                .iter()
                .map(|r| r.board.as_str())
                .collect::<Vec<_>>(),
            ["修订 4", "修订 3", "起稿"]
        );
        draft.copy_draft_to(&saved, 2).unwrap();
        let reopened = Store::for_project(&root.join("世界.atlas"));
        assert_eq!(reopened.load(&manual.id).unwrap().meta.name, "起稿");
        assert_eq!(reopened.list().unwrap().len(), 3);
        reopened.trim(1).unwrap();
        assert_eq!(reopened.list().unwrap().len(), 2);
        assert!(reopened.load("../x").is_err());
        reopened.delete(&manual.id).unwrap();
        assert!(reopened.load(&manual.id).is_err());
        let _ = fs::remove_dir_all(root);
    }
}
