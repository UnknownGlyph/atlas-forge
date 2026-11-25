//! The selection of drawn features (map text, symbols, rivers and routes,
//! hatched areas) in the select tool: one or several at once. A click
//! picks, Shift or Ctrl adds and removes, Shift-drag draws a box; a drag
//! on a selected feature moves them all together. Copy, cut and paste go
//! through the system clipboard (so they work between windows); a panel
//! turns, scales, lines up and spaces the selection. The work itself is
//! `map_core::arrange`.

use crate::app::App;
use crate::view::{Tool, ViewGeom};
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke};
use map_core::arrange::{Align, Clip};
use map_core::features::FeatureRef;
use map_core::LonLat;

const ACCENT: Color32 = Color32::from_rgb(40, 110, 200);

/// A move of the selection in progress: what, where the drag began, and
/// the features as they were (each frame moves these, so nothing drifts).
pub struct MoveDrag {
    fs: Vec<FeatureRef>,
    origin: LonLat,
    original: Clip,
    /// Key points of what is moved, as they were, and of everything else
    /// (for smart snapping).
    keys: Vec<LonLat>,
    targets: Vec<LonLat>,
}

impl App {
    fn line_ref(&self, id: map_core::doc::LineId) -> FeatureRef {
        if self.doc.rivers.get(&id).is_some_and(|r| r.route.is_some()) {
            FeatureRef::Route(id)
        } else {
            FeatureRef::River(id)
        }
    }

    /// The selected drawn features: those picked in the select tool, and
    /// the one a tool's own panel is showing.
    pub fn selection(&self) -> Vec<FeatureRef> {
        let mut v: Vec<FeatureRef> = self.picked.iter().copied().collect();
        let mut add = |f: FeatureRef| {
            if !v.contains(&f) {
                v.push(f);
            }
        };
        if let Some(id) = self.selected_area.filter(|_| self.draft.is_empty()) {
            add(FeatureRef::Area(id));
        }
        if let Some(id) = self.selected_river {
            add(self.line_ref(id));
        }
        if let Some(id) = self.selected_sym.filter(|_| self.tool == Tool::Symbol) {
            add(FeatureRef::Symbol(id));
        }
        if let Some(id) = self.selected_ann.filter(|_| self.tool == Tool::Label) {
            add(FeatureRef::Text(id));
        }
        v.retain(|f| self.doc.feature_exists(*f));
        v
    }

    /// Select exactly these features. A single line or area also opens in
    /// its panel, as a click on it always did.
    pub fn set_selection(&mut self, fs: Vec<FeatureRef>) {
        self.selected = None;
        self.selected_face = None;
        self.selected_river = None;
        self.selected_area = None;
        if self.tool != Tool::Symbol {
            self.selected_sym = None;
        }
        if self.tool != Tool::Label {
            self.selected_ann = None;
        }
        if let [one] = fs[..] {
            match one {
                FeatureRef::Route(id) | FeatureRef::River(id) => self.selected_river = Some(id),
                FeatureRef::Area(id) => self.selected_area = Some(id),
                FeatureRef::Symbol(id) if self.tool == Tool::Symbol => self.selected_sym = Some(id),
                FeatureRef::Text(id) if self.tool == Tool::Label => self.selected_ann = Some(id),
                _ => {}
            }
        }
        self.picked = fs.into_iter().collect();
    }

    pub fn clear_selection(&mut self) {
        self.set_selection(vec![]);
    }

