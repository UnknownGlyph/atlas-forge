//! The in-app controls share the same trace format as the standalone CLI.
use crate::{app::App, i18n::tr};
use eframe::egui;
use map_perf::{Record, Recording, Span, Trace};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

// The UI owns the current session. Worker tasks capture it when they are
// dispatched, so a late completion cannot appear in a newer recording.
thread_local! {
    static ACTIVE: RefCell<Option<(Trace, Option<u64>)>> = const { RefCell::new(None) };
}

pub(crate) fn context() -> Option<(Trace, Option<u64>)> {
    ACTIVE.with_borrow(Clone::clone)
}

pub(crate) fn span(channel: &'static str, name: impl Into<String>) -> Option<Span> {
    ACTIVE.with_borrow(|active| {
        active
            .as_ref()
            .map(|(trace, frame)| trace.span(channel, name, *frame))
    })
}

pub(crate) fn action(act: crate::actions::Act) -> Option<Span> {
    ACTIVE.with_borrow(|active| {
        active
            .as_ref()
            .map(|(trace, frame)| trace.span("命令", format!("{act:?}"), *frame))
    })
}

pub(crate) fn spawn<T: Send + 'static>(
    name: &'static str,
    work: impl FnOnce() -> T + Send + 'static,
) -> JoinHandle<T> {
    let active = context();
    std::thread::spawn(move || {
        let _span = active.map(|(trace, frame)| trace.span("后台任务", name, frame));
        work()
    })
}

type ReportJob = JoinHandle<Result<PathBuf, String>>;

pub(crate) struct Performance {
    pub open: bool,
    recording: Option<Recording>,
    report: Option<PathBuf>,
    log: Option<PathBuf>,
    job: Option<ReportJob>,
    error: Option<String>,
    adapter: Option<String>,
    next_frame: u64,
    marker_down: bool,
}

