//! Country labels: drawn automatically along the line that suits each
//! country's shape (`map_core::labels`), bent along it on a flat map and
//! painted onto the surface on the globe, unless placed by hand. The label
//! tool drags a label to where it should be; the country panel turns,
//! sizes, hides or resets it. A placed label is layout, the same in every
//! year, and is always shown.

use crate::app::App;
use crate::view::{Tool, ViewGeom, ViewKind};
use eframe::egui::{self, epaint::Vertex, Color32, Mesh, Pos2, Rect, RichText, Stroke, Vec2};
use map_core::doc::LabelPlace;
use map_core::labels::{name_size, name_spacing, LabelPath};
use map_core::EntityId;
use std::sync::Arc;

/// Largest and smallest automatic name (px); a name that would be smaller
/// than `MIN_FIT` is left out.
const MAX_PX: f32 = 38.0;
const MIN_PX: f32 = 9.0;
const MIN_FIT: f32 = 8.5;

/// A line drawn on the screen for a name to follow (straight on beyond
/// its ends).
struct Bent {
    pts: Vec<Pos2>,
    cum: Vec<f32>,
}

impl Bent {
    fn new(pts: Vec<Pos2>) -> Self {
        let mut cum = vec![0.0];
        for w in pts.windows(2) {
            cum.push(cum.last().unwrap() + (w[1] - w[0]).length());
        }
        Bent { pts, cum }
    }

    fn len(&self) -> f32 {
        *self.cum.last().unwrap()
    }

    fn at(&self, s: f32) -> Pos2 {
        let n = self.pts.len();
        let i = self.cum.partition_point(|c| *c < s).clamp(1, n - 1);
        let seg = (self.cum[i] - self.cum[i - 1]).max(1e-6);
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        a + (b - a) * ((s - self.cum[i - 1]) / seg)
    }

    /// `x` along from the middle, `y` up from the line; the line's turn
    /// taken over `smooth` each way, so a glyph does not kink at a corner.
    fn place(&self, x: f32, y: f32, smooth: f32) -> Pos2 {
        let s = self.len() * 0.5 + x;
        let t = (self.at(s + smooth) - self.at(s - smooth)).normalized();
        self.at(s) + Vec2::new(t.y, -t.x) * y
    }
}

/// Where a name goes on one view.
enum Line {
    /// Bent along a line on the screen.
    Flat(Bent),
    /// Painted onto the globe along a line on the sphere: `k` degrees per
    /// pixel, read the other way round when `flip` (seen upside down).
    Globe { path: LabelPath, k: f64, flip: bool },
}

impl Line {
    /// The name's line on a view: on a flat map the longest piece of it as
    /// drawn (a line across the map's edge comes apart there), read left
    /// to right; on the globe the line itself.
    fn on(vg: &ViewGeom, path: &LabelPath) -> Option<Line> {
        match vg.kind {
            ViewKind::Flat => {
                let jump = (90.0 * vg.px_per_deg()) as f32;
                let mut pieces: Vec<Vec<Pos2>> = vec![vec![]];
                for p in path.line(32).into_iter().map(|ll| vg.to_screen(ll)) {
                    let p = p?;
                    if pieces.last().unwrap().last().is_some_and(|q| (p - *q).length() > jump) {
                        pieces.push(vec![]);
                    }
                    pieces.last_mut().unwrap().push(p);
                }
                let length = |points: &[Pos2]| points.windows(2).map(|pair| pair[0].distance(pair[1])).sum::<f32>();
                let mut best = pieces.into_iter().filter(|p| p.len() >= 2).max_by(|a, b| length(a).total_cmp(&length(b)))?;
                if best.last().unwrap().x < best[0].x {
                    best.reverse();
                }
                Some(Line::Flat(Bent::new(best)))
            }
            ViewKind::Globe => {
                let (mid, q) = (path.length() * 0.5, path.length() * 0.25);
                let (a, b) = (vg.to_screen(path.point(mid - q, 0.0))?, vg.to_screen(path.point(mid + q, 0.0))?);
                Some(Line::Globe { path: path.clone(), k: 1.0 / vg.px_per_deg(), flip: b.x < a.x })
            }
        }
    }

    /// Its length and the room across it on the screen (px).
    fn room(&self, vg: &ViewGeom, path: &LabelPath) -> (f32, f32) {
        let ppd = vg.px_per_deg();
        match self {
            Line::Flat(b) => {
                let mid = path.length() * 0.5;
                let across = match (vg.to_screen(path.point(mid, path.height * 0.5)), vg.to_screen(path.point(mid, -path.height * 0.5))) {
                    (Some(a), Some(b)) => (a - b).length(),
                    _ => (path.height * ppd) as f32,
                };
                (b.len(), across)
            }
            Line::Globe { .. } => ((path.length() * ppd) as f32, (path.height * ppd) as f32),
        }
    }

