//! Rivers and routes after they are drawn: selecting one, renaming,
//! reshaping and deleting it, and editing its points with the vertex tool.
//! Both are layout, the same in every year, so these edit the line
//! directly (undoable like any edit). Routes are drawn here, styled.

/// Colour, width, dash and arrow of a route. Returns (changed, drag
/// started, drag stopped) so a slider drag is one undo step.
pub fn route_style_ui(ui: &mut egui::Ui, st: &mut map_core::doc::RouteStyle) -> (bool, bool, bool) {
    let mut changed = false;
    if ui.checkbox(&mut st.broad, crate::i18n::tr("粗箭头")).on_hover_text(crate::i18n::tr("画成由窄渐宽、末端带大箭头的色带（进军、迁徙方向）；宽度为色带宽度")).changed() {
        changed = true;
        if st.broad && st.width < 6.0 {
            st.width = 14.0;
        } else if !st.broad && st.width > 12.0 {
            st.width = 2.5;
        }
    }
    if st.broad {
        changed |= ui.checkbox(&mut st.hollow, crate::i18n::tr("空心")).on_hover_text(crate::i18n::tr("只画轮廓（撤退、计划中的行动等）；轮廓使用下面的线型")).changed();
    }
    if !st.broad || st.hollow {
        ui.horizontal(|ui| {
            for d in map_core::doc::Dash::ALL {
                changed |= ui.radio_value(&mut st.dash, d, d.name()).changed();
            }
        });
    }
    if !st.broad {
        ui.horizontal_wrapped(|ui| {
            ui.label(crate::i18n::tr("装饰"));
            for d in map_core::deco::Deco::ALL {
                changed |= ui.radio_value(&mut st.deco, d, d.name()).changed();
            }
            if st.deco == map_core::deco::Deco::Teeth {
                changed |= ui.checkbox(&mut st.deco_flip, crate::i18n::tr("翻转")).on_hover_text(crate::i18n::tr("锯齿默认在画线方向的左侧；勾选后在右侧")).changed();
            }
        });
    }
    let range = if st.broad { 4.0..=60.0 } else { 0.5..=12.0 };
    let rw = ui.add(egui::Slider::new(&mut st.width, range).suffix(" px").text(if st.broad { crate::i18n::tr("宽度") } else { crate::i18n::tr("线宽") }));
    changed |= rw.changed();
    ui.horizontal(|ui| {
        let mut c = Color32::from_rgba_unmultiplied(st.color[0], st.color[1], st.color[2], st.color[3]);
        if ui.color_edit_button_srgba(&mut c).changed() {
            st.color = c.to_array();
            changed = true;
        }
        ui.label(crate::i18n::tr("颜色"));
        changed |= ui.checkbox(&mut st.arrow, crate::i18n::tr("终点箭头")).changed();
    });
    (changed, rw.drag_started(), rw.drag_stopped())
}

use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui::{self, Color32, RichText, Stroke};
use map_core::geom::{self, LonLat};

impl App {
    /// Select tool: a click on a city's point selects it.
    pub fn city_select_click(&mut self, p: LonLat, tol: f64, detail: crate::map_detail::Detail) -> bool {
        let Some(id) = self.doc.city_at_where(p, tol, |id| self.city_shown(id, detail)) else { return false };
        self.selected = Some(id);
        self.selected_face = None;
        self.selected_river = None;
        self.tab = crate::panels::Tab::Countries;
        self.set_status(crate::trf!("已选择城市 {}", self.doc.entities[&id].name), false);
        true
    }

