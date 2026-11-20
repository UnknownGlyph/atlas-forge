use crate::{app::App, i18n::tr};
use eframe::egui;
use map_core::{
    focus::Focus,
    playback::{Playback, Sample, Sampling},
    projection::Projection,
};
use map_io::{
    batch::{AnimJob, Animation},
    scene::ExportOptions,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(crate) struct TimelinePlayback {
    pub open: bool,
    pub spec: Playback,
    local: bool,
    looped: bool,
    current: Option<Running>,
    video: Option<Video>,
}

struct Running {
    samples: Vec<Sample>,
    cursor: usize,
    next: Instant,
    focus: Option<u32>,
}

struct Video {
    progress: Arc<AtomicUsize>,
    cancel: Arc<AtomicBool>,
    total: usize,
    worker: JoinHandle<Result<Option<PathBuf>, String>>,
}

impl Default for TimelinePlayback {
    fn default() -> Self {
        Self {
            open: false,
            spec: Playback {
                from: 1800,
                to: 2000,
                sampling: Sampling::RecordedChanges,
                duration: 12.0,
                calendar_pacing: false,
                speed: 1.0,
            },
            local: false,
            looped: false,
            current: None,
            video: None,
        }
    }
}

fn bounds(focus: &Focus, opts: &mut ExportOptions) {
    let [w, s, e, n] = focus.bounds;
    opts.projection = Projection::Equirectangular;
    opts.lon0 = map_core::geom::wrap_lon((w + e) * 0.5);
    opts.bounds = Some([
        ((w - e) * 0.5).to_radians(),
        s.to_radians(),
        ((e - w) * 0.5).to_radians(),
        n.to_radians(),
    ]);
}

impl App {
    pub(crate) fn open_playback(&mut self, local: bool) {
        self.playback.open = true;
        self.playback.local = local && self.active_focus().is_some();
        let focus = self.playback.local.then(|| self.active_focus()).flatten();
        let interval = focus.and_then(|f| f.years).or_else(|| {
            let years = map_core::playback::study_years(&self.doc, focus);
            years
                .first()
                .zip(years.last())
                .map(|(&a, &b)| (a.saturating_sub(1), b))
        });
        if let Some((a, b)) = interval {
            self.playback.spec.from = a;
            self.playback.spec.to = b;
        } else if let Some(year) = self.doc.year {
            self.playback.spec.from = year;
            self.playback.spec.to = year.saturating_add(10).min(999_999);
        }
    }

    fn playback_schedule(&self) -> Result<Vec<Sample>, String> {
        let p = self.playback.spec;
        let focus = if self.playback.local {
            Some(self.active_focus().ok_or("请先打开要播放的局部视图。")?)
        } else {
            None
        };
        let years = map_core::playback::study_years(&self.doc, focus)
            .into_iter()
            .collect::<Vec<_>>();
        p.schedule(&years)
    }

    pub(crate) fn stop_timeline_playback(&mut self) {
        self.playback.current = None;
    }

    pub(crate) fn cancel_video_on_exit(&mut self) {
        if let Some(job) = &self.playback.video {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub(crate) fn playback_tick(&mut self, ctx: &egui::Context) {
        if self
            .playback
            .video
            .as_ref()
            .is_some_and(|v| v.worker.is_finished())
        {
            let video = self.playback.video.take().unwrap();
            let result = video
                .worker
                .join()
                .unwrap_or_else(|_| Err("视频编码线程意外结束。".into()));
            match result {
                Ok(Some(path)) => {
                    self.set_status(format!("视频已导出 · {}", path.display()), false)
                }
                Ok(None) => self.set_status("视频导出已取消。", false),
                Err(e) => self.set_status(e, true),
            }
        }
        if self.playback.video.is_some() {
            ctx.request_repaint_after(Duration::from_millis(160));
        }
        let Some(run) = &mut self.playback.current else {
            return;
        };
        if self.doc.year != Some(run.samples[run.cursor].year)
            || (run.focus.is_some() && run.focus != self.focus)
        {
            self.playback.current = None;
            return;
        }
        let now = Instant::now();
        // Seeking resumes on the latest due sample after an idle frame.
        // Render only the selected year, rather than refreshing every skipped year.
        let mut moved = false;
        while now >= run.next {
            run.cursor += 1;
            if run.cursor >= run.samples.len() {
                if !self.playback.looped {
                    let final_year = run.samples.last().unwrap().year;
                    self.playback.current = None;
                    if moved {
                        self.set_year(Some(final_year));
                    }
                    return;
                }
                run.cursor = 0;
                run.next = now;
            }
            run.next += Duration::from_millis(run.samples[run.cursor].hold_ms as u64);
            moved = true;
        }
        let year = run.samples[run.cursor].year;
        ctx.request_repaint_after(run.next.saturating_duration_since(now));
        if moved {
            self.set_year(Some(year));
        }
    }

    pub(crate) fn playback_window(&mut self, ctx: &egui::Context) {
        if !self.playback.open {
            return;
        }
        let active = self.active_focus().map(|f| f.name.clone());
        let p = &mut self.playback;
        let mut open = p.open;
        let (mut start, mut video) = (false, false);
        egui::Window::new(tr("时间播放与视频"))
            .id(egui::Id::unique("timeline-playback"))
            .open(&mut open)
            .default_width(430.0)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.radio_value(&mut p.local, false, tr("全局"));
                    ui.add_enabled_ui(active.is_some(), |ui| {
                        ui.radio_value(&mut p.local, true, tr("当前局部视图"));
                    });
                });
                if let Some(name) = active.filter(|_| p.local) {
                    ui.weak(name);
                }
                ui.horizontal(|ui| {
                    ui.label(tr("起止年份"));
                    ui.add(egui::DragValue::new(&mut p.spec.from).range(-999_999..=999_999));
                    ui.add(egui::DragValue::new(&mut p.spec.to).range(-999_999..=999_999));
                });
                ui.horizontal_wrapped(|ui| {
                    let mut every = matches!(p.spec.sampling, Sampling::Every(_));
                    ui.radio_value(&mut every, false, tr("变化年份"));
                    ui.radio_value(&mut every, true, tr("固定年份间隔"));
                    if every {
                        let mut step = match p.spec.sampling {
                            Sampling::Every(s) => s,
                            _ => 1,
                        };
                        ui.add(
                            egui::DragValue::new(&mut step)
                                .range(1..=100_000)
                                .suffix(tr(" 年")),
                        );
                        p.spec.sampling = Sampling::Every(step);
                    } else {
                        p.spec.sampling = Sampling::RecordedChanges;
                    }
                });
                ui.horizontal_wrapped(|ui| {
                    ui.label(tr("基准总时长"));
                    ui.add(
                        egui::DragValue::new(&mut p.spec.duration)
                            .range(0.1..=3600.0)
                            .speed(0.2)
                            .suffix(" s"),
                    );
                    ui.label(tr("播放倍率"));
                    ui.add(
                        egui::DragValue::new(&mut p.spec.speed)
                            .range(0.1..=32.0)
                            .speed(0.05)
                            .suffix(" ×"),
                    );
                });
                ui.checkbox(&mut p.spec.calendar_pacing, tr("较长的历史间隔停留更久"));
                ui.checkbox(&mut p.looped, tr("循环播放"));
                ui.horizontal_wrapped(|ui| {
                    if p.current.is_some() {
                        if ui.button(tr("停止播放")).clicked() {
                            p.current = None;
                        }
                    } else if ui.button(tr("开始播放")).clicked() {
                        start = true;
                    }
                    if ui
                        .add_enabled(p.video.is_none(), egui::Button::new(tr("导出 MP4")))
                        .clicked()
                    {
                        video = true;
                    }
                });
                if let Some(job) = &p.video {
                    let n = job.progress.load(Ordering::Relaxed);
                    ui.add(
                        egui::ProgressBar::new(n as f32 / job.total.max(1) as f32)
                            .show_percentage(),
                    );
                    if ui.button(tr("取消导出")).clicked() {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                }
            });
        self.playback.open = open;
        if start || video {
            let samples = match self.playback_schedule() {
                Ok(s) => s,
                Err(e) => {
                    self.set_status(e, true);
                    return;
                }
            };
            if start {
                self.focus_play = None;
                self.set_year(Some(samples[0].year));
                self.playback.current = Some(Running {
                    next: Instant::now() + Duration::from_millis(samples[0].hold_ms as u64),
                    samples,
                    cursor: 0,
                    focus: if self.playback.local {
                        self.focus
                    } else {
                        None
                    },
                });
                ctx.request_repaint();
            } else if video {
                self.export_playback(samples);
            }
        }
    }

    fn export_playback(&mut self, samples: Vec<Sample>) {
        let focus = if self.playback.local {
            self.active_focus().cloned()
        } else {
            None
        };
        let Some(path) = self
            .file_dialog()
            .add_filter("MP4", &["mp4"])
            .set_file_name("atlas-history.mp4")
            .save_file()
        else {
            return;
        };
        if !path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("mp4"))
        {
            self.set_status("请选择 .mp4 文件。", true);
            return;
        }
        let mut opts = ExportOptions {
            width: 1920,
            height: 1080,
            projection: self.cam2d.proj,
            lon0: self.cam2d.lon0,
            legend: false,
            title: self.doc.meta.name.clone(),
            ..Default::default()
        };
        if let Some(f) = &focus {
            bounds(f, &mut opts);
        }
        let job = AnimJob {
            years: Vec::new(),
            path,
            kind: Animation::Mp4,
            opts,
            font: self.font.as_deref().cloned(),
            frame_ms: 40,
        };
        let doc = self.doc.clone();
        let total = samples.len();
        let progress = Arc::new(AtomicUsize::new(0));
        let cancel = Arc::new(AtomicBool::new(false));
        let (d, c) = (progress.clone(), cancel.clone());
        let worker = std::thread::spawn(move || {
            map_io::batch::export_scheduled(&doc, &job, &samples, &d, &c)
        });
        self.playback.video = Some(Video {
            progress,
            cancel,
            total,
            worker,
        });
    }
}
