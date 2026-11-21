//! Periodic background autosave and the crash-recovery prompt.

use crate::app::App;
use eframe::egui;
use map_core::doc::Document;
use map_io::checkpoints::Store;
use map_io::cloud::{self, Provider};
use map_io::recovery::{Recovery, RecoveryInfo};
use std::path::PathBuf;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

struct Snapshot {
    archived: bool,
    error: Option<String>,
    cloud: [Option<Result<PathBuf,String>>;2],
}

fn cloud_copies(config:&cloud::Config,name:&str,bytes:&[u8])->[Option<Result<PathBuf,String>>;2] {
    Provider::ALL.map(|p| config.folder(p).map(|path|cloud::write(path,name,bytes)))
}

pub struct Autosave {
    recovery: Option<Arc<Recovery>>,
    recovery_directory: PathBuf,
    job: Option<(u64, JoinHandle<Snapshot>)>,
    last: Instant,
    /// History generation captured by the last snapshot.
    saved_gen: Option<u64>,
    attempted_gen: Option<u64>,
    interval: Duration,
    /// The interval was given by ATLAS_AUTOSAVE_SECS (tests): preferences leave it alone.
    fixed: bool,
    enabled: bool,
    cloud: cloud::Config,
    cloud_gen: [Option<u64>;2],
    cloud_saved_file: Option<(u64,PathBuf)>,
    pub cloud_result: [Option<Result<PathBuf,String>>;2],
    steps: u64,
    pub keep: usize,
    pub store: Store,
    /// Changes when the list needs to be read again.
    pub revision: u64,
    /// Snapshots from crashed sessions, offered at startup.
    pub offers: Vec<RecoveryInfo>,
}

impl Autosave {
    pub fn start(prefs: &map_io::prefs::Prefs) -> Self {
        let recovery_directory = prefs.recovery_directory();
        let recovery = Recovery::start(&recovery_directory)
            .ok()
            .map(Arc::new);
        crate::crash::install(recovery.clone(), prefs.crash_directory());
        let offers = recovery.as_ref().map(|r| r.orphans()).unwrap_or_default();
        let env: Option<u64> = std::env::var("ATLAS_AUTOSAVE_SECS")
            .ok()
            .and_then(|s| s.parse().ok());
        Self {
            store: Store::draft_in(&recovery_directory),
            recovery,
            recovery_directory,
            job: None,
            last: Instant::now(),
            saved_gen: None,
            attempted_gen: None,
            interval: Duration::from_secs(env.unwrap_or(60)),
            fixed: env.is_some(),
            enabled: true,
            cloud: Default::default(),
            cloud_gen: [None;2],
            cloud_saved_file: None,
            cloud_result: [None,None],
            steps: 10,
            keep: 20,
            revision: 0,
            offers,
        }
    }

    /// Seconds between snapshots, from the preferences.
    pub fn set_interval(&mut self, secs: u64) {
        if !self.fixed {
            self.interval = Duration::from_secs(secs.max(1));
        }
    }

    pub fn configure(&mut self, prefs: &map_io::prefs::Prefs) -> Option<String> {
        self.set_interval(prefs.autosave_secs);
        self.enabled = prefs.autosave_on;
        self.steps = prefs.autosave_steps;
        if self.cloud != prefs.cloud_backup {
            let _ = self.wait_job();
            for (i,p) in Provider::ALL.into_iter().enumerate() {
                if self.cloud.folder(p) != prefs.cloud_backup.folder(p) {
                    self.cloud_gen[i]=None;
                    self.cloud_result[i]=None;
                }
            }
            self.cloud=prefs.cloud_backup.clone();
        }
        let previous = std::mem::replace(&mut self.keep, prefs.autosave_keep);
        if previous == self.keep {
            return None;
        }
        let _ = self.wait_job();
        self.revision += 1;
        self.store.trim(self.keep).err()
    }

    fn wait_job(&mut self) -> Option<String> {
        self.job.take().and_then(|(generation, j)| {
            let result = j.join().unwrap_or_else(|_| Snapshot {
                archived: false,
                error: Some("自动保存线程崩溃".into()),
                cloud: [None,None],
            });
            if result.archived {
                self.saved_gen = Some(generation);
            }
            self.revision += 1;
            let mut errors:Vec<_>=result.error.into_iter().collect();
            for (i,r) in result.cloud.into_iter().enumerate() {
                if let Some(r)=r{
                    match &r {
                        Ok(_)=>self.cloud_gen[i]=Some(generation),
                        Err(e)=>errors.push(format!("{}：{e}",Provider::ALL[i].label())),
                    }
                    self.cloud_result[i]=Some(r);
                }
            }
            (!errors.is_empty()).then(||errors.join(" · "))
        })
    }

