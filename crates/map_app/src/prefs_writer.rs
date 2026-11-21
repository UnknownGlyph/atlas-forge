//! Persist preference snapshots in order and keep the latest one while a
//! previous disk write finishes. All editor-side changes apply immediately.
use map_io::prefs::{self, Prefs};
use std::{path::Path, thread::JoinHandle};

#[derive(Default)]
pub(crate) struct PrefsWriter {
    job: Option<JoinHandle<Result<(), String>>>,
    pending: Option<Prefs>,
}

impl PrefsWriter {
    pub fn writing(&self) -> bool {
        self.job.is_some() || self.pending.is_some()
    }

    pub fn queue(&mut self, prefs: &Prefs) {
        self.pending = Some(prefs.clone());
    }

    pub fn tick(&mut self, path: &Path, ctx: &eframe::egui::Context) -> Result<(), String> {
        let result = if self.job.as_ref().is_some_and(|job| job.is_finished()) {
            self.join()
        } else {
            Ok(())
        };
        if self.job.is_none() {
            if let Some(next) = self.pending.take() {
                let path = path.to_owned();
                let wake = ctx.clone();
                self.job = Some(crate::perf_ui::spawn("写入界面偏好", move || {
                    let result = prefs::save(&path, &next);
                    wake.request_repaint();
                    result
                }));
            }
        }
        if self.job.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(150));
        }
        result
    }

    pub fn finish(&mut self, path: &Path) -> Result<(), String> {
        let result = self.join();
        if let Some(next) = self.pending.take() {
            return prefs::save(path, &next);
        }
        result
    }

    fn join(&mut self) -> Result<(), String> {
        self.job.take().map_or(Ok(()), |job| {
            job.join()
                .map_err(|_| "Preference writer stopped unexpectedly".to_owned())?
        })
    }
}
