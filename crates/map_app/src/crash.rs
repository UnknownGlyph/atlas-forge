//! What happens when the program panics: a log is written, and if the
//! panic is on the interface thread (the program is about to exit) unsaved
//! work is saved as a recovery snapshot, offered at the next start.
//!
//! The hook cannot reach the application's state, so the interface keeps
//! a copy of the document here (an O(1) clone) whenever it changes.

use map_core::doc::Document;
use map_io::recovery::Recovery;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The work to save if the program crashes now.
struct Unsaved {
    doc: Document,
    original: Option<PathBuf>,
    /// History generation the copy was taken at.
    generation: u64,
}

static UNSAVED: Mutex<Option<Unsaved>> = Mutex::new(None);
static RECOVERY: Mutex<Option<Arc<Recovery>>> = Mutex::new(None);

pub(crate) fn release_recovery() {
    if let Ok(mut recovery) = RECOVERY.lock() {
        recovery.take();
    }
    if let Ok(mut unsaved) = UNSAVED.lock() {
        unsaved.take();
    }
}

/// Keep the document for an emergency save (`dirty`: it has unsaved
/// changes; otherwise there is nothing to save). Cheap when nothing changed.
pub fn remember(doc: &Document, original: &Option<PathBuf>, generation: u64, dirty: bool) {
    let Ok(mut u) = UNSAVED.lock() else { return };
    if !dirty {
        *u = None;
    } else if u.as_ref().is_none_or(|x| x.generation != generation || x.original != *original) {
        *u = Some(Unsaved { doc: doc.clone(), original: original.clone(), generation });
    }
}

/// The log's text for a panic.
fn report(info: &std::panic::PanicHookInfo, thread: &str) -> String {
    let msg = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
    let at = info.location().map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column())).unwrap_or_else(|| "(unknown location)".into());
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!(
        "Atlas Forge {} crashed\nunix time: {secs}\nos: {} {}\nthread: {thread}\nmessage: {msg}\nat: {at}\n\nbacktrace:\n{}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::backtrace::Backtrace::force_capture()
    )
}

/// Install the panic hook. `recovery`: where an emergency snapshot goes.
pub fn install(recovery: Option<Arc<Recovery>>, crash_directory: std::path::PathBuf) {
    if let Ok(mut current) = RECOVERY.lock() {
        *current = recovery;
    }
    let interface = std::thread::current().id();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let here = std::thread::current();
        let on_interface = here.id() == interface;
        let text = report(info, here.name().unwrap_or("(unnamed)"));
        let log = map_io::recovery::write_crash_log(&crash_directory, &text).ok();
        if let Some(p) = &log {
            eprintln!("crash log: {}", p.display());
        }
        // a panic on a worker thread is reported by the interface and the
        // program goes on; on the interface thread it is the end
        if on_interface {
            // try_lock: the panic may have happened while the copy was being taken
            let recovery = RECOVERY.try_lock().ok().and_then(|r| r.clone());
            if let (Some(rec), Ok(mut u)) = (recovery, UNSAVED.try_lock()) {
                if let Some(x) = u.take() {
                    match rec.write_with(&x.doc, x.original, log.clone()) {
                        Ok(()) => eprintln!("unsaved work was saved for recovery"),
                        Err(e) => eprintln!("could not save unsaved work: {e}"),
                    }
                }
            }
        }
        previous(info);
    }));
}
