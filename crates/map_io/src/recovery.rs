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

/// Write `bytes` to `path` durably: temp file, flush to disk, then rename.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", path.extension().and_then(|e| e.to_str()).unwrap_or("")));
    {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    fs::rename(&tmp, path)
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
        write_atomic(&self.path(&self.session, "atlas"), &bytes).map_err(|e| e.to_string())?;
        let info = RecoveryInfo { session: self.session.clone(), original, name: doc.meta.name.clone(), saved_at: now_secs(), crash_log };
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
        let old = RecoveryInfo { session: "old".into(), original: None, name: "旧".into(), saved_at: 1, crash_log: None };
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