    /// The drawn feature under a screen position: symbols and text by their
    /// drawn boxes, lines and areas by their lines. Locked ones are passed over.
    pub fn feature_at(&self, o: Pos2, vg: &ViewGeom) -> Option<FeatureRef> {
        let free = |f: &FeatureRef| !self.doc.locked_in_effect(*f);
        let i = vg.kind.index();
        if let Some(f) = self.sym_rects.borrow()[i].iter().rev().find(|(_, r)| r.contains(o)).map(|(id, _)| FeatureRef::Symbol(*id)).filter(free) {
            return Some(f);
        }
        if let Some(f) = self.ann_rects.borrow()[i].iter().rev().find(|(_, r)| r.contains(o)).map(|(id, _)| FeatureRef::Text(*id)).filter(free) {
            return Some(f);
        }
        let p = vg.to_lonlat(o)?;
        let tol = vg.deg_per_px(o, 8.0);
        let detail = self.map_detail(vg);
        if let Some(f) = self.doc.river_at_where(p, tol, |id, line| self.line_shown(id, line, detail) && free(&self.line_ref(id))).map(|h| self.line_ref(h.0)) {
            return Some(f);
        }
        self.doc.area_edge_at(p, tol).map(FeatureRef::Area).filter(free)
    }

    /// Select tool: a click (or right click) on a drawn feature. Returns
    /// whether it was one, so the click is not also taken for the land under it.
    pub fn select_feature_click(&mut self, resp: &egui::Response, vg: &ViewGeom) -> bool {
        let Some(o) = resp.interact_pointer_pos() else { return false };
        let add = self.press_mods.shift || self.press_mods.command;
        let Some(hit) = self.feature_at(o, vg) else {
            // Shift-click on nothing keeps what is selected
            return add && !self.selection().is_empty();
        };
        let mut sel = self.selection();
        if resp.secondary_clicked() && sel.contains(&hit) {
            return true; // the menu is for the whole selection
        }
        if resp.double_clicked() && matches!(hit, FeatureRef::Symbol(_) | FeatureRef::Text(_)) {
            // their own tools edit them
            self.select_feature(hit);
            return true;
        }
        if add {
            match sel.iter().position(|f| *f == hit) {
                Some(i) => {
                    sel.remove(i);
                }
                None => sel.push(hit),
            }
        } else {
            sel = vec![hit];
        }
        let n = sel.len();
        self.set_selection(sel);
        self.tab = crate::panels::Tab::Countries;
        self.set_status(
            match n {
                0 => "已取消选择".to_string(),
                1 => crate::trf!("已选择{}（拖动可移动，Delete 删除；Shift 单击加选）", hit.kind_name()),
                _ => crate::trf!("已选择 {n} 个要素（拖动任意一个可一起移动）"),
            },
            false,
        );
        true
    }

