//! Periodic background autosave and the crash-recovery prompt.

use crate::app::App;
use eframe::egui;
use map_core::doc::Document;
use map_io::recovery::{Recovery, RecoveryInfo};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub struct Autosave {
    recovery: Option<Arc<Recovery>>,
    job: Option<JoinHandle<Result<(), String>>>,
    last: Instant,
    /// History generation captured by the last snapshot.
    saved_gen: Option<u64>,
    interval: Duration,
    /// The interval was given by ATLAS_AUTOSAVE_SECS (tests): preferences leave it alone.
    fixed: bool,
    /// Snapshots from crashed sessions, offered at startup.
    pub offers: Vec<RecoveryInfo>,
}

impl Autosave {
    pub fn start() -> Self {
        let recovery = Recovery::start(&map_io::recovery::default_dir()).ok().map(Arc::new);
        crate::crash::install(recovery.clone());
        let offers = recovery.as_ref().map(|r| r.orphans()).unwrap_or_default();
        let env: Option<u64> = std::env::var("ATLAS_AUTOSAVE_SECS").ok().and_then(|s| s.parse().ok());
        Self { recovery, job: None, last: Instant::now(), saved_gen: None, interval: Duration::from_secs(env.unwrap_or(60)), fixed: env.is_some(), offers }
    }

    /// Seconds between snapshots, from the preferences.
    pub fn set_interval(&mut self, secs: u64) {
        if !self.fixed {
            self.interval = Duration::from_secs(secs.max(1));
        }
    }

    fn wait_job(&mut self) -> Option<Result<(), String>> {
        self.job.take().map(|j| j.join().unwrap_or_else(|_| Err("自动保存线程崩溃".into())))
    }

    /// The document is safely on disk (saved, opened or newly created):
    /// the recovery snapshot is obsolete.
    pub fn mark_clean(&mut self, gen: u64) {
        let _ = self.wait_job();
        if let Some(r) = &self.recovery {
            r.clear();
        }
        self.saved_gen = Some(gen);
        self.last = Instant::now();
    }

    /// Force a snapshot at the next opportunity (e.g. recovered work that
    /// exists only in memory).
    pub fn mark_unsaved(&mut self) {
        self.saved_gen = None;
        self.last = Instant::now().checked_sub(self.interval).unwrap_or_else(Instant::now);
    }

    /// Start a background snapshot if it is due. Returns an error message
    /// from a previously finished snapshot, if any.
    pub fn tick(&mut self, ctx: &egui::Context, doc: &Document, original: Option<PathBuf>, gen: u64, dirty: bool, busy: bool) -> Option<String> {
        let mut err = None;
        if self.job.as_ref().is_some_and(|j| j.is_finished()) {
            err = self.wait_job().and_then(|r| r.err());
        }
        let Some(rec) = self.recovery.clone() else { return err };
        if self.job.is_some() {
            // come back for its result (the window rests while idle)
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        if !dirty || busy || self.job.is_some() || self.saved_gen == Some(gen) {
            return err;
        }
        let elapsed = self.last.elapsed();
        if elapsed < self.interval {
            ctx.request_repaint_after(self.interval - elapsed);
            return err;
        }
        // O(1) clone; serialisation and disk I/O happen off the UI thread
        let doc = doc.clone();
        self.job = Some(std::thread::spawn(move || rec.write(&doc, original)));
        self.saved_gen = Some(gen);
        self.last = Instant::now();
        err
    }

    pub fn load_offer(&self, session: &str) -> Result<Document, String> {
        self.recovery.as_ref().ok_or("恢复目录不可用")?.load(session)
    }

    pub fn discard_offer(&mut self, session: &str) {
        if let Some(r) = &self.recovery {
            r.discard(session);
        }
        self.offers.retain(|o| o.session != session);
    }

    /// Clean shutdown: no snapshot should survive.
    pub fn finish(&mut self) {
        let _ = self.wait_job();
        if let Some(r) = self.recovery.take() {
            // the crash hook holds the session too: the lock file goes with the process
            match Arc::try_unwrap(r) {
                Ok(r) => r.finish(),
                Err(r) => r.clear(),
            }
        }
    }
}

fn ago(secs: u64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(secs);
    let d = now.saturating_sub(secs);
    match d {
        0..=59 => format!("{d} 秒前"),
        60..=3599 => format!("{} 分钟前", d / 60),
        3600..=86399 => format!("{} 小时前", d / 3600),
        _ => format!("{} 天前", d / 86400),
    }
}

impl App {
    pub fn recovery_window(&mut self, ctx: &egui::Context) {
        if self.autosave.offers.is_empty() {
            return;
        }
        let mut action: Option<(String, bool)> = None;
        egui::Window::new("恢复未保存的工作").collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
            ui.label("上次程序没有正常退出，发现以下自动保存的内容：");
            ui.add_space(4.0);
            for o in &self.autosave.offers {
                ui.horizontal(|ui| {
                    let file = o.original.as_ref().map(|p| p.display().to_string()).unwrap_or("（从未保存）".into());
                    ui.label(format!("{} — {} — {}", o.name, file, ago(o.saved_at)));
                    if let Some(log) = &o.crash_log {
                        ui.weak("崩溃时保存").on_hover_text(format!("程序崩溃时保存的内容。崩溃日志：{}", log.display()));
                    }
                    if ui.button("恢复").clicked() {
                        action = Some((o.session.clone(), true));
                    }
                    if ui.button("丢弃").clicked() {
                        action = Some((o.session.clone(), false));
                    }
                });
            }
        });
        let Some((session, restore)) = action else { return };
        if restore {
            match self.autosave.load_offer(&session) {
                Ok(doc) => {
                    let original = self.autosave.offers.iter().find(|o| o.session == session).and_then(|o| o.original.clone());
                    self.new_doc(doc);
                    self.path = original;
                    // recovered work is not on disk until the user saves it
                    self.dirty = true;
                    self.autosave.discard_offer(&session);
                    self.autosave.mark_unsaved();
                    self.set_status("已恢复自动保存的内容，请尽快保存。", false);
                }
                Err(e) => self.set_status(format!("恢复失败：{e}"), true),
            }
        } else {
            self.autosave.discard_offer(&session);
        }
    }
}
