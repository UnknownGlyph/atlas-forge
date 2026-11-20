//! Reference images: decoding cache, the alignment tool (drag a feature of
//! the image to where it really is) and the "参考图" panel.

use crate::app::App;
use crate::view::{ViewGeom, ViewKind};
use eframe::egui::{self, Color32, Stroke};
use map_core::geom::LonLat;
use map_core::projection::Projection;
use map_core::underlay::{Blob, ControlPoint, Underlay};
use map_render::camera::{Camera2D, GlobeCamera};
use map_render::underlay::{RgbaImage, UnderlayFrame, UnderlayUniform};
use std::sync::{mpsc, Arc};

/// At most this many pixels are kept on the GPU per image (~250 MB with mips).
const MAX_PIXELS: u64 = 48 << 20;

enum Decode {
    Running(mpsc::Receiver<Result<map_io::underlay::Decoded, String>>),
    Ready { img: Arc<RgbaImage>, scaled: bool },
    Failed(String),
}

struct Entry {
    blob: Blob,
    key: u64,
    state: Decode,
}

pub struct UnderlayState {
    cache: Vec<Entry>,
    next_key: u64,
    /// Largest texture side the GPU accepts.
    pub max_dim: u32,
    /// Underlay shown in the panel and moved by the alignment tool.
    pub sel: Option<usize>,
    /// Alignment drag in progress: underlay index and the image pixel grabbed.
    drag: Option<(usize, [f64; 2])>,
    /// That underlay with the provisional point, drawn while dragging.
    preview: Option<(usize, Underlay)>,
}

impl UnderlayState {
    pub fn new(max_dim: u32) -> Self {
        Self { cache: vec![], next_key: 1, max_dim, sel: None, drag: None, preview: None }
    }
}

impl App {
    fn underlay_shown(&self, i: usize) -> &Underlay {
        match &self.ul.preview {
            Some((j, u)) if *j == i => u,
            _ => &self.doc.underlays[i],
        }
    }

    /// Start decoding new images, collect finished ones, drop unused ones.
    pub fn poll_underlays(&mut self, ctx: &egui::Context) {
        let doc = &self.doc;
        self.ul.cache.retain(|e| doc.underlays.iter().any(|u| Arc::ptr_eq(&u.image.0, &e.blob.0)));
        for u in &self.doc.underlays {
            if !self.ul.cache.iter().any(|e| Arc::ptr_eq(&u.image.0, &e.blob.0)) {
                let (tx, rx) = mpsc::channel();
                let (bytes, max_dim, ctx) = (u.image.0.clone(), self.ul.max_dim, ctx.clone());
                std::thread::spawn(move || {
                    let _ = tx.send(map_io::underlay::decode(&bytes, max_dim, MAX_PIXELS));
                    ctx.request_repaint();
                });
                self.ul.cache.push(Entry { blob: u.image.clone(), key: self.ul.next_key, state: Decode::Running(rx) });
                self.ul.next_key += 1;
            }
        }
        for e in &mut self.ul.cache {
            if let Decode::Running(rx) = &e.state {
                match rx.try_recv() {
                    Ok(Ok(d)) => e.state = Decode::Ready { img: Arc::new(RgbaImage { w: d.w, h: d.h, pixels: d.pixels }), scaled: d.scaled },
                    Ok(Err(msg)) => e.state = Decode::Failed(msg),
                    Err(mpsc::TryRecvError::Empty) => {}
                    Err(mpsc::TryRecvError::Disconnected) => e.state = Decode::Failed("解码线程意外退出".into()),
                }
            }
        }
        if self.ul.sel.is_some_and(|i| i >= self.doc.underlays.len()) || (self.ul.sel.is_none() && !self.doc.underlays.is_empty()) {
            self.ul.sel = self.doc.underlays.len().checked_sub(1);
        }
    }