    /// The document is safely on disk (saved, opened or newly created):
    /// the recovery snapshot is obsolete.
    pub fn mark_clean(&mut self, gen: u64) {
        let _ = self.wait_job();
        if let Some(r) = &self.recovery {
            r.clear();
        }
        self.saved_gen = Some(gen);
        self.cloud_gen = [None;2];
        self.cloud_saved_file=None;
        self.attempted_gen = None;
        self.last = Instant::now();
    }

    pub fn open_project(&mut self, path: Option<&std::path::Path>, generation: u64) {
        self.mark_clean(generation);
        self.store = path.map_or_else(|| Store::draft_in(&self.recovery_directory), Store::for_project);
        self.cloud_result = [None,None];
        self.revision += 1;
    }

    pub fn saved_project(&mut self, path: &std::path::Path, generation: u64) -> Result<(), String> {
        self.mark_clean(generation);
        if !self.cloud.is_empty() {
            self.cloud_saved_file=Some((generation,path.to_path_buf()));
            let cloud=self.cloud.clone();
            let source=path.to_path_buf();
            self.job=Some((generation,crate::perf_ui::spawn("迁移恢复快照", move||match std::fs::read(&source) {
                Ok(bytes)=>Snapshot {archived:false,error:None,cloud:cloud_copies(
                    &cloud,source.file_stem().and_then(|s|s.to_str()).unwrap_or("World"),&bytes)},
                Err(e)=>Snapshot {archived:false,error:Some(e.to_string()),cloud:[None,None]},
            })));
        }
        let destination = Store::for_project(path);
        self.store.copy_draft_to(&destination, self.keep)?;
        self.store = destination;
        self.revision += 1;
        Ok(())
    }

    fn reconnect_recovered(&mut self, info: &RecoveryInfo, doc: &Document) -> Result<(), String> {
        self.store = info
            .original
            .as_deref()
            .map(Store::for_project)
            .or_else(|| info.draft_history.as_deref().and_then(|key| Store::from_draft_in(&self.recovery_directory, key)))
            .unwrap_or_else(|| Store::draft_in(&self.recovery_directory));
        self.revision += 1;
        self.mark_unsaved();
        let rec = self.recovery.as_ref().ok_or("恢复目录不可用")?;
        let bytes = map_io::project::to_bytes(doc)?;
        rec.write_encoded(
            &bytes,
            &doc.meta.name,
            info.original.clone(),
            None,
            self.store.draft_key().map(str::to_owned),
        )
    }

    /// Force a snapshot at the next opportunity (e.g. recovered work that
    /// exists only in memory).
    pub fn mark_unsaved(&mut self) {
        self.saved_gen = None;
        self.attempted_gen = None;
        self.last = Instant::now()
            .checked_sub(self.interval)
            .unwrap_or_else(Instant::now);
    }

