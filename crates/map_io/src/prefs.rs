//! What the program remembers between runs, apart from the worlds
//! themselves: the files opened lately and a few settings of the
//! interface. One small JSON file; a missing or damaged one means the
//! defaults, and fields a newer version added are left alone by an older
//! one (unknown fields are ignored, missing ones take their defaults).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// How many recent files are kept.
pub const RECENT_MAX: usize = 10;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// Files opened or saved lately, the latest first.
    pub recent: Vec<PathBuf>,
    /// Size of the interface (1 = the system's own scale).
    pub ui_scale: f32,
    /// Seconds between recovery snapshots of unsaved work.
    pub autosave_secs: u64,
    pub autosave_on: bool,
    pub autosave_keep: usize,
    /// Additional trigger, in committed history operations. Zero is time only.
    pub autosave_steps: u64,
    pub cloud_backup: crate::cloud::Config,
    pub save_directory: Option<PathBuf>,
    pub cache_directory: Option<PathBuf>,
    pub test_project: Option<PathBuf>,
    /// Show the start window when the program opens without a file.
    pub welcome: bool,
    /// Open the latest file at start instead of the sample world.
    pub reopen_last: bool,
    /// "flat", "globe" or "split".
    pub layout: String,
    pub graticule: bool,
    pub progressive_detail: bool,
    /// A scale bar and a north arrow on the map views.
    pub furniture: bool,
    /// Frame times, triangle counts and memory in the status bar.
    pub perf_in_status: bool,
    /// Undo steps kept.
    pub undo_limit: usize,
    /// Locale code of the application's interface, for example "en" or "zh-TW".
    pub language: String,
}

impl Default for Prefs {
    fn default() -> Self {
        Self { recent: vec![], ui_scale: 1.0, autosave_secs: 60, autosave_on: true, autosave_keep: 20, autosave_steps: 10, cloud_backup: Default::default(), save_directory: None, cache_directory: None, test_project: None, welcome: true, reopen_last: false, layout: "split".into(), graticule: true, progressive_detail: true, furniture: true, perf_in_status: false, undo_limit: 500, language: "zh".into() }
    }
}

impl Prefs {
    pub fn recovery_directory(&self) -> PathBuf {
        self.cache_directory.as_ref().map_or_else(crate::recovery::default_dir, |base| base.join("recovery"))
    }

    pub fn crash_directory(&self) -> PathBuf {
        self.cache_directory.as_ref().map_or_else(crate::recovery::crash_dir, |base| base.join("crash"))
    }

    /// Values a hand-edited file may have pushed out of range.
    fn tidy(mut self) -> Self {
        self.ui_scale = if self.ui_scale.is_finite() { self.ui_scale.clamp(0.5, 3.0) } else { 1.0 };
        self.autosave_secs = self.autosave_secs.clamp(5, 3600);
        self.autosave_keep = self.autosave_keep.clamp(1, 1000);
        self.autosave_steps = self.autosave_steps.min(1000);
        self.undo_limit = self.undo_limit.clamp(20, 5000);
        self.recent.truncate(RECENT_MAX);
        if !matches!(self.layout.as_str(), "flat" | "globe" | "split") {
            self.layout = "split".into();
        }
        self
    }

    /// Put a file at the head of the recent list.
    pub fn note_recent(&mut self, p: &Path) {
        self.recent.retain(|q| q != p);
        self.recent.insert(0, p.to_path_buf());
        self.recent.truncate(RECENT_MAX);
    }

    pub fn forget_recent(&mut self, p: &Path) {
        self.recent.retain(|q| q != p);
    }
}