    /// GPU draw list for one view (`cam` already scaled to physical pixels).
    pub fn underlay_frames(&self, kind: ViewKind, cam: &Camera2D, globe: &GlobeCamera, w: f32, h: f32) -> Vec<UnderlayFrame> {
        let mut out = vec![];
        // the terrain's hill shading, over the land, under the lines
        if let (Some(t), Some((ptr, img))) = (self.doc.terrain.as_ref(), self.terrain_img.as_ref()) {
            let k = self.doc.style.hillshade.clamp(0.0, 1.0);
            if k > 0.0 {
                let g = t.georef();
                let uniform = match kind {
                    ViewKind::Flat => UnderlayUniform::for_2d(&g, [t.w, t.h], k, cam, w, h),
                    ViewKind::Globe => UnderlayUniform::for_globe(&g, [t.w, t.h], k, globe, w, h),
                };
                out.push(UnderlayFrame { key: 0x7E77_A1A0_0000_0000 ^ *ptr as u64, image: img.clone(), uniform, above_land: true });
            }
        }
        for i in 0..self.doc.underlays.len() {
            let u = self.underlay_shown(i);
            if !u.visible || u.opacity <= 0.0 {
                continue;
            }
            let Some(e) = self.ul.cache.iter().find(|e| Arc::ptr_eq(&e.blob.0, &u.image.0)) else { continue };
            let Decode::Ready { img, .. } = &e.state else { continue };
            let g = u.fit().georef;
            let uniform = match kind {
                ViewKind::Flat => UnderlayUniform::for_2d(&g, u.size, u.opacity, cam, w, h),
                ViewKind::Globe => UnderlayUniform::for_globe(&g, u.size, u.opacity, globe, w, h),
            };
            out.push(UnderlayFrame { key: e.key, image: img.clone(), uniform, above_land: u.above_land });
        }
        if let Some(p) = self.doc.climate.as_deref().filter(|p| p.visible && p.opacity > 0.0) {
            if let Some((key, image)) = self.climate_ui.image() {
                let size = [image.w, image.h];
                let g = map_core::terrain::global_georef(image.w, image.h);
                let uniform = match kind {
                    ViewKind::Flat => UnderlayUniform::for_2d(&g, size, p.opacity, cam, w, h),
                    ViewKind::Globe => UnderlayUniform::for_globe(&g, size, p.opacity, globe, w, h),
                };
                out.push(UnderlayFrame { key, image, uniform, above_land: true });
            }
        }
        if let Some(project) = self.doc.environment.as_deref().filter(|p| p.visible && p.opacity > 0.0) {
            if let Some((key, image, georef)) = self.environment_ui.raster() {
                let size = [image.w, image.h];
                let uniform = match kind {
                    ViewKind::Flat => UnderlayUniform::for_2d(&georef, size, project.opacity, cam, w, h),
                    ViewKind::Globe => UnderlayUniform::for_globe(&georef, size, project.opacity, globe, w, h),
                };
                out.push(UnderlayFrame { key, image, uniform, above_land: true });
            }
        }
        out
    }

    pub fn import_underlay(&mut self) {
        let Some(p) = self.file_dialog().add_filter(crate::i18n::tr("图片"), &["png", "jpg", "jpeg", "tif", "tiff", "webp", "bmp"]).pick_file() else { return };
        match map_io::underlay::import(&p) {
            Ok(u) => {
                let (name, [w, h]) = (u.name.clone(), u.size);
                if self.edit("导入参考图片", move |d| {
                    d.underlays.push(u);
                    Ok(())
                })
                .is_some()
                {
                    self.ul.sel = Some(self.doc.underlays.len() - 1);
                    self.tab = crate::panels::Tab::Underlay;
                    self.set_status(crate::trf!("已导入参考图片 {name}（{w}×{h}），先按全球等距圆柱图放置。用“配准参考图”工具把图上地物拖到真实位置。"), false);
                }
            }
            Err(e) => self.set_status(crate::trf!("导入参考图片失败：{e}"), true),
        }
    }