    /// Select tool: a drag that starts on a selected feature moves the
    /// whole selection; a Shift-drag draws a selection box. Called before
    /// navigation; returns whether the drag is taken (the map then stays).
    pub fn move_drag(&mut self, resp: &egui::Response, vg: &ViewGeom) -> bool {
        use egui::PointerButton::Primary;
        if self.tool != Tool::Select {
            return false;
        }
        if resp.drag_started_by(Primary) && !self.space_pan {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let shift = self.press_mods.shift;
            if let Some(o) = origin {
                let sel = self.selection();
                let on_selected = !sel.is_empty() && self.feature_at(o, vg).is_some_and(|h| sel.contains(&h));
                if shift {
                    self.gestures.marquee = Some((vg.kind.index(), o, o));
                } else if let (true, Some(q)) = (on_selected, vg.to_lonlat(o)) {
                    self.begin_pending(if sel.len() == 1 { "移动要素" } else { "移动多个要素" });
                    let original = self.doc.copy_features(&sel);
                    let keys = sel.iter().flat_map(|f| self.doc.key_points(*f)).collect();
                    let targets = self.doc.snap_targets(&sel);
                    self.gestures.features = Some(MoveDrag { fs: sel, origin: q, original, keys, targets });
                }
            }
        }
        if let Some((view, from, _)) = self.gestures.marquee {
            // the other view leaves the box alone (and stays still)
            if view != vg.kind.index() {
                return true;
            }
            if let Some(p) = resp.interact_pointer_pos() {
                self.gestures.marquee = Some((view, from, p));
            }
            if resp.drag_stopped() || !resp.dragged_by(Primary) {
                let (_, a, b) = self.gestures.marquee.take().unwrap();
                let rect = Rect::from_two_pos(a, b);
                let mut sel = self.selection();
                for f in self.doc.features() {
                    let inside = self.doc.pickable(f.what) && self.doc.feature_anchor(f.what).and_then(|p| vg.to_screen(p)).is_some_and(|s| rect.contains(s));
                    if inside && !sel.contains(&f.what) {
                        sel.push(f.what);
                    }
                }
                let n = sel.len();
                self.set_selection(sel);
                self.set_status(crate::trf!("已选择 {n} 个要素"), false);
            }
            return true;
        }
        let Some(m) = &self.gestures.features else { return false };
        if resp.dragged_by(Primary) {
            let at = resp.interact_pointer_pos();
            if let Some(mut p) = at.and_then(|p| vg.to_lonlat(p)) {
                // smart snapping (Alt held: not this time): a key point onto
                // another feature's, or lined up with one or with the graticule
                let alt = resp.ctx.input(|i| i.modifiers.alt);
                self.snap_guides.clear();
                if self.smart_snap && !alt && !m.keys.is_empty() {
                    let tol = vg.deg_per_px(at.unwrap(), 8.0);
                    let grid = self.graticule.then_some(15.0);
                    for round in 0..2 {
                        let keys = map_core::copy::moved(&m.keys, m.origin, p);
                        let s = map_core::snap::snap(&keys, &m.targets, grid, if round == 0 { tol } else { tol * 0.25 });
                        p = LonLat::new(map_core::geom::wrap_lon(p.lon + s.dlon), (p.lat + s.dlat).clamp(-89.9, 89.9));
                        if round == 0 {
                            self.snap_guides = s.guides;
                        }
                    }
                }
                // from the features as they were: the grabbed point follows
                // the cursor, shapes and their distances are kept
                let (fs, origin, original) = (m.fs.clone(), m.origin, m.original.clone());
                let mut ok = true;
                for (i, f) in fs.iter().enumerate() {
                    ok &= self.doc.set_feature_from(*f, &original, i).is_ok();
                }
                if ok && self.doc.move_features(&fs, origin, p).is_ok() {
                    if let Some(pe) = &mut self.pending {
                        pe.changed = true;
                    }
                }
            }
        }
        if resp.drag_stopped() {
            self.gestures.features = None;
            self.snap_guides.clear();
            self.end_pending();
            self.set_status(crate::i18n::tr("已移动（Ctrl+Z 撤销）"), false);
        }
        true
    }