    /// Start a background snapshot if it is due. Returns an error message
    /// from a previously finished snapshot, if any.
    pub fn tick(
        &mut self,
        ctx: &egui::Context,
        doc: &Document,
        original: Option<PathBuf>,
        gen: u64,
        dirty: bool,
        busy: bool,
    ) -> Option<String> {
        let mut err = None;
        if self.job.as_ref().is_some_and(|(_, j)| j.is_finished()) {
            err = self.wait_job();
        }
        if self.job.is_some() {
            // come back for its result (the window rests while idle)
            ctx.request_repaint_after(Duration::from_millis(500));
        }
        let capture_local = self.enabled && self.saved_gen != Some(gen);
        let mut cloud=self.cloud.clone();
        for (i,p) in Provider::ALL.into_iter().enumerate(){
            if self.cloud_gen[i]==Some(gen){*cloud.folder_mut(p)=None;}
        }
        let manual = self.cloud_saved_file.as_ref().filter(|(year,_)|!dirty&&*year==gen).map(|(_,p)|p.clone());
        if (!dirty && manual.is_none()) || busy || self.job.is_some() || (!capture_local && cloud.is_empty()) {
            return err;
        }
        let elapsed = self.last.elapsed();
        let step_due = self.steps > 0
            && self
                .saved_gen
                .is_some_and(|prior| gen.saturating_sub(prior) >= self.steps)
            && self.attempted_gen != Some(gen);
        if elapsed < self.interval && !step_due {
            ctx.request_repaint_after(self.interval - elapsed);
            return err;
        }
        // O(1) clone; serialisation and disk I/O happen off the UI thread
        let doc = doc.clone();
        let rec = self.recovery.clone().filter(|_|capture_local);
        let store = self.store.clone();
        let keep = self.keep;
        self.job = Some((
            gen,
            crate::perf_ui::spawn("写入自动保存", move || {
                let label = original.as_deref()
                    .and_then(|path| path.file_stem().and_then(|name| name.to_str()))
                    .unwrap_or(&doc.meta.name)
                    .to_owned();
                if let Some(path)=manual {
                    return match std::fs::read(&path) {
                        Ok(bytes)=>Snapshot {archived:false,error:None,cloud:cloud_copies(&cloud,
                            path.file_stem().and_then(|s|s.to_str()).unwrap_or(&doc.meta.name), &bytes)},
                        Err(e)=>Snapshot {archived:false,error:Some(e.to_string()),cloud:[None,None]},
                    };
                }
                let bytes = match map_io::project::to_bytes(&doc) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        return Snapshot {
                            archived: false,
                            error: Some(error),
                            cloud: [None,None],
                        }
                    }
                };
                let recovery = rec.and_then(|rec| {
                    rec.write_encoded(
                        &bytes,
                        &doc.meta.name,
                        original,
                        None,
                        store.draft_key().map(str::to_owned),
                    )
                    .err()
                    .map(|error| format!("崩溃恢复：{error}"))
                });
                let archive = capture_local.then(||store.capture_encoded(&doc.meta.name,&bytes,keep));
                let archived = archive.as_ref().is_some_and(|r|r.is_ok());
                let errors: Vec<_> = [
                    recovery,
                    archive.and_then(Result::err).map(|error| format!("版本历史：{error}")),
                ]
                .into_iter()
                .flatten()
                .collect();
                Snapshot {
                    archived,
                    error: (!errors.is_empty()).then(|| errors.join("；")),
                    cloud: cloud_copies(&cloud, &label, &bytes),
                }
            }),
        ));
        self.attempted_gen = Some(gen);
        self.last = Instant::now();
        ctx.request_repaint_after(Duration::from_millis(200));
        err
    }

    pub fn load_offer(&self, session: &str) -> Result<Document, String> {
        self.recovery
            .as_ref()
            .ok_or("恢复目录不可用")?
            .load(session)
    }

    pub fn discard_offer(&mut self, session: &str) {
        if let Some(r) = &self.recovery {
            r.discard(session);
        }
        self.offers.retain(|o| o.session != session);
    }

    /// Finish any atomic writes and clear the clean session once the window
    /// has closed. The session lock lasts until these writes have stopped.
    pub fn defer_finish(&mut self) {
        let job = self.job.take();
        let recovery = self.recovery.take();
        crate::shutdown::defer(move || {
            if let Some((_, worker)) = job {
                let _ = worker.join();
            }
            crate::crash::release_recovery();
            if let Some(recovery) = recovery {
                match Arc::try_unwrap(recovery) {
                    Ok(recovery) => recovery.finish(),
                    Err(recovery) => recovery.clear(),
                }
            }
        });
    }
}

pub(crate) fn ago(secs: u64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(secs);
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
        egui::Window::new(crate::i18n::tr("恢复未保存的工作"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(crate::i18n::tr("上次程序没有正常退出，发现以下自动保存的内容："));
                ui.add_space(4.0);
                for o in &self.autosave.offers {
                    ui.horizontal(|ui| {
                        let file = o
                            .original
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or("（从未保存）".into());
                        ui.label(format!("{} — {} — {}", o.name, file, ago(o.saved_at)));
                        if let Some(log) = &o.crash_log {
                            ui.weak(crate::i18n::tr("崩溃时保存")).on_hover_text(crate::trf!(
                                "程序崩溃时保存的内容。崩溃日志：{}",
                                log.display()
                            ));
                        }
                        if ui.button(crate::i18n::tr("恢复")).clicked() {
                            action = Some((o.session.clone(), true));
                        }
                        if ui.button(crate::i18n::tr("丢弃")).clicked() {
                            action = Some((o.session.clone(), false));
                        }
                    });
                }
            });
        let Some((session, restore)) = action else {
            return;
        };
        if restore {
            match self.autosave.load_offer(&session) {
                Ok(doc) => {
                    let Some(offer) = self
                        .autosave
                        .offers
                        .iter()
                        .find(|o| o.session == session)
                        .cloned()
                    else {
                        return;
                    };
                    self.new_doc(doc);
                    self.path = offer.original.clone();
                    // recovered work is not on disk until the user saves it
                    self.dirty = true;
                    match self.autosave.reconnect_recovered(&offer, &self.doc) {
                        Ok(()) => {
                            self.autosave.discard_offer(&session);
                            self.set_status(crate::i18n::tr("已恢复自动保存的内容，请尽快保存。"), false);
                        }
                        Err(e) => self.set_status(
                            crate::trf!("内容已读入，原恢复项仍可使用。建立新恢复副本失败：{e}"),
                            true,
                        ),
                    }
                }
                Err(e) => self.set_status(crate::trf!("恢复失败：{e}"), true),
            }
        } else {
            self.autosave.discard_offer(&session);
        }
    }
}