    pub fn align_input(&mut self, resp: &egui::Response, vg: &ViewGeom, ll: Option<LonLat>) {
        use egui::PointerButton::{Primary, Secondary};
        if resp.drag_stopped() {
            if let (Some((i, px)), Some(p)) = (self.ul.drag.take(), ll) {
                self.ul.preview = None;
                if self.edit("添加配准点", move |d| {
                    d.underlays[i].points.push(ControlPoint { px, ll: p });
                    Ok(())
                })
                .is_some()
                {
                    let f = self.doc.underlays[i].fit();
                    self.set_status(crate::trf!("已添加配准点 {}：{}，残差均方根 {:.1} 像素", self.doc.underlays[i].points.len(), f.kind.name(), f.rms_px), false);
                }
            }
            self.ul.drag = None;
            self.ul.preview = None;
            return;
        }
        let Some(i) = self.ul.sel.filter(|i| *i < self.doc.underlays.len()) else {
            if resp.clicked() || resp.drag_started() {
                self.set_status(crate::i18n::tr("请先在“参考图”面板导入或选择一张图片"), true);
            }
            return;
        };
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).and_then(|o| vg.to_lonlat(o)).or(ll);
            let u = &self.doc.underlays[i];
            let Some(o) = origin else { return };
            let px = u.fit().georef.pixel_of(o);
            if !u.visible {
                self.set_status(crate::i18n::tr("这张参考图已隐藏"), true);
            } else if !(0.0..=u.size[0] as f64).contains(&px[0]) || !(0.0..=u.size[1] as f64).contains(&px[1]) {
                self.set_status(crate::i18n::tr("这里没有图片：从图片上的地物开始拖动"), true);
            } else {
                self.ul.drag = Some((i, px));
            }
        }
        if let (Some((di, px)), Some(p)) = (self.ul.drag, ll) {
            if resp.dragged_by(Primary) {
                let mut u = self.doc.underlays[di].clone();
                u.points.push(ControlPoint { px, ll: p });
                self.ul.preview = Some((di, u));
            }
        }
        if resp.clicked_by(Secondary) {
            // right click on a point's true position removes it
            let pos = resp.interact_pointer_pos();
            let hit = pos.and_then(|pos| {
                self.doc.underlays[i]
                    .points
                    .iter()
                    .enumerate()
                    .filter_map(|(k, c)| vg.to_screen(c.ll).map(|s| (k, s.distance(pos))))
                    .filter(|x| x.1 < 9.0)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
            });
            if let Some((k, _)) = hit {
                self.edit("删除配准点", move |d| {
                    d.underlays[i].points.remove(k);
                    Ok(())
                });
            }
        }
    }

    /// Control points of the selected underlay: where each point belongs
    /// (ring) and where the fitted image currently puts it (line = residual).
    pub fn draw_underlay_marks(&self, painter: &egui::Painter, vg: &ViewGeom) {
        let Some(i) = self.ul.sel.filter(|i| *i < self.doc.underlays.len()) else { return };
        if self.tool != crate::view::Tool::Align {
            return;
        }
        let u = self.underlay_shown(i);
        let g = u.fit().georef;
        for (k, c) in u.points.iter().enumerate() {
            let Some(t) = vg.to_screen(c.ll) else { continue };
            if let Some(f) = g.lonlat_of(c.px).and_then(|ll| vg.to_screen(ll)) {
                painter.line_segment([f, t], Stroke::new(2.0, Color32::from_rgb(230, 40, 40)));
            }
            painter.circle(t, 5.0, Color32::WHITE, Stroke::new(2.0, Color32::from_rgb(200, 30, 30)));
            painter.text(t + egui::vec2(7.0, -7.0), egui::Align2::LEFT_BOTTOM, format!("{}", k + 1), egui::FontId::proportional(13.0), Color32::from_rgb(160, 0, 0));
        }
    }

    pub fn underlay_tab(&mut self, ui: &mut egui::Ui) {
        if ui.button(crate::i18n::tr("导入参考图片…")).clicked() {
            self.import_underlay();
        }
        ui.weak(crate::i18n::tr("参考图片只用于描绘，不进入导出结果；图片原文件保存在工程中。"));
        if self.doc.underlays.is_empty() {
            return;
        }
        ui.separator();
        let mut remove = None;
        for (i, u) in self.doc.underlays.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.selectable_label(self.ul.sel == Some(i), &u.name).clicked() {
                    self.ul.sel = Some(i);
                }
                if ui.small_button(crate::i18n::tr("删除")).clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.edit("删除参考图片", move |d| {
                d.underlays.remove(i);
                Ok(())
            });
            return;
        }
        let Some(i) = self.ul.sel.filter(|i| *i < self.doc.underlays.len()) else { return };
        ui.separator();
        let status = self.ul.cache.iter().find(|e| Arc::ptr_eq(&e.blob.0, &self.doc.underlays[i].image.0)).map(|e| match &e.state {
            Decode::Running(_) => ("正在解码…".to_string(), false),
            Decode::Ready { img, scaled: true } => (format!("显示时缩小为 {}×{}（显卡纹理限制）", img.w, img.h), false),
            Decode::Ready { .. } => (String::new(), false),
            Decode::Failed(e) => (e.clone(), true),
        });
        let fit = self.doc.underlays[i].fit();
        let ranks = (self.doc.underlays[i].points.len() >= 4).then(|| self.doc.underlays[i].rank_projections());
        let mut clear = false;
        let mut del_point = None;
        let u = &mut self.doc.underlays[i];
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("名称"));
            ui.text_edit_singleline(&mut u.name);
        });
        ui.label(crate::trf!("{}×{} 像素，{:.1} MB", u.size[0], u.size[1], u.image.0.len() as f64 / 1048576.0));
        if let Some((s, err)) = status {
            if !s.is_empty() {
                ui.colored_label(if err { Color32::from_rgb(200, 60, 60) } else { ui.visuals().weak_text_color() }, s);
            }
        }
        ui.checkbox(&mut u.visible, crate::i18n::tr("显示"));
        ui.add(egui::Slider::new(&mut u.opacity, 0.0..=1.0).text(crate::i18n::tr("不透明度")));
        ui.radio_value(&mut u.above_land, true, crate::i18n::tr("盖在陆地上（边界线之下）"));
        ui.radio_value(&mut u.above_land, false, crate::i18n::tr("只在陆地下层（透过海洋可见）"));
        ui.separator();
        ui.label(egui::RichText::new(crate::i18n::tr("图片的投影")).strong());
        egui::ComboBox::from_id_salt("ul_proj").selected_text(u.proj.name()).show_ui(ui, |ui| {
            for p in Projection::ALL {
                ui.selectable_value(&mut u.proj, p, p.name());
            }
        });
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("中央经线"));
            ui.add(egui::DragValue::new(&mut u.lon0).range(-180.0..=180.0).speed(0.5).suffix("°"));
        });
        ui.label(egui::RichText::new(crate::i18n::tr("初始范围（没有配准点时使用）")).strong());
        ui.horizontal(|ui| {
            for (k, l) in ["西", "南", "东", "北"].iter().enumerate() {
                ui.label(*l);
                let r = if k % 2 == 0 { -360.0..=360.0 } else { -90.0..=90.0 };
                ui.add(egui::DragValue::new(&mut u.bounds[k]).range(r).speed(0.5));
            }
        });
        ui.separator();
        ui.label(egui::RichText::new(crate::i18n::tr("配准点")).strong());
        ui.weak(crate::i18n::tr("选择“配准参考图”工具 (A)，在地图上把图片中的地物拖到它的真实位置。1 个点平移，2 个点平移+缩放+旋转，3 个点以上仿射。右键单击点的位置删除。"));
        ui.label(crate::trf!("当前：{}{}", fit.kind.name(), if u.points.is_empty() { String::new() } else { format!("，残差均方根 {:.1} 像素", fit.rms_px) }));
        egui::Grid::new("ul_points").striped(true).show(ui, |ui| {
            ui.label("#");
            ui.label(crate::i18n::tr("经度"));
            ui.label(crate::i18n::tr("纬度"));
            ui.label(crate::i18n::tr("残差 px"));
            ui.label("");
            ui.end_row();
            for (k, c) in u.points.iter_mut().enumerate() {
                ui.label(format!("{}", k + 1));
                ui.add(egui::DragValue::new(&mut c.ll.lon).range(-180.0..=180.0).speed(0.01).max_decimals(5));
                ui.add(egui::DragValue::new(&mut c.ll.lat).range(-90.0..=90.0).speed(0.01).max_decimals(5));
                ui.label(format!("{:.1}", fit.residuals_px.get(k).copied().unwrap_or(0.0)));
                if ui.small_button("✕").clicked() {
                    del_point = Some(k);
                }
                ui.end_row();
            }
        });
        if !u.points.is_empty() && ui.button(crate::i18n::tr("清除全部配准点")).clicked() {
            clear = true;
        }
        if let Some(r) = ranks {
            ui.separator();
            ui.label(egui::RichText::new(crate::i18n::tr("各投影的拟合残差")).strong());
            let cur = u.proj;
            let mut pick = None;
            for (p, rms) in r {
                ui.horizontal(|ui| {
                    ui.label(format!("{}：{:.1} px", p.name(), rms));
                    if p != cur && ui.small_button(crate::i18n::tr("改用")).clicked() {
                        pick = Some(p);
                    }
                });
            }
            if let Some(p) = pick {
                u.proj = p;
            }
        }
        if let Some(k) = del_point {
            self.edit("删除配准点", move |d| {
                d.underlays[i].points.remove(k);
                Ok(())
            });
        }
        if clear {
            self.edit("清除配准点", move |d| {
                d.underlays[i].points.clear();
                Ok(())
            });
        }
    }
}
