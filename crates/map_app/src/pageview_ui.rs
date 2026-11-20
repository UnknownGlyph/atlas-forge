//! The layout view ("版式视图"): the page as it will be exported, drawn by
//! the exporter itself (in the background, at the export's own size, shown
//! scaled to the window), on which the title, the legend, the scale bar,
//! the north arrow and inset maps are dragged where they should be; an
//! inset's lower right corner resizes it. What is seen is what is
//! exported: the positions go into the export settings and, saved as a
//! layout, with it.

use crate::app::App;
use eframe::egui::{self, vec2, Color32, Pos2, Rect, Sense, Stroke};
use map_core::layout::Inset;
use map_io::scene::Furniture;
use std::sync::mpsc;

type Rendered = Result<(egui::ColorImage, Vec<(Furniture, [f32; 4])>), String>;

/// The page's picture and what is being dragged.
#[derive(Default)]
pub struct PageView {
    pub open: bool,
    texture: Option<egui::TextureHandle>,
    /// What the page shown was made from (the export settings and the
    /// document's history generation).
    shown: Option<String>,
    boxes: Vec<(Furniture, [f32; 4])>,
    size: [usize; 2],
    job: Option<(String, mpsc::Receiver<Rendered>)>,
    drag: Option<Drag>,
    /// The inset being edited.
    pub selected: Option<usize>,
    error: Option<String>,
}

struct Drag {
    what: Furniture,
    start: [f32; 4],
    grab: Pos2,
    resize: bool,
    now: [f32; 4],
}

/// Pages larger than this are not shown (the picture would be too big).
const MAX_PIXELS: u64 = 30_000_000;

impl App {
    /// The part of the projected plane the 2D view shows, when it shows the
    /// export's projection (for an inset's region).
    fn view_bounds_for_page(&self) -> Option<[f64; 4]> {
        let o = self.export_options();
        let c = &self.cam2d;
        if c.proj != o.projection || (c.lon0 - o.lon0).abs() > 1e-9 {
            return None;
        }
        let r = self.last_flat_rect;
        let (hw, hh) = (r.width() as f64 * 0.5 / c.zoom, r.height() as f64 * 0.5 / c.zoom);
        Some([c.center[0] - hw, c.center[1] - hh, c.center[0] + hw, c.center[1] + hh])
    }