    /// Outline what is selected, and the selection box while it is drawn.
    pub fn draw_selection(&self, painter: &egui::Painter, vg: &ViewGeom) {
        let i = vg.kind.index();
        let stroke = Stroke::new(1.5, ACCENT);
        for f in self.selection() {
            match f {
                FeatureRef::Symbol(id) => {
                    if let Some((_, r)) = self.sym_rects.borrow()[i].iter().find(|(x, _)| *x == id) {
                        painter.rect_stroke(r.expand(2.0), 2.0, stroke, egui::StrokeKind::Outside);
                    }
                }
                FeatureRef::Text(id) => {
                    if let Some((_, r)) = self.ann_rects.borrow()[i].iter().find(|(x, _)| *x == id) {
                        painter.rect_stroke(r.expand(2.0), 2.0, stroke, egui::StrokeKind::Outside);
                    }
                }
                // a single line or area is already shown as selected by its own drawing
                _ if self.picked.len() < 2 => {}
                _ => {
                    let pts = self.doc.feature_points(f);
                    for w in pts.windows(2) {
                        if let (Some(a), Some(b)) = (vg.to_screen(w[0]), vg.to_screen(w[1])) {
                            // not across the seam of the flat map
                            if (a - b).length() < vg.rect.width() * 0.5 {
                                painter.line_segment([a, b], Stroke::new(3.0, ACCENT.gamma_multiply(0.6)));
                            }
                        }
                    }
                }
            }
        }
        // smart-snapping guides while moving
        let guide = Stroke::new(1.2, Color32::from_rgb(230, 60, 160));
        let line = |pts: Vec<LonLat>| {
            let s: Vec<Pos2> = pts.into_iter().filter_map(|p| vg.to_screen(p)).collect();
            for w in s.windows(2) {
                if (w[1] - w[0]).length() < vg.rect.width() * 0.5 {
                    painter.line_segment([w[0], w[1]], guide);
                }
            }
        };
        for g in &self.snap_guides {
            match *g {
                map_core::snap::Guide::Point(t) => {
                    if let Some(s) = vg.to_screen(t) {
                        painter.circle_stroke(s, 6.0, Stroke::new(1.8, guide.color));
                    }
                }
                map_core::snap::Guide::Parallel(lat, from) => {
                    line((0..=180).map(|i| LonLat::new(-180.0 + i as f64 * 2.0, lat)).collect());
                    if let Some(s) = from.and_then(|t| vg.to_screen(t)) {
                        painter.circle_filled(s, 3.0, guide.color);
                    }
                }
                map_core::snap::Guide::Meridian(lon, from) => {
                    line((0..=89).map(|i| LonLat::new(lon, -89.0 + i as f64 * 2.0)).collect());
                    if let Some(s) = from.and_then(|t| vg.to_screen(t)) {
                        painter.circle_filled(s, 3.0, guide.color);
                    }
                }
            }
        }
        if let Some((_, a, b)) = self.gestures.marquee.filter(|m| m.0 == i) {
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(r, 0.0, ACCENT.gamma_multiply(0.12));
            painter.rect_stroke(r, 0.0, Stroke::new(1.0, ACCENT), egui::StrokeKind::Inside);
        }
    }

    // ------------------------------------------------------- what it does

    pub fn delete_selection(&mut self) {
        let fs = self.selection();
        if fs.is_empty() {
            return;
        }
        let n = fs.len();
        self.edit(if n == 1 { "删除要素" } else { "删除多个要素" }, |d| {
            d.delete_features(&fs);
            Ok(())
        });
        self.clear_selection();
        self.selected_sym = None;
        self.selected_ann = None;
        self.set_status(crate::trf!("已删除 {n} 个要素（Ctrl+Z 撤销）"), false);
    }

    /// Select the features an edit made (pasted, duplicated): in a tool
    /// that edits one of them, that tool's selection too.
    fn select_new(&mut self, new: Vec<FeatureRef>) {
        self.set_selection(new.clone());
        if let [one] = new[..] {
            match one {
                FeatureRef::Symbol(id) if self.tool == Tool::Symbol => self.selected_sym = Some(id),
                FeatureRef::Text(id) if self.tool == Tool::Label => self.selected_ann = Some(id),
                _ => {}
            }
        }
    }

    /// Ctrl+D: copies of the selection a little down and to the right.
    pub fn duplicate_selection(&mut self) {
        let fs = self.selection();
        if fs.is_empty() {
            self.set_status(crate::i18n::tr("先选中要复制的线、区域、符号或文字"), true);
            return;
        }
        // about 24 px at the current 2D zoom
        let d = 24.0 * 57.29578 / self.cam2d.zoom.max(1e-9);
        if let Some(new) = self.edit(if fs.len() == 1 { "再制要素" } else { "再制多个要素" }, |doc| doc.duplicate_features(&fs, d, -d)) {
            let n = new.len();
            self.select_new(new);
            self.set_status(crate::trf!("已再制 {n} 个要素：拖动可移动"), false);
        }
    }

    /// Copy (or cut) the selection to the clipboard.
    pub fn copy_selection(&mut self, ctx: &egui::Context, cut: bool) {
        let fs = self.selection();
        if fs.is_empty() {
            return;
        }
        let clip = self.doc.copy_features(&fs);
        ctx.copy_text(map_io::clip::to_text(&clip));
        self.clip = Some(clip);
        let n = fs.len();
        if cut {
            self.edit("剪切要素", |d| {
                d.delete_features(&fs);
                Ok(())
            });
            self.clear_selection();
            self.selected_sym = None;
            self.selected_ann = None;
        }
        self.set_status(crate::trf!("已{} {n} 个要素（Ctrl+V 粘贴，也可粘贴到另一个窗口）", if cut { "剪切" } else { "复制" }), false);
    }

