//! Crash recovery.
//!
//! Each running session owns `<dir>/<session>.lock`, held with an exclusive
//! OS file lock for the life of the process, and periodically writes
//! `<session>.atlas` (the document) plus `<session>.json` (metadata).
//! A clean exit removes all three. After a crash the OS releases the lock,
//! so the next launch can tell orphaned sessions (lock acquirable) from
//! sessions of other instances that are still running (lock held).

use crate::project;
use map_core::doc::Document;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecoveryInfo {
    pub session: String,
    /// Project file the document belongs to (None = never saved).
    pub original: Option<PathBuf>,
    pub name: String,
    /// Seconds since the Unix epoch.
    pub saved_at: u64,
    /// The crash log, when the snapshot was written as the program crashed.
    #[serde(default)]
    pub crash_log: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_history: Option<String>,
}

pub struct Recovery {
    dir: PathBuf,
    session: String,
    _lock: File,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Default location: `%LOCALAPPDATA%\AtlasForge\recovery` (or the platform
/// equivalent), falling back to the temp directory.
pub fn default_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_STATE_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local").join("state")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("AtlasForge").join("recovery")
}

/// Where crash logs are kept (next to the recovery folder; never cleared
/// by the program).
pub fn crash_dir() -> PathBuf {
    let rec = default_dir();
    rec.parent().map(|p| p.join("crash")).unwrap_or_else(|| rec.join("crash"))
}

/// Write a crash log into `dir` and return its path.
pub fn write_crash_log(dir: &Path, text: &str) -> std::io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("crash-{}-{}.txt", now_secs(), std::process::id()));
    fs::write(&path, text)?;
    Ok(path)
}

/// Flush a new file next to `path`, then atomically replace `path`.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::{ErrorKind, Write};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let pid = std::process::id();
    let (temporary, mut file) = loop {
        let number = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".atlas-write-{pid}-{number}.atlas.tmp"));
        match File::options().write(true).create_new(true).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    let result = written.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

impl Recovery {
    pub fn start(dir: &Path) -> std::io::Result<Self> {
        fs::create_dir_all(dir)?;
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let session = format!("{}-{}", std::process::id(), nanos % 1_000_000_000_000);
        let lock = File::create(dir.join(format!("{session}.lock")))?;
        lock.try_lock().map_err(|e| std::io::Error::other(format!("cannot lock recovery session: {e}")))?;
        let me = Self { dir: dir.to_path_buf(), session, _lock: lock };
        me.sweep();
        Ok(me)
    }

