//! Hatched areas on screen: drawn with the hatch tool (like land: points,
//! freehand, snapping and tracing along borders), selected by clicking
//! their outline, edited in the panel. They never change territory.

use crate::app::App;
use crate::view::{Tool, ViewGeom};
use eframe::egui::{self, Color32, Pos2, RichText, Shape, Stroke, Vec2};
use map_core::area::{centre, fill_pattern_in, group_rings, Pattern, DOT_RADIUS, HATCH_WIDTH, NAME_PX, OUTLINE_WIDTH};
use map_core::geom::LonLat;

fn c32(c: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

impl App {
    /// Every hatched area shown in the current year (all of them, the
    /// others faint, with the hatch tool), under routes, symbols and text.
    /// `taken` collects the names' boxes so city names keep clear of them.
    pub fn draw_areas(&self, painter: &egui::Painter, vg: &ViewGeom, taken: &mut crate::label_boxes::LabelBoxes, pass: map_core::stack::DrawPass) {
        let editing = self.tool == Tool::Hatch;
        for (id, a) in self.doc.areas.iter().filter(|(id, _)| self.doc.group_of_id(**id) == pass.group) {
            let shown = a.shown_in(self.doc.year);
            if !shown && !editing {
                continue;
            }
            let fade = if shown { 1.0 } else { 0.35 };
            let col = c32(a.color).gamma_multiply(fade);
            let selected = self.selected_area == Some(*id);
            // every ring on screen, closed (the outline first, then pieces and holes)
            let screens: Vec<Vec<Option<Pos2>>> = a
                .rings()
                .into_iter()
                .map(|r| {
                    let mut closed = r.clone();
                    closed.push(r[0]);
                    crate::river_ui::densify_ll(&closed).into_iter().map(|q| vg.to_screen(q)).collect()
                })
                .collect();
            if screens.is_empty() {
                continue;
            }
            let whole = screens.iter().all(|s| s.iter().all(|p| p.is_some()) && s.windows(2).all(|w| (w[0].unwrap().x - w[1].unwrap().x).abs() < vg.rect.width() * 0.5));
            if whole {
                // the whole shape is on screen: wash, stripes and name
                let rings: Vec<Vec<[f32; 2]>> = screens.iter().map(|s| s[..s.len() - 1].iter().map(|p| p.map_or([0.0, 0.0], |p| [p.x, p.y])).collect()).collect();
                let ring = &rings[0];
                if a.fills() {
                    for (outer, holes) in group_rings(&rings) {
                        let mut flat: Vec<f64> = rings[outer].iter().flat_map(|p| [p[0] as f64, p[1] as f64]).collect();
                        let mut starts = Vec::new();
                        for h in &holes {
                            starts.push(flat.len() / 2);
                            flat.extend(rings[*h].iter().flat_map(|p| [p[0] as f64, p[1] as f64]));
                        }
                        if let Ok(tri) = earcutr::earcut(&flat, &starts, 2) {
                            let mut mesh = egui::Mesh::default();
                            for p in flat.chunks_exact(2) {
                                mesh.colored_vertex(egui::pos2(p[0] as f32, p[1] as f32), c32(a.tint_color()).gamma_multiply(fade));
                            }
                            mesh.indices = tri.into_iter().map(|i| i as u32).collect();
                            painter.add(Shape::mesh(mesh));
                        }
                    }
                }
                // the name at the middle, with the stripes cut back around it
                let name = (!a.name.is_empty()).then(|| centre(ring)).flatten().map(|c| {
                    let galley = painter.layout_job(italic_job(&a.name, egui::FontId::proportional(NAME_PX), col));
                    let tl = egui::pos2(c[0], c[1]) - galley.size() * 0.5;
                    (tl, galley)
                });
                let gap = name.as_ref().map(|(tl, g)| [tl.x - 4.0, tl.y - 2.0, tl.x + g.size().x + 4.0, tl.y + g.size().y + 2.0]);
                let clip = painter.clip_rect().expand(a.spacing * 0.7 + 4.0);
                let (stripes, dots) = fill_pattern_in(&rings, a.pattern, a.spacing, a.angle, gap,
                    Some([clip.left(), clip.top(), clip.right(), clip.bottom()]));
                for [p, q] in stripes {
                    painter.line_segment([egui::pos2(p[0], p[1]), egui::pos2(q[0], q[1])], Stroke::new(HATCH_WIDTH, col));
                }
                for p in dots {
                    painter.circle_filled(egui::pos2(p[0], p[1]), DOT_RADIUS, col);
                }
                if let Some((tl, galley)) = name {
                    taken.push(egui::Rect::from_min_size(tl, galley.size()).expand(2.0));
                    let halo = painter.layout_job(italic_job(&a.name, egui::FontId::proportional(NAME_PX), Color32::from_rgba_unmultiplied(255, 255, 255, 200)));
                    for o in [Vec2::new(-1.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, -1.0), Vec2::new(0.0, 1.0)] {
                        painter.galley(tl + o, halo.clone(), Color32::WHITE);
                    }
                    painter.galley(tl, galley, Color32::WHITE);
                }
            }
            if a.pattern == Pattern::Solid && !selected && !editing { continue; }
            // the outlines, broken where they leave the view or wrap
            let mut pieces: Vec<Vec<Pos2>> = vec![Vec::new()];
            for screen in &screens {
                pieces.push(Vec::new());
                for (i, s) in screen.iter().enumerate() {
                    match s {
                        Some(p) if i > 0 && screen[i - 1].is_some_and(|q| (q.x - p.x).abs() < vg.rect.width() * 0.5) => pieces.last_mut().unwrap().push(*p),
                        Some(p) => pieces.push(vec![*p]),
                        None => pieces.push(Vec::new()),
                    }
                }
            }
            for piece in pieces.into_iter().filter(|p| p.len() >= 2) {
                if selected {
                    painter.add(Shape::line(piece.clone(), Stroke::new(6.0, Color32::from_rgba_unmultiplied(255, 170, 0, 120))));
                }
                let stroke = Stroke::new(OUTLINE_WIDTH, col);
                if a.dashed {
                    crate::screen_path::dashed(painter, &piece, stroke, 7.0, 4.0);
                } else {
                    painter.add(Shape::line(piece, stroke));
                }
            }
        }
    }

    /// Vertex tool on hatched areas: drag a point, drag anywhere on the
    /// outline to insert one, right-click to delete one. An area wins over
    /// a border only when it is at least as close (a river is tried first).
    /// Returns whether it took the input.
    pub fn area_edit_input(&mut self, resp: &egui::Response, vg: &ViewGeom, p: LonLat, tol: f64) -> bool {
        use egui::PointerButton::{Primary, Secondary};
        use map_core::geom::geo_dist_deg;
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let q = origin.and_then(|o| vg.to_lonlat(o)).unwrap_or(p);
            let border_pt = self.doc.topo.nearest_vertex(q, tol).map(|(_, at)| geo_dist_deg(q, at));
            let border_line = self.doc.topo.nearest_on_edge(q, tol).map(|(_, _, at)| geo_dist_deg(q, at));
            if let Some((id, i, d)) = self.doc.area_point_at(q, tol).filter(|h| !self.doc.curves.contains_key(&h.0)) {
                if border_pt.is_none_or(|b| d <= b) {
                    self.begin_pending("移动区域点");
                    self.drag_area_pt = Some((id, i));
                    self.selected_area = Some(id);
                    return true;
                }
            } else if border_pt.is_none() {
                if let Some((id, side, at)) = self.doc.area_on_edge(q, tol).filter(|h| !self.doc.curves.contains_key(&h.0)) {
                    if border_line.is_none_or(|b| geo_dist_deg(q, at) <= b) {
                        self.begin_pending("插入区域点");
                        if let Ok(i) = self.doc.insert_area_point(id, side, at) {
                            if let Some(pe) = &mut self.pending {
                                pe.changed = true;
                            }
                            self.drag_area_pt = Some((id, i));
                            self.selected_area = Some(id);
                        }
                        return true;
                    }
                }
            }
        }
        if let Some((id, i)) = self.drag_area_pt {
            if resp.dragged_by(Primary) && self.doc.move_area_point(id, i, p).is_ok() {
                if let Some(pe) = &mut self.pending {
                    pe.changed = true;
                }
            }
            if resp.drag_stopped() {
                self.drag_area_pt = None;
                self.end_pending();
            }
            return true;
        }
        if resp.clicked_by(Secondary) {
            let border_pt = self.doc.topo.nearest_vertex(p, tol).map(|(_, at)| geo_dist_deg(p, at));
            if let Some((id, i, d)) = self.doc.area_point_at(p, tol).filter(|h| !self.doc.curves.contains_key(&h.0)) {
                if border_pt.is_none_or(|b| d <= b) {
                    self.edit("删除区域点", |doc| doc.delete_area_point(id, i));
                    return true;
                }
            }
        }
        false
    }

    /// With the vertex tool, the outline points of areas near the cursor.
    pub fn draw_area_handles(&self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        if self.tool != Tool::EditVertex {
            return;
        }
        let Some(h) = hover else { return };
        let Some(hs) = vg.to_screen(h) else { return };
        let radius = vg.deg_per_px(hs, 160.0);
        // (curves show their anchors instead, see `curve_ui`)
        for (id, a) in self.doc.areas.iter().filter(|(id, _)| !self.doc.curves.contains_key(*id)) {
            let col = c32(a.color);
            for (i, q) in a.ring.iter().enumerate() {
                if map_core::geom::geo_dist_deg(*q, h) > radius {
                    continue;
                }
                if let Some(s) = vg.to_screen(*q) {
                    let r = if self.drag_area_pt == Some((*id, i)) { 5.0 } else { 3.5 };
                    painter.rect_filled(egui::Rect::from_center_size(s, Vec2::splat(r * 2.0)), 1.0, col);
                    painter.rect_stroke(egui::Rect::from_center_size(s, Vec2::splat(r * 2.0)), 1.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Outside);
                }
            }
        }
    }

    /// A click on a hatched area's outline selects it. Returns whether it hit one.
    pub fn area_select_click(&mut self, p: LonLat, tol: f64) -> bool {
        match self.doc.area_edge_at(p, tol) {
            Some(id) => {
                self.selected_area = Some(id);
                self.selected = None;
                self.selected_face = None;
                self.selected_river = None;
                self.tab = crate::panels::Tab::Countries;
                self.set_status(crate::i18n::tr("已选择斜线区域（Delete 删除）"), false);
                true
            }
            None => false,
        }
    }

    pub fn delete_selected_area(&mut self) {
        if let Some(id) = self.selected_area.take() {
            if self.edit("删除斜线区域", |d| (d.delete_features(&[map_core::features::FeatureRef::Area(id)]) > 0).then_some(()).ok_or_else(|| "区域不存在".to_string())).is_some() {
                self.set_status(crate::i18n::tr("已删除斜线区域（Ctrl+Z 撤销）"), false);
            }
        }
    }

    /// Side panel for the selected hatched area.
    pub fn area_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_area.filter(|id| self.doc.areas.contains_key(id)) else {
            self.selected_area = None;
            return;
        };
        ui.separator();
        ui.label(RichText::new(crate::i18n::tr("所选斜线区域")).strong());
        self.curve_panel(ui, id);
        self.style_row(ui, &[map_core::features::FeatureRef::Area(id)]);
        let old = self.doc.areas[&id].clone();
        let mut v = old.clone();
        if self.area_name.as_ref().is_none_or(|(x, _)| *x != id) {
            self.area_name = Some((id, old.name.clone()));
        }
        let buf = &mut self.area_name.as_mut().unwrap().1;
        let r = ui.add(egui::TextEdit::singleline(buf).hint_text(crate::i18n::tr("名称（可留空）")));
        if r.lost_focus() && *buf != old.name {
            v.name = buf.trim().to_string();
        }
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("图案"));
            for p in Pattern::ALL {
                ui.radio_value(&mut v.pattern, p, p.name());
            }
        });
        let rs = ui.add(egui::Slider::new(&mut v.spacing, 3.0..=40.0).suffix(" px").text(crate::i18n::tr("间距")));
        let ra = ui.add(egui::Slider::new(&mut v.angle, -90.0..=90.0).suffix("°").text(crate::i18n::tr("角度")));
        ui.horizontal(|ui| {
            let mut c = c32(v.color);
            if ui.color_edit_button_srgba(&mut c).changed() {
                v.color = c.to_srgba_unmultiplied();
            }
            ui.label(crate::i18n::tr("颜色"));
            ui.checkbox(&mut v.tint, crate::i18n::tr("底色"));
            ui.checkbox(&mut v.dashed, crate::i18n::tr("虚线边框"));
        });
        if let Some(t) = legend_field(ui, &mut self.area_legend, id, &old.legend) {
            v.legend = t;
        }
        let mut drags = vec![rs, ra];
        drags.extend(crate::symbol_ui::years_ui(ui, &mut v.from, &mut v.to, self.doc.year));
        ui.horizontal(|ui| {
            if ui.small_button(crate::i18n::tr("删除区域")).clicked() {
                self.selected_area = Some(id);
                self.delete_selected_area();
            }
        });
        // combining with another area: union, cut out, overlap
        let others: Vec<(map_core::doc::LineId, String)> = self
            .doc
            .areas
            .iter()
            .filter(|(o, _)| **o != id)
            .enumerate()
            .map(|(i, (o, a))| (*o, if a.name.is_empty() { format!("（未命名 {}）", i + 1) } else { a.name.clone() }))
            .collect();
        if !others.is_empty() {
            if self.area_partner.is_none_or(|p| !others.iter().any(|o| o.0 == p)) {
                self.area_partner = Some(others[0].0);
            }
            let mut op = None;
            ui.horizontal_wrapped(|ui| {
                ui.label(crate::i18n::tr("与区域"));
                let current = others.iter().find(|o| Some(o.0) == self.area_partner).map_or(String::new(), |o| o.1.clone());
                egui::ComboBox::from_id_salt("area_partner").selected_text(current).show_ui(ui, |ui| {
                    for (o, name) in &others {
                        ui.selectable_value(&mut self.area_partner, Some(*o), name);
                    }
                });
                for k in [map_core::area::AreaOp::Union, map_core::area::AreaOp::Difference, map_core::area::AreaOp::Intersection] {
                    if ui.small_button(k.name()).on_hover_text(crate::i18n::tr("结果保留在本区域（名称、样式、年份不变），另一个区域被用掉；可撤销")).clicked() {
                        op = Some(k);
                    }
                }
            });
            if let (Some(k), Some(other)) = (op, self.area_partner) {
                if self.edit(&format!("区域{}", k.name()), |d| d.combine_areas(id, other, k)).is_some() {
                    self.area_partner = None;
                    self.set_status(crate::trf!("已{}区域（Ctrl+Z 撤销）", k.name()), false);
                }
            }
        }
        ui.weak(crate::i18n::tr("斜线区域只是标记（争议、占领、势力范围等），不改变领土归属。编辑顶点工具（E）：拖动边线上的点，拖动边线任意处插入点，右键删除点。选择工具下拖动边线可整体移动，Ctrl+D 复制。"));
        if drags.iter().any(|r| r.drag_started()) {
            self.begin_pending("调整斜线区域");
        }
        if v != old {
            if self.pending.is_some() {
                self.doc.areas.insert(id, v);
                if let Some(pe) = &mut self.pending {
                    pe.changed = true;
                }
            } else {
                self.edit("修改斜线区域", |d| {
                    d.areas.get(&id).ok_or("区域不存在")?;
                    d.areas.insert(id, v);
                    Ok(())
                });
            }
        }
        if drags.iter().any(|r| r.drag_stopped()) {
            self.end_pending();
        }
    }
}

/// The "图例说明" field of a route or area: returns the new text when an
/// edit is committed (focus leaves the field).
pub fn legend_field(ui: &mut egui::Ui, buf: &mut Option<(map_core::doc::LineId, String)>, id: map_core::doc::LineId, current: &str) -> Option<String> {
    if buf.as_ref().is_none_or(|(x, _)| *x != id) {
        *buf = Some((id, current.to_string()));
    }
    let text = &mut buf.as_mut().unwrap().1;
    let r = ui
        .horizontal(|ui| {
            ui.label(crate::i18n::tr("图例说明"));
            ui.add(egui::TextEdit::singleline(text).hint_text(crate::i18n::tr("留空则不进图例")))
        })
        .inner
        .on_hover_text(crate::i18n::tr("导出时在图例中显示这一项；说明与样式都相同的会合并为一行"));
    (r.lost_focus() && text.trim() != current).then(|| text.trim().to_string())
}

fn italic_job(text: &str, font: egui::FontId, color: Color32) -> egui::text::LayoutJob {
    let mut j = egui::text::LayoutJob::default();
    j.append(text, 0.0, egui::TextFormat { font_id: font, color, italics: true, ..Default::default() });
    j
}
