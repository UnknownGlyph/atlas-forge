use crate::{app::App, i18n::tr};
use eframe::egui::{self, ColorImage, TextureHandle};
use map_api::automation::{self, BatchReport, VisualStep};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

type Run = Result<(BatchReport, Vec<Arc<VisualStep>>), String>;
type Preview = (usize, Result<ColorImage, String>);

#[derive(Default)]
pub(crate) struct Replay {
    pub open: bool,
    base: u64,
    run: Option<mpsc::Receiver<Run>>,
    report: Option<BatchReport>,
    steps: Vec<Arc<VisualStep>>,
    index: usize,
    pending_preview: Option<mpsc::Receiver<Preview>>,
    displayed: Option<(usize, TextureHandle)>,
    failed_preview: Option<usize>,
    next: Option<Instant>,
    error: Option<String>,
}

impl App {
    pub(crate) fn replay_window(&mut self, ctx: &egui::Context) {
        let state = &mut self.replay;
        if !state.open { return; }
        if let Some(rx) = &state.run {
            match rx.try_recv() {
                Ok(Ok((report, steps))) => {
                    state.run = None;
                    state.report = Some(report);
                    state.steps = steps;
                    state.index = 0;
                }
                Ok(Err(error)) => { state.run = None; state.error = Some(error); }
                Err(mpsc::TryRecvError::Disconnected) => {
                    state.run = None;
                    state.error = Some(tr("脚本线程已停止。").into());
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
            }
        }
        if let Some(rx) = &state.pending_preview {
            match rx.try_recv() {
                Ok((index, result)) => {
                    state.pending_preview = None;
                    match result {
                        Ok(image) if index == state.index => state.displayed = Some((index,
                            ctx.load_texture("atlas-replay-frame", image, egui::TextureOptions::LINEAR))),
                        Err(error) if index == state.index => {
                            state.failed_preview = Some(index);
                            state.error = Some(error);
                        }
                        _ => {}
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => state.pending_preview = None,
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(80)),
            }
        }
        let mut choose = false;
        let mut apply = false;
        let mut open = state.open;
        let busy = state.run.is_some();
        egui::Window::new(tr("脚本可视步骤"))
            .id(egui::Id::unique("atlas-replay"))
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .default_width(720.0)
            .open(&mut open)
            .show(ctx, |ui| {
                if ui.add_enabled(!busy, egui::Button::new(tr("用当前工程运行脚本…"))).clicked() {
                    choose = true;
                }
                if busy {
                    ui.spinner();
                }
                if let Some(error) = &state.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                if let Some(failed) = state.report.as_ref().and_then(|report| report.failed_at) {
                    ui.colored_label(ui.visuals().error_fg_color, crate::trf!("第 {failed} 步失败，工程未被修改。"));
                    if let Some(message) = state.report.as_ref().and_then(|r| r.results.get(failed-1))
                        .and_then(|reply| reply["error"]["message"].as_str()) {
                        ui.label(message);
                    }
                }
                let Some(last) = state.steps.len().checked_sub(1) else { return };
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(state.index > 0, egui::Button::new(tr("上一步"))).clicked() {
                        state.index -= 1;
                        state.next = None;
                    }
                    if ui.add_enabled(state.index < last, egui::Button::new(tr("下一步"))).clicked() {
                        state.index += 1;
                        state.next = None;
                    }
                    if ui.button(if state.next.is_some() { tr("停止播放") } else { tr("开始播放") }).clicked() {
                        if state.next.take().is_none() {
                            if state.index == last { state.index = 0; }
                            state.next = Some(Instant::now() + Duration::from_millis(950));
                        }
                    }
                    if ui.add(egui::Slider::new(&mut state.index, 0..=last).text(tr("步骤"))).changed() {
                        state.next = None;
                    }
                    let allows_apply = !busy && last > 0 && state.report.as_ref().is_some_and(|report| report.ok)
                        && self.history.generation() == state.base;
                    apply = ui.add_enabled(allows_apply, egui::Button::new(tr("采用最终工程"))).clicked();
                });
                let step = &state.steps[state.index];
                if step.number == 0 {
                    ui.strong(tr("开始前"));
                } else {
                    ui.strong(format!("{}/{}  {}", step.number, last, step.operation));
                }
                if let Some((index, texture)) = &state.displayed {
                    if *index == state.index {
                        let max_h = (ctx.content_rect().height() - 245.0).max(120.0);
                        let w = ui.available_width().min(max_h * 900.0 / 500.0);
                        ui.image((texture.id(), egui::vec2(w, w * 500.0 / 900.0)));
                    } else {
                        ui.spinner();
                    }
                }
            });
        state.open = open;
        if !open {
            state.next = None;
        }
        if let Some(next) = state.next {
            let now = Instant::now();
            let current_ready = state.displayed.as_ref().is_some_and(|(index, _)| *index == state.index)
                || state.failed_preview == Some(state.index);
            if now >= next && current_ready {
                if state.index + 1 < state.steps.len() {
                    state.index += 1;
                    state.next = Some(now + Duration::from_millis(950));
                } else {
                    state.next = None;
                }
            }
            ctx.request_repaint_after(Duration::from_millis(90));
        }
        if state.open && state.pending_preview.is_none()
            && state.displayed.as_ref().is_none_or(|(index, _)| *index != state.index)
            && state.failed_preview != Some(state.index)
        {
            if let Some(step) = state.steps.get(state.index).cloned() {
                let index = state.index;
                let font = self.font.clone();
                let (tx, rx) = mpsc::channel();
                state.pending_preview = Some(rx);
                let ctx = ctx.clone();
                crate::perf_ui::spawn("加载脚本动作", move || {
                    let image = step.preview(900, 500, font.as_deref().map(Vec::as_slice)).and_then(|png| {
                        let rgba = image::load_from_memory(&png).map_err(|e| e.to_string())?.to_rgba8();
                        Ok(ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw()))
                    });
                    let _ = tx.send((index, image));
                    ctx.request_repaint();
                });
            }
        }
        if apply {
            if let Some(step) = state.steps.last() {
                let result = step.document.clone();
                self.edit("采用脚本可视步骤", |world| { *world = result; Ok(()) });
                self.replay.open = false;
            }
        }
        if choose {
            let Some(path) = self.file_dialog().add_filter(tr("自动化脚本"), &["json"]).pick_file() else { return };
            let world = self.doc.clone();
            let ctx = ctx.clone();
            let (tx, rx) = mpsc::channel();
            self.replay = Replay {
                open: true,
                base: self.history.generation(),
                run: Some(rx),
                ..Default::default()
            };
            crate::perf_ui::spawn("生成脚本预览", move || {
                let result = automation::load_script(&path).map(|requests| {
                    let mut session = map_api::Session::new(world, "Atlas Forge");
                    let (report, steps) = automation::run_visual_steps(&mut session, &requests);
                    (report, steps.into_iter().map(Arc::new).collect())
                });
                let _ = tx.send(result);
                ctx.request_repaint();
            });
        }
    }
}