    /// Remove the lock files of sessions that ended without a clean exit
    /// and left no snapshot (nothing to offer): they would pile up.
    fn sweep(&self) {
        let Ok(rd) = fs::read_dir(&self.dir) else { return };
        for p in rd.flatten().map(|e| e.path()) {
            let Some(session) = p.file_stem().and_then(|s| s.to_str()) else { continue };
            if p.extension().and_then(|e| e.to_str()) != Some("lock") || session == self.session || self.path(session, "atlas").exists() {
                continue;
            }
            // only if its owner is gone (the lock can be taken)
            let free = File::options().read(true).write(true).open(&p).is_ok_and(|f| f.try_lock().is_ok());
            if free {
                let _ = fs::remove_file(&p);
            }
        }
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    fn path(&self, session: &str, ext: &str) -> PathBuf {
        self.dir.join(format!("{session}.{ext}"))
    }

    /// Store a recovery snapshot of `doc` (call from a background thread).
    pub fn write(&self, doc: &Document, original: Option<PathBuf>) -> Result<(), String> {
        self.write_with(doc, original, None)
    }

    /// As [`Recovery::write`], from a crash: the snapshot names its log.
    pub fn write_with(&self, doc: &Document, original: Option<PathBuf>, crash_log: Option<PathBuf>) -> Result<(), String> {
        let bytes = project::to_bytes(doc)?;
        let existing = original.is_none().then(|| {
            let bytes = fs::read(self.path(&self.session, "json")).ok()?;
            serde_json::from_slice::<RecoveryInfo>(&bytes).ok()?.draft_history
        }).flatten();
        self.write_encoded(&bytes, &doc.meta.name, original, crash_log, existing)
    }

    pub fn write_encoded(&self, bytes: &[u8], name: &str, original: Option<PathBuf>, crash_log: Option<PathBuf>, draft_history: Option<String>) -> Result<(),String> {
        write_atomic(&self.path(&self.session, "atlas"), bytes).map_err(|e| e.to_string())?;
        let info = RecoveryInfo { session: self.session.clone(), original, name: name.into(), saved_at: now_secs(), crash_log, draft_history };
        let json = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
        write_atomic(&self.path(&self.session, "json"), &json).map_err(|e| e.to_string())
    }

    /// Forget this session's snapshot (after a successful save or a clean exit).
    pub fn clear(&self) {
        for ext in ["atlas", "json"] {
            let _ = fs::remove_file(self.path(&self.session, ext));
        }
    }

    /// Clean shutdown: remove snapshot and lock file.
    pub fn finish(self) {
        self.clear();
        let lock = self.path(&self.session, "lock");
        drop(self._lock);
        let _ = fs::remove_file(lock);
    }

    /// Snapshots left behind by sessions that are no longer running.
    pub fn orphans(&self) -> Vec<RecoveryInfo> {
        let Ok(rd) = fs::read_dir(&self.dir) else { return vec![] };
        let mut out = Vec::new();
        for entry in rd.flatten() {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(session) = p.file_stem().and_then(|s| s.to_str()).map(str::to_string) else { continue };
            if session == self.session || !self.path(&session, "atlas").exists() {
                continue;
            }
            // still running elsewhere?
            if let Ok(f) = File::options().read(true).write(true).open(self.path(&session, "lock")) {
                if f.try_lock().is_err() {
                    continue;
                }
            }
            if let Ok(info) = fs::read(&p).map_err(|e| e.to_string()).and_then(|b| serde_json::from_slice::<RecoveryInfo>(&b).map_err(|e| e.to_string())) {
                out.push(info);
            }
        }
        out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        out
    }

    pub fn load(&self, session: &str) -> Result<Document, String> {
        project::load(&self.path(session, "atlas"))
    }

    /// Delete an orphaned session's files.
    pub fn discard(&self, session: &str) {
        for ext in ["atlas", "json", "lock"] {
            let _ = fs::remove_file(self.path(session, ext));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_writers_use_independent_temporary_files() {
        let dir = std::env::temp_dir().join(format!("atlas_atomic_{}_{}", std::process::id(), now_secs()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("world.atlas");
        let old_temporary = dir.join("world.atlas.tmp");
        fs::write(&old_temporary, b"earlier save").unwrap();
        let contents = [vec![0xAA; 256 * 1024], vec![0x55; 256 * 1024]];
        let barrier = std::sync::Barrier::new(contents.len());
        std::thread::scope(|scope| {
            for data in &contents {
                scope.spawn(|| {
                    barrier.wait();
                    write_atomic(&path, data).unwrap();
                });
            }
        });
        assert!(contents.contains(&fs::read(&path).unwrap()));
        assert_eq!(fs::read(&old_temporary).unwrap(), b"earlier save");
        let occupied = dir.join("directory.atlas");
        fs::create_dir(&occupied).unwrap();
        assert!(write_atomic(&occupied, b"cannot replace a directory").is_err());
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 3);
        fs::remove_file(&path).unwrap();
        fs::remove_file(&old_temporary).unwrap();
        fs::remove_dir(&occupied).unwrap();
        fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn crashed_session_is_offered_live_one_is_not() {
        let dir = std::env::temp_dir().join(format!("atlas_rec_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let doc = map_core::sample::demo_world();

        let a = Recovery::start(&dir).unwrap();
        a.write(&doc, Some(PathBuf::from("C:/maps/world.atlas"))).unwrap();
        let b = Recovery::start(&dir).unwrap();
        assert!(b.orphans().is_empty(), "a live session must not be offered for recovery");

        // simulate a crash: the lock is released but the files stay
        let session_a = a.session().to_string();
        drop(a);
        let found = b.orphans();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].session, session_a);
        assert_eq!(found[0].original.as_deref(), Some(Path::new("C:/maps/world.atlas")));
        assert_eq!(crate::project::without_revs(&b.load(&session_a).unwrap()), crate::project::without_revs(&doc));

        b.discard(&session_a);
        assert!(b.orphans().is_empty());
        // a clean finish leaves nothing behind
        b.write(&doc, None).unwrap();
        b.finish();
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 0);
        let _ = fs::remove_dir_all(&dir);
    }

    /// A snapshot written as the program crashed names its log; snapshots
    /// written before that existed read as ordinary ones. Lock files left
    /// by sessions that ended with nothing to recover are swept away.
    #[test]
    fn crash_snapshots_and_stale_locks() {
        let dir = std::env::temp_dir().join(format!("atlas_rec_crash_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let doc = map_core::sample::demo_world();
        let a = Recovery::start(&dir).unwrap();
        let log = write_crash_log(&dir.join("crash"), "panicked at somewhere").unwrap();
        assert!(fs::read_to_string(&log).unwrap().contains("somewhere"));
        a.write_with(&doc, None, Some(log.clone())).unwrap();
        let session_a = a.session().to_string();
        // an older snapshot's metadata has no `crash_log`
        let old = RecoveryInfo { session: "old".into(), original: None, name: "旧".into(), saved_at: 1, crash_log: None, draft_history: None };
        let mut v = serde_json::to_value(&old).unwrap();
        v.as_object_mut().unwrap().remove("crash_log");
        assert_eq!(serde_json::from_value::<RecoveryInfo>(v).unwrap(), old);
        // a session that ended without a snapshot leaves only its lock
        let c = Recovery::start(&dir).unwrap();
        let lock_c = dir.join(format!("{}.lock", c.session()));
        drop(c);
        assert!(lock_c.exists());
        drop(a);
        let b = Recovery::start(&dir).unwrap();
        assert!(!lock_c.exists(), "the stale lock is swept");
        let found = b.orphans();
        assert_eq!(found.len(), 1, "the crashed session is still offered");
        assert_eq!((found[0].session.as_str(), found[0].crash_log.as_ref()), (session_a.as_str(), Some(&log)));
        let _ = fs::remove_dir_all(&dir);
    }
}