    /// The screen point of a point of the name (px along from its middle,
    /// px up from its middle line); `None` behind the globe.
    fn place(&self, vg: &ViewGeom, x: f32, y: f32, smooth: f32) -> Option<Pos2> {
        match self {
            Line::Flat(b) => Some(b.place(x, y, smooth)),
            Line::Globe { path, k, flip } => {
                let (x, y) = if *flip { (-x, -y) } else { (x, y) };
                vg.to_screen(path.point(path.length() * 0.5 + x as f64 * k, y as f64 * k))
            }
        }
    }
}

/// The glyphs of a laid-out name, each corner taken to the screen by
/// `place` (px along from the name's middle, px up from its middle line);
/// `None` when a corner has nowhere to go.
/// (`scale`: the galley was laid out at a size `scale` times smaller than
/// the name is drawn; see [`quantised`].)
fn bend(galley: &egui::Galley, scale: f32, uv: Vec2, colour: Color32, place: impl Fn(f32, f32) -> Option<Pos2>) -> Option<Mesh> {
    let mut mesh = Mesh::default();
    let (cx, cy) = (galley.mesh_bounds.center().x, galley.rect.center().y);
    for row in &galley.rows {
        let m = &row.visuals.mesh;
        let base = mesh.vertices.len() as u32;
        for v in &m.vertices {
            let q = row.pos + v.pos.to_vec2();
            mesh.vertices.push(Vertex { pos: place((q.x - cx) * scale, (cy - q.y) * scale)?, uv: Pos2::new(v.uv.x * uv.x, v.uv.y * uv.y), color: colour });
        }
        mesh.indices.extend(m.indices.iter().map(|i| i + base));
    }
    Some(mesh)
}

/// The boxes of a bent name's glyphs (each glyph is one quad).
fn glyph_boxes(mesh: &Mesh, pad: f32) -> Vec<Rect> {
    mesh.vertices.chunks(4).map(|quad| {
        let mut rect = Rect::NOTHING;
        for vertex in quad { rect.extend_with(vertex.pos); }
        rect.expand(pad)
    }).collect()
}

/// A name ready to draw: its line on the view, its glyphs and their size.
struct Laid {
    e: EntityId,
    place: Option<LabelPlace>,
    line: Line,
    /// The name laid out spread along its line, then (for an automatic
    /// one) close set and smaller, each with how far it may slide along
    /// the line either way (px), and the scale it is drawn at (see
    /// [`quantised`]).
    galleys: Vec<(Arc<egui::Galley>, f32, f32)>,
    size: f32,
    colour: Color32,
    /// Outside the region of the local view being looked at.
    faded: bool,
}