    /// City tool on an existing city: click selects, drag moves, right
    /// click deletes. Returns whether it took the input.
    pub fn city_input(&mut self, resp: &egui::Response, vg: &ViewGeom, p: LonLat, tol: f64) -> bool {
        use egui::PointerButton::{Primary, Secondary};
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let q = origin.and_then(|o| vg.to_lonlat(o)).unwrap_or(p);
            if let Some(id) = self.doc.city_at(q, tol) {
                self.begin_pending("移动城市");
                self.drag_city = Some(id);
                self.selected = Some(id);
            }
        }
        if let Some(id) = self.drag_city {
            if resp.dragged_by(Primary) {
                if let Some(e) = self.doc.entities.get_mut(&id) {
                    e.pos = Some(p.normalized());
                    if let Some(pe) = &mut self.pending {
                        pe.changed = true;
                    }
                }
            }
            if resp.drag_stopped() {
                self.drag_city = None;
                let name = |d: &map_core::doc::Document, e: Option<map_core::EntityId>| e.and_then(|e| d.entities.get(&e)).map_or("无归属".to_string(), |x| x.name.clone());
                match self.doc.move_city(id, p) {
                    Ok(map_core::doc::CityMove::Same) => self.set_status(crate::i18n::tr("已移动城市"), false),
                    Ok(map_core::doc::CityMove::Parent(to)) => self.set_status(crate::trf!("已移动城市：现属于 {}", name(&self.doc, to)), false),
                    Ok(map_core::doc::CityMove::Kept { here }) => {
                        self.set_status(crate::trf!("已移动城市：新位置在 {} 境内；因为选定了年份或城市归属有年份记录，归属未自动修改（可在城市属性中改为所在国家）", name(&self.doc, here)), false)
                    }
                    Err(e) => self.set_status(crate::trf!("移动城市失败：{e}"), true),
                }
                self.end_pending();
            }
            return true;
        }
        if let Some(id) = self.doc.city_at(p, tol) {
            if resp.clicked() {
                self.selected = Some(id);
                self.selected_river = None;
                self.tab = crate::panels::Tab::Countries;
                return true;
            }
            if resp.clicked_by(Secondary) {
                let name = self.doc.entities[&id].name.clone();
                if self.edit("删除城市", |d| {
                    d.delete_entity(id);
                    Ok(())
                }).is_some() {
                    self.selected = self.selected.filter(|s| *s != id);
                    self.set_status(crate::trf!("已删除城市 {name}（Ctrl+Z 撤销）"), false);
                }
                return true;
            }
        }
        false
    }

    /// Vertex tool on rivers: drag a point, drag anywhere on the line to
    /// insert one, right-click to delete one. A river wins over a border
    /// only when it is at least as close. Returns whether it took the input.
    pub fn river_edit_input(&mut self, resp: &egui::Response, vg: &ViewGeom, p: LonLat, tol: f64) -> bool {
        use egui::PointerButton::{Primary, Secondary};
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let q = origin.and_then(|o| vg.to_lonlat(o)).unwrap_or(p);
            let border_pt = self.doc.topo.nearest_vertex(q, tol).map(|(_, at)| geom::geo_dist_deg(q, at));
            let border_line = self.doc.topo.nearest_on_edge(q, tol).map(|(_, _, at)| geom::geo_dist_deg(q, at));
            if let Some((id, i, d)) = self.doc.river_point_at(q, tol).filter(|h| !self.doc.curves.contains_key(&h.0)) {
                if border_pt.is_none_or(|b| d <= b) {
                    self.begin_pending("移动河流点");
                    self.drag_river = Some((id, i));
                    return true;
                }
            } else if border_pt.is_none() {
                if let Some((id, seg, at)) = self.doc.river_at(q, tol).filter(|h| !self.doc.curves.contains_key(&h.0)) {
                    if border_line.is_none_or(|b| geom::geo_dist_deg(q, at) <= b) {
                        self.begin_pending("插入河流点");
                        if let Some(r) = self.doc.rivers.get_mut(&id) {
                            r.pts.insert(seg + 1, at);
                        }
                        if let Some(pe) = &mut self.pending {
                            pe.changed = true;
                        }
                        self.drag_river = Some((id, seg + 1));
                        return true;
                    }
                }
            }
        }
        if let Some((id, i)) = self.drag_river {
            if resp.dragged_by(Primary) {
                if let Some(pt) = self.doc.rivers.get_mut(&id).and_then(|r| r.pts.get_mut(i)) {
                    *pt = p;
                    if let Some(pe) = &mut self.pending {
                        pe.changed = true;
                    }
                }
            }
            if resp.drag_stopped() {
                self.drag_river = None;
                self.end_pending();
            }
            return true;
        }
        if resp.clicked_by(Secondary) {
            let border_pt = self.doc.topo.nearest_vertex(p, tol).map(|(_, at)| geom::geo_dist_deg(p, at));
            if let Some((id, i, d)) = self.doc.river_point_at(p, tol).filter(|h| !self.doc.curves.contains_key(&h.0)) {
                if border_pt.is_none_or(|b| d <= b) {
                    self.edit("删除河流点", |doc| doc.delete_river_point(id, i));
                    return true;
                }
            }
        }
        false
    }

    pub fn delete_selected_river(&mut self) {
        if let Some(id) = self.selected_river.take() {
            let Some((name, kind)) = self.doc.rivers.get(&id).map(|r| (r.name.clone(), r.kind_name())) else { return };
            if self.edit(&format!("删除{kind}"), |d| {
                use map_core::features::FeatureRef;
                let route = d.rivers.get(&id).is_some_and(|line| line.route.is_some());
                let item = if route { FeatureRef::Route(id) } else { FeatureRef::River(id) };
                (d.delete_features(&[item]) > 0).then_some(()).ok_or_else(|| "线不存在".to_string())
            }).is_some() {
                self.set_status(crate::trf!("已删除{kind} {name}（Ctrl+Z 撤销）"), false);
            }
        }
    }

    /// Names written along their rivers and routes (where switched on and
    /// the line is long enough on screen); `taken` gets their glyphs' boxes.
    pub fn draw_line_names(&self, painter: &egui::Painter, vg: &ViewGeom, taken: &mut crate::label_boxes::LabelBoxes) {
        use map_core::path_text::{along_path, NAME_PX};
        let detail = self.map_detail(vg);
        for (id, r) in self.doc.rivers.iter().filter(|(id, r)| r.show_name && !r.name.is_empty() && !r.hidden
            && !self.doc.group_hides(**id) && self.line_shown(**id, r, detail)) {
            // as see-through as its layer
            let mut layer = painter.clone();
            layer.multiply_opacity(self.doc.group_opacity(*id));
            let painter = &layer;
            // the longest stretch of the line that is on screen in one piece
            let mut pieces: Vec<Vec<[f32; 2]>> = vec![Vec::new()];
            let mut prev: Option<egui::Pos2> = None;
            for q in densify_ll(&r.pts) {
                let s = vg.to_screen(q);
                match s {
                    Some(s) if prev.is_none_or(|p| (p.x - s.x).abs() < vg.rect.width() * 0.5) => pieces.last_mut().unwrap().push([s.x, s.y]),
                    Some(s) => pieces.push(vec![[s.x, s.y]]),
                    None => pieces.push(Vec::new()),
                }
                prev = s;
            }
            let len = |p: &Vec<[f32; 2]>| p.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum::<f32>();
            let Some(path) = pieces.into_iter().max_by(|a, b| len(a).total_cmp(&len(b))) else { continue };
            let c = r.name_color(self.doc.style.river);
            let colour = Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
            let job = |text: &str, col: Color32| {
                let mut j = egui::text::LayoutJob::default();
                j.append(text, 0.0, egui::TextFormat { font_id: egui::FontId::proportional(NAME_PX), color: col, italics: r.route.is_none(), ..Default::default() });
                j
            };
            let glyphs: Vec<String> = r.name.chars().map(String::from).collect();
            let galleys: Vec<_> = glyphs.iter().map(|g| painter.layout_job(job(g, colour))).collect();
            let widths: Vec<f32> = galleys.iter().map(|g| g.size().x).collect();
            let width = r.route.map_or(2.0, |st| st.width);
            let Some(places) = along_path(&path, &widths, NAME_PX * 0.6 + width * 0.5) else { continue };
            for ((g, galley), ([x, y], turn)) in glyphs.iter().zip(galleys).zip(places) {
                let angle = -turn.to_radians();
                let half = galley.size() * 0.5;
                let (cs, sn) = (angle.cos(), angle.sin());
                let rot = |v: egui::Vec2| egui::vec2(v.x * cs - v.y * sn, v.x * sn + v.y * cs);
                let centre = egui::pos2(x, y);
                let tl = centre - rot(half);
                let halo = painter.layout_job(job(g, Color32::from_rgba_unmultiplied(255, 255, 255, 190)));
                for o in [egui::vec2(-1.0, 0.0), egui::vec2(1.0, 0.0), egui::vec2(0.0, -1.0), egui::vec2(0.0, 1.0)] {
                    painter.add(egui::epaint::TextShape::new(tl + o, halo.clone(), Color32::WHITE).with_angle(angle));
                }
                painter.add(egui::epaint::TextShape::new(tl, galley, colour).with_angle(angle));
                taken.push(egui::Rect::from_center_size(centre, egui::Vec2::splat(half.x.max(half.y) * 2.0)));
            }
        }
    }

    /// Every route, styled: dashes and dots in screen space, an arrowhead
    /// on the end. Drawn over the map, under labels.
    pub fn draw_routes(&self, painter: &egui::Painter, vg: &ViewGeom, layer: &crate::draw_layers::LayerFeatures) {
        use egui::Shape;
        let detail = self.map_detail(vg);
        for id in &layer.rivers {
            let r = &self.doc.rivers[id];
            if r.hidden || !self.line_shown(*id, r, detail) { continue; }
            let Some(rs) = r.route else {
                // a river in a layer is drawn in the turn of its layer
                // (rivers in no layer are part of the base map, drawn with it)
                if layer.pass.group.is_some() {
                    let st = &self.doc.style;
                    let stroke = Stroke::new(st.river_width, Color32::from_rgba_unmultiplied(st.river[0], st.river[1], st.river[2], st.river[3]));
                    let mut piece: Vec<egui::Pos2> = Vec::new();
                    for q in densify_ll(&r.pts) {
                        match vg.to_screen(q) {
                            Some(s) if piece.last().is_none_or(|p| (p.x - s.x).abs() < vg.rect.width() * 0.5) => piece.push(s),
                            s => {
                                if piece.len() >= 2 {
                                    painter.add(Shape::line(std::mem::take(&mut piece), stroke));
                                }
                                piece.clear();
                                piece.extend(s);
                            }
                        }
                    }
                    if piece.len() >= 2 {
                        painter.add(Shape::line(piece, stroke));
                    }
                }
                continue;
            };
            let colour = Color32::from_rgba_unmultiplied(rs.color[0], rs.color[1], rs.color[2], rs.color[3]);
            // on screen, broken where the line leaves the view or wraps
            let mut pieces: Vec<Vec<egui::Pos2>> = vec![Vec::new()];
            let mut prev: Option<egui::Pos2> = None;
            for q in densify_ll(&r.pts) {
                let screen = vg.to_screen(q);
                match screen {
                    Some(s) if prev.is_none_or(|p| (p.x - s.x).abs() < vg.rect.width() * 0.5) => pieces.last_mut().unwrap().push(s),
                    Some(s) => pieces.push(vec![s]),
                    None => pieces.push(Vec::new()),
                }
                prev = screen;
            }
            pieces.retain(|p| p.len() >= 2);
            let ends_here = vg.to_screen(*r.pts.last().unwrap_or(&LonLat::new(0.0, 0.0))).is_some();
            let last = pieces.len().saturating_sub(1);
            for (i, mut piece) in pieces.into_iter().enumerate() {
                if rs.broad {
                    // a filled band, see-through, with a thin edge
                    let f: Vec<[f32; 2]> = piece.iter().map(|p| [p.x, p.y]).collect();
                    let Some(outline) = map_core::broad::broad_arrow(&f, rs.width, rs.arrow && i == last && ends_here) else { continue };
                    let flat: Vec<f64> = outline.iter().flat_map(|p| [p[0] as f64, p[1] as f64]).collect();
                    if let (false, Ok(tri)) = (rs.hollow, earcutr::earcut(&flat, &[], 2)) {
                        let fill = rs.broad_fill();
                        let mut mesh = egui::Mesh::default();
                        for p in &outline {
                            mesh.colored_vertex(egui::pos2(p[0], p[1]), Color32::from_rgba_unmultiplied(fill[0], fill[1], fill[2], fill[3]));
                        }
                        mesh.indices = tri.into_iter().map(|i| i as u32).collect();
                        painter.add(Shape::mesh(mesh));
                    }
                    let (edge, dash) = rs.broad_edge(1.0);
                    let mut ring: Vec<egui::Pos2> = outline.iter().map(|p| egui::pos2(p[0], p[1])).collect();
                    ring.push(ring[0]);
                    let stroke = Stroke::new(edge, colour);
                    match dash {
                        None => {
                            painter.add(Shape::line(ring, stroke));
                        }
                        Some([d, gap]) if d > 0.1 => crate::screen_path::dashed(painter, &ring, stroke, d + edge, (gap - edge).max(1.0)),
                        Some([_, gap]) => crate::screen_path::dotted(painter, &ring, colour, gap, edge * 0.5),
                    }
                    continue;
                }
                if rs.arrow && i == last && ends_here {
                    let f: Vec<[f32; 2]> = piece.iter().map(|p| [p.x, p.y]).collect();
                    if let Some((line, head)) = map_core::doc::arrow_end(&f, rs.width) {
                        piece = line.iter().map(|p| egui::pos2(p[0], p[1])).collect();
                        painter.add(Shape::convex_polygon(head.iter().map(|p| egui::pos2(p[0], p[1])).collect(), colour, Stroke::NONE));
                    }
                }
                // a front line's teeth, a railway's ties
                if rs.deco != map_core::deco::Deco::None {
                    let (_, half, height) = map_core::deco::deco_size(rs.deco, rs.width);
                    let runs = crate::screen_path::visible_runs(&piece, painter.clip_rect().expand(half + height + rs.width + 4.0));
                    for (run, distance) in runs {
                        let points: Vec<[f32; 2]> = run.iter().map(|p| [p.x, p.y]).collect();
                        for mark in map_core::deco::decorate_from(&points, rs.deco, rs.width, rs.deco_flip, distance) {
                            match mark {
                                map_core::deco::DecoMark::Fill(points) => {
                                    let polygon = points.iter().map(|p| egui::pos2(p[0], p[1])).collect();
                                    painter.add(Shape::convex_polygon(polygon, colour, Stroke::NONE));
                                }
                                map_core::deco::DecoMark::Stroke([a, b]) => {
                                    painter.line_segment([egui::pos2(a[0], a[1]), egui::pos2(b[0], b[1])], Stroke::new(rs.width, colour));
                                }
                            }
                        }
                    }
                }
                let stroke = Stroke::new(rs.width, colour);
                match rs.pattern(rs.width) {
                    None => {
                        painter.add(Shape::line(piece, stroke));
                    }
                    Some([dash, gap]) if dash > 0.1 => {
                        // egui dashes have square ends; shorten them by the caps the export adds
                        crate::screen_path::dashed(painter, &piece, stroke, dash + rs.width, (gap - rs.width).max(1.0));
                    }
                    Some([_, gap]) => {
                        crate::screen_path::dotted(painter, &piece, colour, gap, rs.width * 0.5);
                    }
                }
            }
        }
    }

    /// The selected river highlighted; with the vertex tool, river points
    /// near the cursor as round handles.
    pub fn draw_river_marks(&self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        let line = |pts: &[LonLat], stroke: Stroke| {
            let s: Vec<_> = pts.iter().map(|p| vg.to_screen(*p)).collect();
            for w in s.windows(2) {
                if let (Some(a), Some(b)) = (w[0], w[1]) {
                    if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                        painter.line_segment([a, b], stroke);
                    }
                }
            }
        };
        if let Some(r) = self.selected_river.and_then(|id| self.doc.rivers.get(&id)) {
            line(&r.pts, Stroke::new(6.0, Color32::from_rgba_unmultiplied(255, 170, 0, 120)));
        }
        // the city under the cursor (or being dragged) with the city tool
        if self.tool == crate::view::Tool::City {
            let tol = hover.and_then(|h| vg.to_screen(h)).map(|hs| vg.deg_per_px(hs, 8.0)).unwrap_or(0.0);
            let id = self.drag_city.or_else(|| hover.and_then(|h| self.doc.city_at(h, tol)));
            if let Some(s) = id.and_then(|id| self.doc.entities.get(&id)).and_then(|e| e.pos).and_then(|p| vg.to_screen(p)) {
                painter.circle_stroke(s, 8.0, Stroke::new(2.0, Color32::from_rgb(230, 80, 20)));
            }
        }
        if let Some(s) = self.selected.and_then(|id| self.doc.entities.get(&id)).filter(|e| e.kind == map_core::EntityKind::City).and_then(|e| e.pos).and_then(|p| vg.to_screen(p)) {
            painter.circle_stroke(s, 6.5, Stroke::new(2.0, Color32::from_rgb(255, 170, 0)));
        }
        if self.tool != crate::view::Tool::EditVertex {
            return;
        }
        let Some(h) = hover else { return };
        let Some(hs) = vg.to_screen(h) else { return };
        let radius = vg.deg_per_px(hs, 160.0);
        for (id, r) in self.doc.rivers.iter().filter(|(id, r)| !r.hidden && !self.doc.group_hides(**id) && !self.doc.curves.contains_key(*id)) {
            for (i, q) in r.pts.iter().enumerate() {
                if geom::geo_dist_deg(*q, h) > radius {
                    continue;
                }
                if let Some(s) = vg.to_screen(*q) {
                    let active = self.drag_river == Some((*id, i));
                    painter.circle_filled(s, if active { 5.0 } else { 3.5 }, Color32::from_rgb(20, 110, 210));
                    painter.circle_stroke(s, if active { 5.0 } else { 3.5 }, Stroke::new(1.0, Color32::WHITE));
                }
            }
        }
    }

    /// Side panel for the selected river.
    pub fn river_panel(&mut self, ui: &mut egui::Ui) {
        let Some(id) = self.selected_river.filter(|id| self.doc.rivers.contains_key(id)) else {
            self.selected_river = None;
            return;
        };
        ui.separator();
        let kind = self.doc.rivers[&id].kind_name();
        ui.label(RichText::new(crate::trf!("所选{kind}")).strong());
        let cur = self.doc.rivers[&id].name.clone();
        ui.horizontal(|ui| {
            ui.label(crate::i18n::tr("名称"));
            if self.river_name.as_ref().is_none_or(|(r, _)| *r != id) {
                self.river_name = Some((id, cur.clone()));
            }
            let buf = &mut self.river_name.as_mut().unwrap().1;
            let r = ui.text_edit_singleline(buf);
            if r.lost_focus() && *buf != cur {
                let v = buf.clone();
                self.edit(&format!("重命名{kind}"), |d| {
                    let mut r = d.rivers.get(&id).ok_or("线不存在")?.clone();
                    r.name = v;
                    d.rivers.insert(id, r);
                    Ok(())
                });
            }
        });
        ui.label(crate::trf!("长度约 {} km · {} 个点", map_core::doc::format_num(self.doc.river_length_km(id).round()), self.doc.rivers[&id].pts.len()));
        if self.doc.rivers[&id].route.is_some() {
            ui.weak(crate::roads_ui::travel_lines(self.doc.river_length_km(id))).on_hover_text(crate::i18n::tr("按每日行程估算：步行 30 km、车队 25 km、骑马 60 km、驿马换马 200 km；一天按十小时计"));
        }
        self.curve_panel(ui, id);
        let mut show = self.doc.rivers[&id].show_name;
        if ui.checkbox(&mut show, crate::i18n::tr("沿线标注名称")).on_hover_text(crate::i18n::tr("把名称沿着这条线写在线的上方，从左往右读；线在屏幕上太短时不显示")).changed() {
            self.edit(if show { "显示线名" } else { "隐藏线名" }, |d| {
                let mut r = d.rivers.get(&id).ok_or("线不存在")?.clone();
                r.show_name = show;
                d.rivers.insert(id, r);
                Ok(())
            });
        }
        ui.horizontal(|ui| {
            use map_core::shape::LineStyle;
            ui.label(crate::i18n::tr("形状："));
            for (style, text) in [(LineStyle::Smooth, "平滑"), (LineStyle::Natural, "自然化")] {
                if ui.small_button(text).on_hover_text(crate::i18n::tr("两端不动；细节精度取当前缩放，自然化使用工具栏的粗糙度")).clicked() {
                    let min_len = 4.0 * 57.29578 / self.cam2d.zoom.max(1e-9);
                    let amount = self.roughness as f64;
                    self.edit(text, |d| {
                        let mut r = d.rivers.get(&id).ok_or("河流不存在")?.clone();
                        let mut fixed = vec![false; r.pts.len()];
                        fixed[0] = true;
                        *fixed.last_mut().unwrap() = true;
                        r.pts = map_core::shape::apply(&r.pts, &fixed, false, style, amount, min_len).0;
                        d.rivers.insert(id, r);
                        // new points: no longer the curve's
                        d.curves.remove(&id);
                        Ok(())
                    });
                }
            }
            if ui.small_button(crate::trf!("删除{kind}")).clicked() {
                self.delete_selected_river();
            }
        });
        if let Some(mut st) = self.doc.rivers.get(&id).and_then(|r| r.route) {
            let current = self.doc.rivers[&id].legend.clone();
            if let Some(t) = crate::area_ui::legend_field(ui, &mut self.route_legend, id, &current) {
                self.edit("修改图例说明", |d| {
                    let mut r = d.rivers.get(&id).ok_or("线不存在")?.clone();
                    r.legend = t;
                    d.rivers.insert(id, r);
                    Ok(())
                });
            }
            self.style_row(ui, &[map_core::features::FeatureRef::Route(id)]);
            let (changed, started, stopped) = route_style_ui(ui, &mut st);
            if started {
                self.begin_pending("调整线路样式");
            }
            if changed {
                let apply = move |d: &mut map_core::Document| -> Result<(), String> {
                    let mut r = d.rivers.get(&id).ok_or("线不存在")?.clone();
                    r.route = Some(st);
                    d.rivers.insert(id, r);
                    Ok(())
                };
                if self.pending.is_some() {
                    let _ = apply(&mut self.doc);
                    if let Some(pe) = &mut self.pending {
                        pe.changed = true;
                    }
                } else {
                    self.edit("调整线路样式", apply);
                }
            }
            if stopped {
                self.end_pending();
            }
        }
        ui.weak(crate::trf!("编辑顶点工具（E）：拖动{kind}的点，拖动线上任意处插入点，右键删除点。选择工具下拖动这条线可整体移动，Ctrl+D 复制。{kind}不随年份变化。"));
    }
}

/// The line with segments longer than a degree subdivided, so it bends
/// with the projection; longitudes kept continuous across the seam.
pub(crate) fn densify_ll(pts: &[LonLat]) -> Vec<LonLat> {
    let un = geom::unwrap_path(pts, None);
    let mut out = Vec::new();
    for w in un.windows(2) {
        let n = ((w[1][0] - w[0][0]).abs().max((w[1][1] - w[0][1]).abs()).ceil() as usize).max(1);
        for k in 0..n {
            let t = k as f64 / n as f64;
            out.push(LonLat::new(geom::wrap_lon(w[0][0] + (w[1][0] - w[0][0]) * t), w[0][1] + (w[1][1] - w[0][1]) * t));
        }
    }
    if let Some(l) = un.last() {
        out.push(LonLat::new(geom::wrap_lon(l[0]), l[1]));
    }
    out
}