    pub fn page_window(&mut self, ctx: &egui::Context) {
        if !self.page.open {
            return;
        }
        let o = self.export_options();
        let key = format!("{o:?}|{}", self.history.generation());
        // a new picture when the page is out of date (one at a time)
        if self.page.shown.as_deref() != Some(key.as_str()) && self.page.job.is_none() {
            if o.width as u64 * o.height as u64 > MAX_PIXELS {
                self.page.error = Some(format!("页面 {} × {} 超过 3000 万像素，版式视图不显示；可先把尺寸调小编排（位置按比例保存），再改回。", o.width, o.height));
                self.page.shown = Some(key.clone());
            } else {
                let (tx, rx) = mpsc::channel();
                let (doc, font, opts) = (self.doc.clone(), self.font.clone(), o.clone());
                std::thread::spawn(move || {
                    let th = map_core::theme::evaluate(&doc);
                    let s = map_io::scene::build_scene(&doc, &th, &opts);
                    let r = map_io::png::render(&s, font.as_deref().map(|v| v.as_slice())).map(|pm| (egui::ColorImage::from_rgba_premultiplied([pm.width() as usize, pm.height() as usize], pm.data()), s.furniture.clone()));
                    let _ = tx.send(r);
                });
                self.page.job = Some((key.clone(), rx));
            }
        }
        if let Some((k, rx)) = &self.page.job {
            match rx.try_recv() {
                Ok(r) => {
                    let k = k.clone();
                    match r {
                        Ok((img, boxes)) => {
                            self.page.size = img.size;
                            match &mut self.page.texture {
                                Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                                None => self.page.texture = Some(ctx.load_texture("layout-page", img, egui::TextureOptions::LINEAR)),
                            }
                            self.page.boxes = boxes;
                            self.page.error = None;
                        }
                        Err(e) => self.page.error = Some(e),
                    }
                    self.page.shown = Some(k);
                    self.page.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => ctx.request_repaint_after(std::time::Duration::from_millis(80)),
                Err(mpsc::TryRecvError::Disconnected) => self.page.job = None,
            }
        }
        let mut open = true;
        egui::Window::new(crate::i18n::tr("版式视图")).open(&mut open).default_size([920.0, 640.0]).resizable(true).show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui.button(crate::i18n::tr("添加插图框")).on_hover_text(crate::i18n::tr("在页面右下角放一个插图框，显示二维视图当前看到的区域（二维视图需与导出的投影相同）；之后可拖动、拖右下角改大小")).clicked() {
                    let world = o.projection.bounds();
                    let bounds = self.view_bounds_for_page().unwrap_or_else(|| {
                        let (cx, cy, dx, dy) = ((world[0] + world[2]) * 0.5, (world[1] + world[3]) * 0.5, (world[2] - world[0]) * 0.1, (world[3] - world[1]) * 0.1);
                        [cx - dx, cy - dy, cx + dx, cy + dy]
                    });
                    self.export.placement.insets.push(Inset { rect: [0.66, 0.6, 0.3, 0.32], bounds, title: String::new() });
                    self.page.selected = Some(self.export.placement.insets.len() - 1);
                }
                if ui.button(crate::i18n::tr("全部复位")).on_hover_text(crate::i18n::tr("标题、图例、比例尺、指北针回到默认位置（插图框不变）")).clicked() {
                    let p = &mut self.export.placement;
                    (p.title, p.legend, p.scale_bar, p.north_arrow) = (None, None, None, None);
                }
                ui.weak(crate::trf!("页面 {} × {}", o.width, o.height));
                if self.page.job.is_some() {
                    ui.add(egui::Spinner::new().size(12.0));
                    ui.weak(crate::i18n::tr("更新中…"));
                }
            });
            // the selected inset: its title, region, removal
            if let Some(i) = self.page.selected.filter(|i| *i < self.export.placement.insets.len()) {
                let view = self.view_bounds_for_page();
                ui.horizontal(|ui| {
                    ui.label(crate::trf!("插图框 {}", i + 1));
                    let inset = &mut self.export.placement.insets[i];
                    ui.add(egui::TextEdit::singleline(&mut inset.title).hint_text(crate::i18n::tr("标题（可空）")).desired_width(140.0));
                    if ui.add_enabled(view.is_some(), egui::Button::new(crate::i18n::tr("范围用当前二维视图"))).on_disabled_hover_text(crate::i18n::tr("二维视图的投影与中央经线需与导出相同")).clicked() {
                        if let Some(b) = view {
                            inset.bounds = b;
                        }
                    }
                    if ui.button(crate::i18n::tr("删除")).clicked() {
                        self.export.placement.insets.remove(i);
                        self.page.selected = None;
                    }
                });
            }
            if let Some(e) = &self.page.error {
                ui.colored_label(Color32::from_rgb(220, 90, 60), e);
            }
            let Some(tex) = &self.page.texture else {
                ui.weak(crate::i18n::tr("正在生成页面…"));
                return;
            };
            let [pw, ph] = [self.page.size[0] as f32, self.page.size[1] as f32];
            let avail = ui.available_size();
            let scale = (avail.x / pw).min((avail.y - 22.0).max(100.0) / ph);
            let (resp, painter) = ui.allocate_painter(vec2(pw * scale, ph * scale), Sense::click_and_drag());
            let r = resp.rect;
            painter.image(tex.id(), r, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), Color32::WHITE);
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, Color32::from_gray(120)), egui::StrokeKind::Outside);
            let screen = |b: [f32; 4]| Rect::from_min_size(r.min + vec2(b[0] * scale, b[1] * scale), vec2(b[2] * scale, b[3] * scale));
            // what is under the pointer: furniture first (drawn on top), then insets
            let order = |f: &Furniture| match f {
                Furniture::Inset(_) => 1,
                _ => 0,
            };
            let mut boxes = self.page.boxes.clone();
            boxes.sort_by_key(|b| order(&b.0));
            let pick = |p: Pos2| boxes.iter().find(|(_, b)| screen(*b).expand(2.0).contains(p)).copied();
            let accent = Color32::from_rgb(230, 120, 20);
            // an inset's resize corner first (other pieces may lie over it)
            let corner = |p: Pos2| boxes.iter().find(|(f, b)| matches!(f, Furniture::Inset(_)) && (p - screen(*b).right_bottom()).length() < 14.0).copied();
            if resp.drag_started() {
                // where the button went down (the drag is noticed a few pixels later)
                let at = ui.input(|i| i.pointer.press_origin()).or(resp.interact_pointer_pos());
                let hit = at.and_then(corner).map(|x| (x, true)).or_else(|| at.and_then(pick).map(|x| (x, false)));
                if let (Some(((what, b), resize)), Some(p)) = (hit, at) {
                    self.page.drag = Some(Drag { what, start: b, grab: p, resize, now: b });
                    if let Furniture::Inset(i) = what {
                        self.page.selected = Some(i);
                    }
                }
            }
            if resp.clicked() {
                self.page.selected = match resp.interact_pointer_pos().and_then(pick) {
                    Some((Furniture::Inset(i), _)) => Some(i),
                    _ => None,
                };
            }
            if let (Some(d), Some(p)) = (&mut self.page.drag, resp.interact_pointer_pos()) {
                let dv = (p - d.grab) / scale;
                let s = d.start;
                d.now = if d.resize { [s[0], s[1], (s[2] + dv.x).clamp(40.0, pw - s[0]), (s[3] + dv.y).clamp(30.0, ph - s[1])] } else { [(s[0] + dv.x).clamp(0.0, (pw - s[2]).max(0.0)), (s[1] + dv.y).clamp(0.0, (ph - s[3]).max(0.0)), s[2], s[3]] };
            }
            if resp.drag_stopped() {
                if let Some(d) = self.page.drag.take() {
                    let at = [d.now[0] / pw, d.now[1] / ph];
                    let p = &mut self.export.placement;
                    match d.what {
                        Furniture::Title => p.title = Some(at),
                        Furniture::Legend => p.legend = Some(at),
                        Furniture::ScaleBar => p.scale_bar = Some(at),
                        Furniture::NorthArrow => p.north_arrow = Some(at),
                        Furniture::Inset(i) => {
                            if let Some(inset) = p.insets.get_mut(i) {
                                inset.rect = [at[0], at[1], d.now[2] / pw, d.now[3] / ph];
                            }
                        }
                    }
                }
            }
            // every piece's box; the hovered and the selected brighter; the dragged one where it goes
            let hover = resp.hover_pos().and_then(pick).map(|x| x.0);
            for (what, b) in &self.page.boxes {
                let sel = matches!(what, Furniture::Inset(i) if Some(*i) == self.page.selected);
                let strong = sel || Some(*what) == hover;
                let rect = screen(*b);
                let pts = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
                painter.extend(egui::Shape::dashed_line(&pts, Stroke::new(if strong { 2.0 } else { 1.0 }, accent.gamma_multiply(if strong { 1.0 } else { 0.6 })), 6.0, 4.0));
                if matches!(what, Furniture::Inset(_)) {
                    painter.rect_filled(Rect::from_center_size(rect.right_bottom(), vec2(8.0, 8.0)), 1.0, accent);
                }
            }
            if let Some(d) = &self.page.drag {
                painter.rect_stroke(screen(d.now), 0.0, Stroke::new(2.0, accent), egui::StrokeKind::Middle);
            }
            if hover.is_some() {
                ctx.set_cursor_icon(egui::CursorIcon::Grab);
            }
            ui.weak(crate::i18n::tr("拖动标题、图例、比例尺、指北针或插图框改变位置；拖插图框右下角改大小。位置随导出设置保存，也会存进版式。"));
        });
        if !open {
            self.page.open = false;
            self.page.drag = None;
        }
    }
}