impl Performance {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut performance = Self {
            open: false,
            recording: None,
            report: None,
            log: None,
            job: None,
            error: None,
            adapter: cc.wgpu_render_state.as_ref().map(|state| {
                let info = state.adapter.get_info();
                format!("{} / {:?}", info.name, info.backend)
            }),
            next_frame: 0,
            marker_down: false,
        };
        if let Some(path) = std::env::var_os("ATLAS_PERF_LOG") {
            performance.start(Path::new(&path));
            performance.open = performance.error.is_some();
        }
        performance
    }

    fn start(&mut self, path: &Path) {
        self.error = None;
        let report = map_perf::report::default_path(path);
        if report == path || report.exists() {
            self.error = Some(tr("请选择没有同名 HTML 报告的新日志文件。").to_owned());
            return;
        }
        match Recording::start(path, env!("CARGO_PKG_VERSION"), self.adapter.clone()) {
            Ok(recording) => {
                ACTIVE.set(Some((recording.trace(), None)));
                self.recording = Some(recording);
                self.log = Some(path.to_owned());
                self.report = None;
                self.next_frame = 0;
                self.marker_down = false;
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }

    pub fn begin_frame(&mut self) -> Option<u64> {
        self.recording.as_ref()?;
        self.next_frame += 1;
        let number = self.next_frame;
        ACTIVE.with_borrow_mut(|active| {
            if let Some((_, frame)) = active {
                *frame = Some(number);
            }
        });
        Some(number)
    }

    pub fn end_frame(&self, frame: map_perf::Frame) {
        if let Some(recording) = &self.recording {
            recording.trace().emit(Record::Frame(frame));
        }
        ACTIVE.with_borrow_mut(|active| {
            if let Some((_, frame)) = active {
                *frame = None;
            }
        });
    }

    pub fn mark_key(&mut self, input: &mut egui::RawInput) {
        let Some(recording) = &self.recording else {
            return;
        };
        if !input.focused {
            self.marker_down = false;
        }
        let mut held = self.marker_down;
        input.events.retain(|event| {
            if let egui::Event::Key {
                key: egui::Key::F8,
                pressed,
                modifiers,
                ..
            } = event
            {
                if held || modifiers.is_none() {
                    if *pressed && !held {
                        mark_stall(&recording.trace());
                    }
                    held = *pressed;
                    return false;
                }
            }
            true
        });
        self.marker_down = held;
    }

    fn stop(&mut self) {
        if let Some(recording) = self.recording.take() {
            ACTIVE.set(None);
            self.job = Some(std::thread::spawn(move || finish_recording(recording)));
        }
    }

    fn join_report(&mut self) {
        if let Some(job) = self.job.take() {
            match job.join() {
                Ok(Ok(report)) => self.report = Some(report),
                Ok(Err(error)) => self.error = Some(error),
                Err(_) => self.error = Some(tr("报告生成任务异常退出。").to_owned()),
            }
        }
    }

    pub fn defer_finish(&mut self) {
        if let Some(recording) = self.recording.take() {
            let window = recording
                .trace()
                .span("退出", "关闭窗口与释放界面资源", None);
            ACTIVE.set(None);
            crate::shutdown::defer(move || {
                drop(window);
                if let Err(error) = finish_recording(recording) {
                    eprintln!("Atlas Forge performance report: {error}");
                }
            });
        }
        if let Some(job) = self.job.take() {
            crate::shutdown::defer(move || {
                let _ = job.join();
            });
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if self.recording.as_ref().is_some_and(Recording::failed) {
            self.open = true;
            self.stop();
        }
        if let Some(job) = &self.job {
            if job.is_finished() {
                self.join_report();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            }
        }
    }
}

impl App {
    pub(crate) fn performance_window(&mut self, ctx: &egui::Context) {
        self.performance.poll(ctx);
        if !self.performance.open {
            return;
        }
        let mut open = true;
        let mut start = false;
        let performance = &mut self.performance;
        egui::Window::new(tr("性能检测"))
            .id(egui::Id::unique("atlas-performance"))
            .default_width(430.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(tr("录制期间可正常绘图、旋转球体、导入或导出。结束后会生成可离线查看的 HTML 报告和 JSONL 日志。"));
                if let Some(recording) = &performance.recording {
                    let trace = recording.trace();
                    let elapsed = trace.elapsed_ms() / 1000.0;
                    ui.heading(format!("{} · {elapsed:.0} s", tr("正在录制")));
                    ui.horizontal(|ui| {
                        if ui.button(tr("标记此处卡顿（F8）")).clicked() {
                            mark_stall(&trace);
                        }
                        if ui.button(tr("停止并生成报告")).clicked() {
                            performance.stop();
                        }
                    });
                } else if performance.job.is_some() {
                    ui.horizontal(|ui| { ui.spinner(); ui.label(tr("正在生成性能报告…")); });
                } else {
                    start = ui.button(tr("开始录制…")).clicked();
                }
                if let Some(path) = &performance.log {
                    ui.separator();
                    ui.label(tr("原始日志"));
                    ui.label(path.display().to_string());
                }
                if let Some(path) = &performance.report {
                    ui.horizontal(|ui| {
                        if ui.button(tr("打开 HTML 报告")).clicked() {
                            if let Err(error) = open_report(path) { performance.error = Some(error.to_string()); }
                        }
                        if ui.button(tr("复制报告路径")).clicked() {
                            ctx.copy_text(path.display().to_string());
                        }
                    });
                }
                if let Some(error) = &performance.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
            });
        self.performance.open = open;
        if start {
            let timestamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if let Some(path) = self
                .file_dialog()
                .add_filter(tr("性能日志"), &["jsonl"])
                .set_file_name(format!("atlas-perf-{timestamp}.jsonl"))
                .save_file()
            {
                self.performance.start(&path);
            }
        }
    }
}

fn finish_recording(recording: Recording) -> Result<PathBuf, String> {
    let trace = recording.finish().map_err(|e| e.to_string())?;
    let report = map_perf::report::default_path(&trace);
    map_perf::report::write(&trace, &report).map_err(|e| e.to_string())?;
    Ok(report)
}

fn mark_stall(trace: &Trace) {
    trace.emit(Record::Marker {
        label: tr("用户标记的卡顿").to_owned(),
    });
}

fn open_report(path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let launcher = "explorer.exe";
    #[cfg(target_os = "macos")]
    let launcher = "open";
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let launcher = "xdg-open";
    std::process::Command::new(launcher).arg(path).spawn()?;
    Ok(())
}