    /// Paste features: under the pointer when it is over the map, else a
    /// little beside where they were copied from.
    pub fn paste(&mut self, clip: Clip) {
        if clip.items.is_empty() {
            return;
        }
        let at = self.cursor;
        let d = 24.0 * 57.29578 / self.cam2d.zoom.max(1e-9);
        let made = self.edit(if clip.items.len() == 1 { "粘贴要素" } else { "粘贴多个要素" }, |doc| {
            let new = doc.paste_features(&clip, at)?;
            if at.is_none() {
                if let Some(c) = doc.features_centre(&new) {
                    doc.move_features(&new, c, LonLat::new(map_core::geom::wrap_lon(c.lon + d), (c.lat - d).clamp(-89.0, 89.0)))?;
                }
            }
            Ok(new)
        });
        if let Some(new) = made {
            let n = new.len();
            self.select_new(new);
            self.set_status(crate::trf!("已粘贴 {n} 个要素"), false);
        }
    }

    /// Ctrl+C, Ctrl+X and Ctrl+V arrive as clipboard events, not as keys.
    pub fn clipboard_events(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() || self.palette.is_some() {
            return;
        }
        let events = ctx.input(|i| i.events.iter().filter(|e| matches!(e, egui::Event::Copy | egui::Event::Cut | egui::Event::Paste(_))).cloned().collect::<Vec<_>>());
        for e in events {
            match e {
                egui::Event::Copy => self.copy_selection(ctx, false),
                egui::Event::Cut => self.copy_selection(ctx, true),
                egui::Event::Paste(text) => match map_io::clip::from_text(&text) {
                    Some(clip) => self.paste(clip),
                    None => self.set_status(crate::i18n::tr("剪贴板里不是地图要素"), true),
                },
                _ => {}
            }
        }
    }

    pub fn hide_selection(&mut self) {
        let fs = self.selection();
        let n = fs.len();
        if self.edit("隐藏要素", |d| fs.iter().try_for_each(|f| d.set_hidden(*f, true))).is_some() {
            self.clear_selection();
            self.set_status(crate::trf!("已隐藏 {n} 个要素（在“要素”标签中可重新显示）"), false);
        }
    }

    pub fn lock_selection(&mut self) {
        let fs = self.selection();
        let n = fs.len();
        if self.edit("锁定要素", |d| fs.iter().try_for_each(|f| d.set_locked(*f, true))).is_some() {
            self.clear_selection();
            self.set_status(crate::trf!("已锁定 {n} 个要素：不会再被选中或移动（“对象”菜单或“要素”标签中解锁）"), false);
        }
    }

    pub fn unlock_all(&mut self) {
        let n = self.doc.locked.len();
        self.edit("全部解锁", |d| {
            d.locked.clear();
            Ok(())
        });
        self.set_status(crate::trf!("已解锁 {n} 个要素"), false);
    }

    pub fn select_all_features(&mut self) {
        let all: Vec<FeatureRef> = self.doc.features().into_iter().map(|f| f.what).filter(|f| self.doc.pickable(*f)).collect();
        let n = all.len();
        self.set_selection(all);
        self.set_status(crate::trf!("已选择全部 {n} 个要素"), false);
    }

    pub fn align_selection(&mut self, how: Align) {
        let fs = self.selection();
        self.edit("对齐要素", |d| d.align_features(&fs, how));
    }

    pub fn distribute_selection(&mut self, by_lon: bool) {
        let fs = self.selection();
        self.edit("均匀分布要素", |d| d.distribute_features(&fs, by_lon));
    }