impl App {
    /// Draw the country labels, recording where each one is for the label
    /// tool. `taken` collects their glyphs' boxes so city names keep clear.
    pub fn draw_country_labels(&self, painter: &egui::Painter, vg: &ViewGeom, taken: &mut crate::label_boxes::LabelBoxes) {
        let editing = self.tool == Tool::Label;
        let mut rects: Vec<(EntityId, Rect)> = Vec::new();
        if !self.doc.style.show_labels && !editing {
            self.label_rects.borrow_mut()[vg.kind.index()] = rects;
            return;
        }
        let label_col = {
            let c = self.doc.style.label;
            Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
        };
        // placed labels first (they always show), then the automatic ones largest first
        let placed = self.doc.entities.iter().filter(|(_, x)| {
            x.kind == map_core::EntityKind::Country && x.layer == self.doc.active_layer
                && x.exists_in(self.doc.year) && (editing || x.parent.is_none())
        }).filter_map(|(&id, x)| x.label.map(|place| (id, x, Some(place), self.label_cache.path(id))));
        let automatic = self.labels.iter().filter_map(|(id, path)| {
            let entity = self.doc.entities.get(id)?;
            (entity.label.is_none()).then_some((*id, entity, None, Some(path)))
        });
        // all laid out first, so the glyph atlas holds every glyph when
        // their places in it are read
        let mut laid: Vec<Laid> = Vec::new();
        for (e, ent, place, path) in placed.chain(automatic) {
            if !editing && (ent.label_hidden || (place.is_none() && path.is_some_and(|path| !line_near_view(vg, path)))) {
                continue;
            }
            let tier = map_core::label_type::Tier::administrative(self.doc.ancestors(e).count().saturating_sub(1));
            let style = self.doc.style.label_type(tier);
            let family = self.font_family(style.font, style.bold);
            let font = |size: f32| egui::FontId::new(size, family.clone());
            // a local view: names outside its region are faded, like the land
            let outside = self.focus_view.is_some()
                && self.active_focus().is_some_and(|f| !f.contains(place.map(|p| p.at).or(path.map(|p| p.middle())).unwrap_or(f.center())));
            let colour = if ent.label_hidden || outside { label_col.gamma_multiply(0.35) } else { label_col };
            let name = ent.name.trim();
            // the name's width in em, in its font
            let em = painter.layout_no_wrap(name.to_string(), font(20.0), colour).size().x / 20.0;
            if em <= 0.0 {
                continue;
            }
            let (line, size, spacing, len) = match place {
                Some(pl) => {
                    // as large as its automatic name would be here, times its own scale
                    let base = path.and_then(|p| Line::on(vg, p).map(|l| l.room(vg, p))).map_or(style.size, |(len, room)| (name_size(len as f64, room as f64, em as f64) as f32).clamp(MIN_PX, MAX_PX));
                    let size = (base * pl.scale).clamp(6.0, 120.0);
                    let line = match vg.kind {
                        ViewKind::Flat => {
                            let Some(at) = vg.to_screen(pl.at) else { continue };
                            // egui's y runs down; the stored angle is counter-clockwise
                            let (s, c) = pl.angle.to_radians().sin_cos();
                            let dir = Vec2::new(c, -s) * 2000.0;
                            Line::Flat(Bent::new(vec![at - dir, at + dir]))
                        }
                        ViewKind::Globe => {
                            let Some(l) = Line::on(vg, &LabelPath::straight(pl.at, pl.angle as f64, 30.0, 1.0, em as f64)) else { continue };
                            l
                        }
                    };
                    (line, size, 0.0, 0.0)
                }
                None => {
                    let Some(path) = path else { continue };
                    let Some(line) = Line::on(vg, path) else { continue };
                    let (len, room) = line.room(vg, path);
                    let fitted = name_size(len as f64, room as f64, em as f64) as f32;
                    if fitted < MIN_FIT && !editing {
                        continue;
                    }
                    let size = fitted.clamp(MIN_PX, MAX_PX);
                    // a long line: the letters spread along it
                    let spacing = name_spacing(len as f64, (em * size) as f64, size as f64, name.chars().count()) as f32;
                    (line, size, spacing, len)
                }
            };
            // (an automatic name may also be set closer, or a little smaller,
            // to keep off a city's dot)
            let mut ways = vec![(spacing, size)];
            if place.is_none() {
                if spacing > 0.0 {
                    ways.push((0.0, size));
                }
                if size * 0.8 >= MIN_PX {
                    ways.push((0.0, size * 0.8));
                }
            }
            let galleys = ways
                .iter()
                .map(|(sp, size)| {
                    // laid out at a size from a fixed ladder and scaled to the
                    // size wanted: zooming asks for a new size every frame, and
                    // each new size had every glyph rasterised again
                    let q = quantised(*size);
                    let scale = size / q;
                    let mut job = egui::text::LayoutJob::default();
                    job.append(name, 0.0, egui::TextFormat { font_id: font(q), color: colour, extra_letter_spacing: *sp / scale, ..Default::default() });
                    let galley = painter.layout_job(job);
                    // (the room across the line holds over the middle of it)
                    let slack = ((len * 0.88 - galley.mesh_bounds.width() * scale) * 0.5).max(0.0);
                    (galley, slack, scale)
                })
                .collect();
            laid.push(Laid { e, place, line, galleys, size, colour, faded: outside });
        }
        let atlas = painter.ctx().fonts(|f| f.font_image_size());
        if self.label_atlas.replace(atlas) != atlas {
            // the atlas grew while this frame was laid out: right next frame
            painter.ctx().request_repaint();
        }
        let uv = Vec2::new(1.0 / atlas[0] as f32, 1.0 / atlas[1] as f32);
        // city dots, for names to keep off
        let mut dots = crate::label_boxes::LabelBoxes::default();
        if self.doc.style.show_cities {
            let detail = self.map_detail(vg);
            dots.extend(self.doc
                .entities
                .iter()
                .filter(|(id, x)| x.kind == map_core::EntityKind::City && x.exists_in(self.doc.year)
                    && self.city_shown(**id, detail))
                .filter_map(|(_, x)| x.pos.and_then(|p| vg.to_screen(p)))
                .map(|p| Rect::from_center_size(p, Vec2::splat(9.0))));
        }
        for Laid { e, place, line, galleys, size, colour, faded } in laid {
            let smooth = (size * 0.5).max(2.0);
            // where it is meant to be, else slid along its line off the city
            // dots, else set closer and slid
            let mut found = None;
            let tries = galleys.iter().flat_map(|(g, slack, scale)| [0.0, 0.5, -0.5, 1.0, -1.0].map(|k| (g.clone(), k * slack, *scale)));
            for (galley, shift, scale) in tries {
                let Some(mesh) = bend(&galley, scale, uv, colour, |x, y| line.place(vg, x + shift, y, smooth)) else { continue };
                let boxes = glyph_boxes(&mesh, 1.5);
                // (a dot in the gap between two letters is on the name too)
                let clear = !boxes.windows(2).map(|w| w[0].union(w[1])).chain(boxes.first().copied())
                    .any(|span| dots.intersects(span));
                if found.is_none() || clear {
                    found = Some((mesh, boxes, shift, galley, scale));
                }
                if clear {
                    break;
                }
            }
            let Some((mesh, boxes, shift, galley, scale)) = found else { continue };
            let at = |x: f32, y: f32| line.place(vg, x + shift, y, smooth);
            if mesh.vertices.is_empty() {
                continue;
            }
            let all = boxes.iter().fold(Rect::NOTHING, |a, b| a.union(*b));
            if place.is_none() && !editing {
                // on the globe: not when seen so slantwise it cannot be read
                if matches!(line, Line::Globe { .. }) {
                    let along = (galley.mesh_bounds.width() * scale).max(1.0);
                    let squeezed = match (at(-along * 0.5, 0.0), at(along * 0.5, 0.0), at(0.0, size * 0.5), at(0.0, -size * 0.5)) {
                        (Some(a), Some(b), Some(c), Some(d)) => (b - a).length() < along * 0.55 || (c - d).length() < size * 0.55,
                        _ => true,
                    };
                    if squeezed {
                        continue;
                    }
                }
                if !vg.rect.contains_rect(all) || boxes.iter().any(|g| taken.intersects(*g)) {
                    continue;
                }
            }
            taken.extend(boxes.iter().copied());
            rects.push((e, all));
            // a soft white halo, then the name
            // white behind dark names, dark behind light ones (a dark theme)
            let h = map_core::doc::halo_for(self.doc.style.label);
            let halo = Color32::from_rgba_unmultiplied(h[0], h[1], h[2], if faded { 60 } else { 150 });
            let mut outlined = Mesh::with_texture(mesh.texture_id);
            outlined.vertices.reserve(mesh.vertices.len() * 9);
            outlined.indices.reserve(mesh.indices.len() * 9);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let o = Vec2::new(a.cos(), a.sin()) * 1.4;
                let base = outlined.vertices.len() as u32;
                outlined.vertices.extend(mesh.vertices.iter().map(|v| Vertex { pos: v.pos + o, color: halo, ..*v }));
                outlined.indices.extend(mesh.indices.iter().map(|index| base + index));
            }
            outlined.append(mesh);
            painter.add(egui::Shape::mesh(outlined));
            if editing {
                let strong = self.selected == Some(e) || self.drag_label.is_some_and(|d| d.0 == e);
                let col = if place.is_some() { Color32::from_rgb(230, 80, 20) } else { Color32::from_gray(120) };
                painter.rect_stroke(all, 2.0, Stroke::new(if strong { 2.0 } else { 0.8 }, col), egui::StrokeKind::Outside);
            }
        }
        self.label_rects.borrow_mut()[vg.kind.index()] = rects;
    }

    /// The label tool: drag a label to place it, click one to select its country.
    pub fn label_input(&mut self, resp: &egui::Response, vg: &ViewGeom) {
        use egui::PointerButton::Primary;
        // free text lies on top of country labels
        if self.drag_label.is_none() && self.annotation_input(resp, vg) {
            return;
        }
        let hit = |rects: &[(EntityId, Rect)], at: Pos2| rects.iter().rev().find(|(_, r)| r.contains(at)).map(|(e, r)| (*e, r.center()));
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let found = origin.and_then(|o| hit(&self.label_rects.borrow()[vg.kind.index()], o).map(|(e, c)| (e, c - o)));
            if let Some((e, off)) = found {
                self.begin_pending("移动标注");
                self.drag_label = Some((e, off));
                self.select_country(e);
            }
        }
        if let Some((e, off)) = self.drag_label {
            if resp.dragged_by(Primary) {
                if let Some(ll) = resp.interact_pointer_pos().and_then(|p| vg.to_lonlat(p + off)) {
                    if let Some(ent) = self.doc.entities.get_mut(&e) {
                        let keep = ent.label.unwrap_or(LabelPlace::at(ll));
                        ent.label = Some(LabelPlace { at: ll, ..keep });
                        if let Some(pe) = &mut self.pending {
                            pe.changed = true;
                        }
                    }
                }
            }
            if resp.drag_stopped() {
                self.drag_label = None;
                self.end_pending();
                self.set_status(crate::i18n::tr("标注已放置：所有年份相同，始终显示；国家属性中可旋转、缩放、隐藏或恢复自动"), false);
            }
            return;
        }
        if resp.clicked() {
            let found = resp.interact_pointer_pos().and_then(|p| hit(&self.label_rects.borrow()[vg.kind.index()], p));
            if let Some((e, _)) = found {
                self.select_country(e);
            }
        }
    }

    fn select_country(&mut self, e: EntityId) {
        self.selected = Some(e);
        self.selected_face = None;
        self.selected_river = None;
        self.tab = crate::panels::Tab::Countries;
    }

    /// Label settings for the selected country.
    pub fn label_panel(&mut self, ui: &mut egui::Ui, sel: EntityId) {
        let Some(ent) = self.doc.entities.get(&sel) else { return };
        let (place, hidden) = (ent.label, ent.label_hidden);
        ui.horizontal(|ui| {
            ui.label(RichText::new(crate::i18n::tr("标注")).strong());
            ui.weak(if place.is_some() { crate::i18n::tr("手动放置") } else { crate::i18n::tr("自动位置") });
            let mut h = hidden;
            if ui.checkbox(&mut h, crate::i18n::tr("隐藏")).changed() {
                self.edit(if h { "隐藏标注" } else { "显示标注" }, |d| {
                    d.entities.get_mut(&sel).ok_or("国家不存在")?.label_hidden = h;
                    Ok(())
                });
            }
            if place.is_some() && ui.small_button(crate::i18n::tr("恢复自动")).clicked() {
                self.edit("恢复自动标注", |d| {
                    d.entities.get_mut(&sel).ok_or("国家不存在")?.label = None;
                    Ok(())
                });
            }
        });
        let Some(pl) = place else {
            ui.weak(crate::i18n::tr("用“标注 T”工具拖动标注即可手动放置。"));
            return;
        };
        let (mut angle, mut scale) = (pl.angle, pl.scale);
        let ra = ui.add(egui::Slider::new(&mut angle, -90.0..=90.0).suffix("°").text(crate::i18n::tr("角度")));
        let rs = ui.add(egui::Slider::new(&mut scale, 0.4..=3.0).text(crate::i18n::tr("大小")));
        for r in [&ra, &rs] {
            if r.drag_started() {
                self.begin_pending("调整标注");
            }
        }
        if ra.changed() || rs.changed() {
            let dragging = self.pending.is_some();
            let apply = move |d: &mut map_core::Document| -> Result<(), String> {
                let e = d.entities.get_mut(&sel).ok_or("国家不存在")?;
                e.label = Some(LabelPlace { angle, scale, ..pl });
                Ok(())
            };
            if dragging {
                let _ = apply(&mut self.doc);
                if let Some(pe) = &mut self.pending {
                    pe.changed = true;
                }
            } else {
                self.edit("调整标注", apply);
            }
        }
        if ra.drag_stopped() || rs.drag_stopped() {
            self.end_pending();
        }
    }
}

/// Whether a name's line (with room for its letters around it) comes into
/// the view: a few points along it, their box grown by the line's height.
fn line_near_view(vg: &ViewGeom, path: &LabelPath) -> bool {
    let pts: Vec<Pos2> = path.line(8).into_iter().filter_map(|ll| vg.to_screen(ll)).collect();
    if pts.is_empty() {
        return false;
    }
    let b = pts.iter().fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(*p, *p)));
    let grow = (path.height * vg.px_per_deg()) as f32;
    b.expand(grow.min(400.0)).intersects(vg.rect)
}

/// A text size from a fixed ladder (steps of 8 %) near `size`: names are
/// laid out at it and scaled the rest of the way.
pub(crate) fn quantised(size: f32) -> f32 {
    let step = 1.08f32.ln();
    (size.max(1.0).ln() / step).round().mul_add(step, 0.0).exp()
}