/// Where the preferences live: `ATLAS_PREFS` if set (tests, portable
/// installs), else `%APPDATA%\AtlasForge\prefs.json` or the platform's
/// configuration folder.
pub fn default_path() -> PathBuf {
    if let Some(p) = std::env::var_os("ATLAS_PREFS") {
        return PathBuf::from(p);
    }
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from))
        .or_else(|| {
            std::env::var_os("HOME").map(|h| {
                let h = PathBuf::from(h);
                if cfg!(target_os = "macos") {
                    h.join("Library").join("Application Support")
                } else {
                    h.join(".config")
                }
            })
        })
        .unwrap_or_else(std::env::temp_dir);
    base.join("AtlasForge").join("prefs.json")
}

/// The stored preferences; the defaults when there are none or the file
/// cannot be read as preferences.
pub fn load(path: &Path) -> Prefs {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str::<Prefs>(&t).ok()).unwrap_or_default().tidy()
}

/// Write the preferences (whole file replaced at once).
pub fn save(path: &Path, prefs: &Prefs) -> Result<(), String> {
    use std::{fs, io::Write, sync::atomic::{AtomicU64, Ordering}};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

    let parent = path.parent().filter(|dir| !dir.as_os_str().is_empty()).unwrap_or(Path::new("."));
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let text = serde_json::to_vec_pretty(prefs).map_err(|e| e.to_string())?;

    // Windows can have more than one Atlas window/process writing preferences.
    // A separate temporary file keeps each replacement atomic.
    let (temporary, mut file) = loop {
        let number = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(".atlas-prefs-{}-{number}.tmp", std::process::id()));
        match fs::File::options().create_new(true).write(true).open(&temporary) {
            Ok(file) => break (temporary, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.to_string()),
        }
    };
    let write = file.write_all(&text).and_then(|()| file.sync_all());
    drop(file);
    let result = write.and_then(|()| fs::rename(&temporary, path));
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("atlas_prefs_{}_{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("sub").join("prefs.json")
    }

    #[test]
    fn stored_and_read_back() {
        let path = temp("round");
        assert_eq!(load(&path), Prefs::default(), "no file: the defaults");
        let mut p = Prefs { ui_scale: 1.25, autosave_secs: 120, welcome: false, layout: "flat".into(), ..Default::default() };
        p.note_recent(Path::new("a.atlas"));
        p.note_recent(Path::new("世界 二.atlas"));
        save(&path, &p).unwrap();
        assert_eq!(load(&path), p);
        // saved again over the old file
        p.perf_in_status = true;
        save(&path, &p).unwrap();
        assert_eq!(load(&path), p);
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }

    #[test]
    fn recent_files_latest_first_without_repeats() {
        let mut p = Prefs::default();
        for i in 0..14 {
            p.note_recent(Path::new(&format!("w{i}.atlas")));
        }
        assert_eq!(p.recent.len(), RECENT_MAX);
        assert_eq!(p.recent[0], Path::new("w13.atlas"));
        p.note_recent(Path::new("w8.atlas"));
        assert_eq!((p.recent.len(), p.recent[0].as_path(), p.recent[1].as_path()), (RECENT_MAX, Path::new("w8.atlas"), Path::new("w13.atlas")));
        assert_eq!(p.recent.iter().filter(|q| q.as_path() == Path::new("w8.atlas")).count(), 1);
        p.forget_recent(Path::new("w8.atlas"));
        assert_eq!((p.recent.len(), p.recent[0].as_path()), (RECENT_MAX - 1, Path::new("w13.atlas")));
    }

    #[test]
    fn damaged_or_partial_files() {
        let path = temp("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Prefs::default());
        // an older file with few fields, a newer one with unknown fields,
        // and values out of range
        std::fs::write(&path, r#"{"recent":["x.atlas"],"some_future_field":3,"ui_scale":99,"autosave_secs":0,"layout":"cube"}"#).unwrap();
        let p = load(&path);
        assert_eq!(p.recent, [PathBuf::from("x.atlas")]);
        assert_eq!((p.ui_scale, p.autosave_secs, p.layout.as_str(), p.welcome), (3.0, 5, "split", true));
        let _ = std::fs::remove_dir_all(path.parent().unwrap().parent().unwrap());
    }
}