    /// The panel for the selection: what is selected, and turning,
    /// scaling, lining up and spacing it.
    pub fn selection_panel(&mut self, ui: &mut egui::Ui) {
        let fs = self.selection();
        // one line or area has its own panel; the group panel is for more,
        // and for symbols and text picked with the select tool
        if fs.is_empty() || (fs.len() == 1 && self.tool != Tool::Select) {
            return;
        }
        let mut kinds: Vec<(&str, usize)> = Vec::new();
        for f in &fs {
            match kinds.iter_mut().find(|k| k.0 == f.kind_name()) {
                Some(k) => k.1 += 1,
                None => kinds.push((f.kind_name(), 1)),
            }
        }
        ui.label(RichText::new(crate::trf!("已选择 {} 个要素", fs.len())).strong());
        ui.weak(kinds.iter().map(|(k, n)| format!("{k} {n}")).collect::<Vec<_>>().join("，"));
        ui.horizontal_wrapped(|ui| {
            let ctx = ui.ctx().clone();
            if ui.button(crate::i18n::tr("复制")).on_hover_text(crate::i18n::tr("Ctrl+C；可粘贴到另一个窗口")).clicked() {
                self.copy_selection(&ctx, false);
            }
            if ui.button(crate::i18n::tr("原处再制")).on_hover_text("Ctrl+D").clicked() {
                self.duplicate_selection();
            }
            if ui.button(crate::i18n::tr("隐藏")).clicked() {
                self.hide_selection();
            }
            if ui.button(crate::i18n::tr("锁定")).on_hover_text(crate::i18n::tr("锁定后不会被选中或移动")).clicked() {
                self.lock_selection();
            }
            if ui.button(crate::i18n::tr("删除")).on_hover_text("Delete").clicked() {
                self.delete_selection();
            }
        });
        ui.push_id("styles", |ui| self.style_row(ui, &fs));
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("旋转"));
            ui.add(egui::DragValue::new(&mut self.xf_rotate).speed(1.0).range(-360.0..=360.0).suffix("°"));
            if ui.button(crate::i18n::tr("应用")).on_hover_text(crate::i18n::tr("绕所选要素的中心逆时针旋转（负值为顺时针）；直排的文字随之转向")).clicked() {
                let deg = self.xf_rotate;
                self.edit("旋转要素", |d| {
                    let c = d.features_centre(&fs).ok_or("没有可旋转的要素")?;
                    d.rotate_features(&fs, c, deg)
                });
            }
            ui.separator();
            ui.label(crate::i18n::tr("缩放"));
            ui.add(egui::DragValue::new(&mut self.xf_scale).speed(1.0).range(1.0..=1000.0).suffix("%"));
            if ui.button(crate::i18n::tr("应用 ")).on_hover_text(crate::i18n::tr("以所选要素的中心为基准放大或缩小形状与间距（文字与符号本身的大小不变）")).clicked() {
                let k = self.xf_scale / 100.0;
                self.edit("缩放要素", |d| {
                    let c = d.features_centre(&fs).ok_or("没有可缩放的要素")?;
                    d.scale_features(&fs, c, k)
                });
            }
        });
        if fs.len() >= 2 {
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::tr("对齐"));
                for (how, name, hint) in [
                    (Align::West, "西", "各要素的中心对齐到最西的经度"),
                    (Align::MidLon, "经向居中", "对齐到中间的经度"),
                    (Align::East, "东", "对齐到最东的经度"),
                    (Align::North, "北", "对齐到最北的纬度"),
                    (Align::MidLat, "纬向居中", "对齐到中间的纬度"),
                    (Align::South, "南", "对齐到最南的纬度"),
                ] {
                    if ui.small_button(name).on_hover_text(hint).clicked() {
                        self.align_selection(how);
                    }
                }
            });
        }
        if fs.len() >= 3 {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("均匀分布"));
                if ui.small_button(crate::i18n::tr("沿经度")).on_hover_text(crate::i18n::tr("最西与最东的不动，其余按经度等距排开")).clicked() {
                    self.distribute_selection(true);
                }
                if ui.small_button(crate::i18n::tr("沿纬度")).clicked() {
                    self.distribute_selection(false);
                }
            });
        }
        ui.separator();
    }
}
