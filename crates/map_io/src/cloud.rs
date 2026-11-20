//! Versioned autosave copies in folders synchronized by the desktop clients.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    OneDrive,
    Nutstore,
}

impl Provider {
    pub const ALL: [Self; 2] = [Self::OneDrive, Self::Nutstore];
    pub fn label(self) -> &'static str {
        match self {
            Self::OneDrive => "OneDrive",
            Self::Nutstore => "坚果云",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub one_drive: Option<PathBuf>,
    pub nutstore: Option<PathBuf>,
}

impl Config {
    pub fn folder(&self, provider: Provider) -> Option<&Path> {
        match provider {
            Provider::OneDrive => self.one_drive.as_deref(),
            Provider::Nutstore => self.nutstore.as_deref(),
        }
    }
    pub fn folder_mut(&mut self, provider: Provider) -> &mut Option<PathBuf> {
        match provider {
            Provider::OneDrive => &mut self.one_drive,
            Provider::Nutstore => &mut self.nutstore,
        }
    }
    pub fn is_empty(&self) -> bool {
        self.one_drive.is_none() && self.nutstore.is_none()
    }
}

fn label(name: &str) -> String {
    let normalized: String = name
        .chars()
        .take(52)
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let normalized = normalized.trim_matches('_');
    if normalized.is_empty() {
        "World".into()
    } else {
        normalized.into()
    }
}

/// Store a fresh native project file. An older backup is never overwritten
/// or removed, and a partial write is not offered as a project file.
pub fn write(folder: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    if !folder.is_dir() {
        return Err("所选同步目录在本机不可用。".into());
    }
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |t| t.as_nanos());
    let sequence = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let dir = folder
        .join("Atlas Forge Backups")
        .join(format!("Atlas-{}", label(name)));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let base = format!(
        "version-{stamp:024}-{}-{sequence}.atlas",
        std::process::id()
    );
    let path = dir.join(&base);
    if path.exists() {
        return Err("同步目录已有同名备份，请重试。".into());
    }
    let temp = dir.join(format!(".{base}.tmp"));
    let mut f = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let result = (|| {
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&temp, &path)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&temp);
        return Err(e.to_string());
    }
    Ok(path)
}

pub fn suggested_onedrive() -> Option<PathBuf> {
    ["OneDrive", "OneDriveCommercial", "OneDriveConsumer"]
        .into_iter()
        .filter_map(std::env::var_os)
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn immutable_sync_copies_are_independently_restorable() {
        let folder = std::env::temp_dir().join(format!("atlas-sync-check-{}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        let mut d = map_core::Document::default();
        d.meta.name = "拂晓".into();
        let first = write(
            &folder,
            "../坏:路径",
            &crate::project::to_bytes(&d).unwrap(),
        )
        .unwrap();
        d.meta.name = "远方".into();
        let second = write(
            &folder,
            "../坏:路径",
            &crate::project::to_bytes(&d).unwrap(),
        )
        .unwrap();
        assert_ne!(first, second);
        assert!(first.starts_with(&folder) && second.starts_with(&folder));
        assert_eq!(crate::project::load(&first).unwrap().meta.name, "拂晓");
        assert_eq!(crate::project::load(&second).unwrap().meta.name, "远方");
        assert!(write(&folder.join("absent"), "World", b"").is_err());
        fs::remove_file(first).unwrap();
        fs::remove_file(second).unwrap();
    }
}
