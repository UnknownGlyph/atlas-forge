//! Drawing tools (land, lake, river, split line): the draft, its editing,
//! freehand strokes, snapping and tracing along existing boundaries, the
//! preview, and applying the result (in the background when it needs a
//! rebuild).

use crate::app::App;
use crate::view::{DrawMode, Tool, ViewGeom, ViewKind};
use eframe::egui::{self, Color32, Pos2, Stroke, Vec2};
use map_core::doc::{Document, EntityId, LayerId};
use map_core::geom::{self, LonLat};
use map_core::overlay::{DrawIntent, OverlayReport};
use map_core::topo::FaceKind;
use std::time::Instant;

/// Pixels within which the pointer grabs a draft point or snaps to a border.
const GRAB_PX: f32 = 8.0;

/// What a styled draft was computed from.
pub type StyleKey = (Vec<LonLat>, Vec<bool>, map_core::shape::LineStyle, u32, u32, bool);

/// What a primary drag on the map is doing to the draft.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DraftDrag {
    /// Moving the draft point at this index.
    Move(usize),
    /// A freehand stroke that started at this draft index.
    Stroke(usize),
}

/// A drawn shape being applied in the background (an overlay rebuild).
pub struct BgEdit {
    pub label: &'static str,
    /// History generation when it started: if anything else is edited in
    /// the meantime the result no longer applies.
    pub generation: u64,
    pub year: Option<i32>,
    pub layer: LayerId,
    pub started: Instant,
    /// The draft, restored when the result cannot be used.
    pub pts: Vec<LonLat>,
    pub created: Option<EntityId>,
    pub handle: std::thread::JoinHandle<Result<(Document, OverlayReport), String>>,
}

impl App {
    fn is_snapped(&self, i: usize) -> bool {
        self.draft_snap.get(i).copied().unwrap_or(false)
    }

    fn sync_snaps(&mut self) {
        self.draft_snap.resize(self.draft.len(), false);
    }

    /// The draft in the chosen line style (None when plain). Detail is
    /// added down to about 4 screen pixels; traced/snapped runs stay.
    pub fn styled_draft(&mut self, vg: &ViewGeom) -> Option<Vec<LonLat>> {
        use map_core::shape::LineStyle;
        if self.line_style == LineStyle::Plain || self.draft.len() < 2 {
            return None;
        }
        let closed = matches!(self.tool, Tool::DrawLand | Tool::DrawLake | Tool::Hatch) && self.draft.len() >= 3;
        let min_len = vg.deg_per_px(vg.rect.center(), 4.0);
        self.sync_snaps();
        // rivers have no area to keep
        let keep = self.keep_area && self.tool != Tool::River;
        let key: StyleKey = (self.draft.clone(), self.draft_snap.clone(), self.line_style, self.roughness.to_bits(), (min_len as f32).to_bits(), keep);
        if self.styled_cache.as_ref().is_none_or(|(k, _)| *k != key) {
            let (mut pts, flags) = map_core::shape::apply(&self.draft, &self.draft_snap, closed, self.line_style, self.roughness as f64, min_len);
            if keep {
                // when the style cannot keep the area, the drawn shape is used as clicked
                pts = map_core::shape::preserve_area(&self.draft, &pts, &flags, closed).unwrap_or_else(|| self.draft.clone());
            }
            self.styled_cache = Some((key, pts));
        }
        self.styled_cache.as_ref().map(|(_, p)| p.clone())
    }

    pub fn clear_draft(&mut self) {
        self.pen.clear();
        self.pen_pull = false;
        self.draft.clear();
        self.draft_snap.clear();
        self.draft_drag = None;
        self.trace_preview = None;
    }

    pub fn pop_draft(&mut self) {
        if self.tool == Tool::Pen {
            self.pen.pop();
            return;
        }
        self.draft.pop();
        self.sync_snaps();
    }

    /// Snap a point onto a nearby border/coast (vertex first); a river or
    /// route instead snaps to its own kind's line ends and lines (routes
    /// also to cities), never to borders, and is never traced.
    fn snap(&self, p: LonLat, tol: f64) -> (LonLat, bool) {
        if self.tool == Tool::River {
            return (self.doc.line_snap(p, tol, self.route_mode).map_or(p, |s| s.0), false);
        }
        match self.doc.topo.snap_boundary(p, tol) {
            Some(s) => (s, true),
            None => (p, false),
        }
    }

    /// The boundary path from the last draft point to `q` when both lie on
    /// borders and tracing is on (coast only when both are on a coast).
    fn trace_path(&self, q: LonLat, tol: f64) -> Option<Vec<LonLat>> {
        let last = *self.draft.last()?;
        if !self.trace || !self.is_snapped(self.draft.len() - 1) {
            return None;
        }
        let t = &self.doc.topo;
        let coast = |p: LonLat| map_core::trace::locate(t, p, tol).is_some_and(|(e, _, _)| map_core::trace::is_coast(t, e));
        let coast_only = coast(last) && coast(q);
        let path = map_core::trace::boundary_path(t, last, q, tol, coast_only)?;
        (path.len() > 2).then_some(path)
    }

    /// Add a clicked point: snapped, angle-constrained with Shift, and
    /// traced along the boundary from the previous point when possible.
    fn add_draft_point(&mut self, vg: &ViewGeom, screen: Pos2, p: LonLat, shift: bool, ctrl: bool) {
        self.last_draft_view = Some(*vg);
        let tol = vg.deg_per_px(screen, GRAB_PX);
        let (mut q, mut snapped) = self.snap(p, tol);
        // Ctrl: no snapping for a river or route either
        if ctrl && self.tool == Tool::River {
            q = p;
        }
        // Shift: 15° steps from the previous point, on screen
        if shift {
            if let Some(prev) = self.draft.last().and_then(|l| vg.to_screen(*l)) {
                let d = screen - prev;
                let step = std::f32::consts::PI / 12.0;
                let a = (d.y.atan2(d.x) / step).round() * step;
                let s = prev + Vec2::new(a.cos(), a.sin()) * d.length();
                if let Some(ll) = vg.to_lonlat(s) {
                    (q, snapped) = (ll, false);
                }
            }
        }
        if snapped && !ctrl {
            if let Some(path) = self.trace_path(q, tol) {
                for r in &path[1..] {
                    self.draft.push(*r);
                    self.draft_snap.push(true);
                }
                return;
            }
        }
        self.draft.push(q);
        self.draft_snap.push(snapped);
    }

    fn nearest_anchor(&self, vg: &ViewGeom, pos: Pos2) -> Option<usize> {
        self.draft
            .iter()
            .enumerate()
            .filter_map(|(i, p)| vg.to_screen(*p).map(|s| (i, s.distance(pos))))
            .filter(|x| x.1 <= GRAB_PX)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|x| x.0)
    }

    /// Midpoint handles of the draft's segments (index of the first point).
    fn midpoints(&self, vg: &ViewGeom) -> Vec<(usize, Pos2)> {
        let closed = matches!(self.tool, Tool::DrawLand | Tool::DrawLake) && self.draft.len() >= 3;
        let n = self.draft.len();
        let segs = if closed { n } else { n.saturating_sub(1) };
        (0..segs)
            .filter_map(|i| {
                let (a, b) = (vg.to_screen(self.draft[i])?, vg.to_screen(self.draft[(i + 1) % n])?);
                // short segments (traced boundaries, freehand) get no handle
                (a.distance(b) > 3.0 * GRAB_PX && (a.x - b.x).abs() < vg.rect.width() * 0.5).then(|| (i, a + (b - a) * 0.5))
            })
            .collect()
    }

    /// Input for the drawing tools.
    pub fn draft_input(&mut self, resp: &egui::Response, vg: &ViewGeom, p: LonLat) {
        use egui::PointerButton::{Primary, Secondary};
        let (shift, ctrl) = resp.ctx.input(|i| (i.modifiers.shift, i.modifiers.command));
        let pos = resp.interact_pointer_pos().or(resp.hover_pos()).unwrap_or(vg.rect.center());
        if self.brush_paint && matches!(self.tool, Tool::DrawLand | Tool::DrawLake) {
            return self.brush_input(resp, vg, pos);
        }
        // right click on a draft point removes it
        if resp.clicked_by(Secondary) {
            if let Some(i) = self.nearest_anchor(vg, pos) {
                self.draft.remove(i);
                self.draft_snap.remove(i.min(self.draft_snap.len().saturating_sub(1)));
                self.sync_snaps();
            }
            return;
        }
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin()).unwrap_or(pos);
            let Some(o) = vg.to_lonlat(origin) else { return };
            if let Some(i) = self.nearest_anchor(vg, origin) {
                self.draft_drag = Some(DraftDrag::Move(i));
            } else if let Some((i, _)) = self.midpoints(vg).into_iter().find(|(_, m)| m.distance(origin) <= GRAB_PX) {
                self.draft.insert(i + 1, o);
                self.draft_snap.insert((i + 1).min(self.draft_snap.len()), false);
                self.draft_drag = Some(DraftDrag::Move(i + 1));
            } else {
                if self.tool.ends_on_boundary() && self.draft.is_empty() && self.doc.topo.snap_boundary(o, vg.deg_per_px(origin, GRAB_PX)).is_none() {
                    self.set_status("这条线必须从已有边界开始（光标靠近边界时会吸附）", true);
                    return;
                }
                self.add_draft_point(vg, origin, o, false, ctrl);
                self.draft_drag = Some(DraftDrag::Stroke(self.draft.len() - 1));
            }
        }
        if resp.dragged_by(Primary) {
            match self.draft_drag {
                Some(DraftDrag::Move(i)) if i < self.draft.len() => {
                    let tol = vg.deg_per_px(pos, GRAB_PX);
                    let (q, s) = self.snap(p, tol);
                    self.draft[i] = q;
                    self.sync_snaps();
                    self.draft_snap[i] = s;
                }
                Some(DraftDrag::Stroke(_)) => {
                    let far = self.draft.last().and_then(|l| vg.to_screen(*l)).is_none_or(|l| l.distance(pos) >= 4.0);
                    if far {
                        self.draft.push(p);
                        self.draft_snap.push(false);
                    }
                }
                _ => {}
            }
        }
        if resp.drag_stopped() {
            if let Some(DraftDrag::Stroke(start)) = self.draft_drag.take() {
                self.finish_stroke(vg, start, pos);
            }
            self.draft_drag = None;
            return;
        }
        // a quick click just before a double-click makes egui count the
        // double-click's second click as a triple: that finishes too
        if resp.double_clicked() || (resp.triple_clicked() && !self.draft.is_empty()) {
            self.finish_draft();
        } else if resp.triple_clicked() {
            // the third click of a real triple-click, after the double finished
        } else if resp.clicked() {
            let closes = matches!(self.tool, Tool::DrawLand | Tool::DrawLake | Tool::Hatch) && self.draft.len() >= 3 && vg.to_screen(self.draft[0]).is_some_and(|f| f.distance(pos) < 10.0);
            if closes {
                self.finish_draft();
                return;
            }
            if self.tool.ends_on_boundary() && self.draft.is_empty() {
                let tol = vg.deg_per_px(pos, GRAB_PX);
                if self.doc.topo.snap_boundary(p, tol).is_none() {
                    self.set_status("这条线必须从已有边界开始（光标靠近边界时会吸附）", true);
                    return;
                }
            }
            self.add_draft_point(vg, pos, p, shift, ctrl);
            // a split or redraw line ends on a boundary
            if self.tool.ends_on_boundary() && self.draft.len() >= 2 && self.is_snapped(self.draft.len() - 1) {
                self.finish_draft();
            }
        }
    }

    /// A freehand stroke ended: simplify it on screen, snap its end, and
    /// close or finish the shape when it ends where it should.
    fn finish_stroke(&mut self, vg: &ViewGeom, start: usize, end: Pos2) {
        if start >= self.draft.len() {
            return;
        }
        let screen: Vec<Option<Pos2>> = self.draft[start..].iter().map(|p| vg.to_screen(*p)).collect();
        if screen.iter().all(|s| s.is_some()) {
            let pts: Vec<[f64; 2]> = screen.iter().map(|s| [s.unwrap().x as f64, s.unwrap().y as f64]).collect();
            let keep = map_core::trace::simplify(&pts, 1.5);
            let kept: Vec<(LonLat, bool)> = keep.iter().map(|k| (self.draft[start + k], self.is_snapped(start + k))).collect();
            self.draft.truncate(start);
            self.draft_snap.truncate(start);
            for (p, s) in kept {
                self.draft.push(p);
                self.draft_snap.push(s);
            }
        }
        let tol = vg.deg_per_px(end, GRAB_PX);
        let last = self.draft.len() - 1;
        let (q, s) = self.snap(self.draft[last], tol);
        if s {
            self.draft[last] = q;
            self.draft_snap[last] = true;
        }
        let closes = matches!(self.tool, Tool::DrawLand | Tool::DrawLake | Tool::Hatch) && self.draft.len() >= 4 && vg.to_screen(self.draft[0]).is_some_and(|f| f.distance(end) < 12.0);
        if closes {
            self.pop_draft();
            self.finish_draft();
        } else if self.tool.ends_on_boundary() && self.draft.len() >= 2 && self.is_snapped(last) {
            self.finish_draft();
        }
    }

    /// Draft points with every segment sampled along its straight screen
    /// line (in the 2D view), so the stored geometry is what was drawn: a
    /// long stroke must not take the short way round the planet.
    /// The draft as placed (styled when a line style is set), before any
    /// sampling along the screen. Hatched areas keep these points: they
    /// are drawn bending with the projection anyway, and each placed point
    /// stays one handle for the vertex tool.
    fn drafted(&mut self) -> Vec<LonLat> {
        self.last_draft_view.and_then(|vg| self.styled_draft(&vg)).unwrap_or_else(|| self.draft.clone())
    }

    fn densified(&mut self, closed: bool) -> Vec<LonLat> {
        let pts = self.drafted();
        match self.last_draft_view {
            Some(vg) => screen_densified(&vg, pts, closed),
            None => pts,
        }
    }
}

/// Every segment sampled along its straight screen line (2D view only).
fn screen_densified(vg: &ViewGeom, pts: Vec<LonLat>, closed: bool) -> Vec<LonLat> {
    if vg.kind != ViewKind::Flat {
        return pts;
    }
    let n = pts.len();
    let mut out = Vec::new();
    let segs = if closed { n } else { n.saturating_sub(1) };
    for i in 0..n {
        out.push(pts[i]);
        if i >= segs {
            continue;
        }
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        if let (Some(sa), Some(sb)) = (vg.to_screen(a), vg.to_screen(b)) {
            let k = (sa.distance(sb) / 24.0).ceil() as usize;
            for j in 1..k {
                if let Some(ll) = vg.to_lonlat(sa + (sb - sa) * (j as f32 / k as f32)) {
                    out.push(ll);
                }
            }
        }
    }
    out
}

impl App {
    /// The intent of the land tool with the current brush and draw mode.
    fn land_intent(&self) -> Result<DrawIntent, &'static str> {
        let brush = self.brush.filter(|b| self.doc.entities.get(b).is_some_and(|e| e.kind == map_core::EntityKind::Country && e.layer == self.doc.active_layer));
        match self.draw_mode {
            DrawMode::AddLand => Ok(DrawIntent::AddLand),
            DrawMode::Expand => brush.map(DrawIntent::Expand).ok_or("“扩张”需要先选择画笔国家"),
            DrawMode::Take => brush.map(DrawIntent::Take).ok_or("“夺取”需要先选择画笔国家"),
            // the country is created when the shape is applied
            DrawMode::NewCountry => Ok(DrawIntent::Take(map_core::EntityId(u64::MAX))),
            DrawMode::Province(parent) => Ok(DrawIntent::Subdivide { parent, child: EntityId(u64::MAX) }),
        }
    }

    fn new_country_name(&self) -> String {
        format!("新国家{}", self.doc.countries().len() + 1)
    }

    pub fn finish_draft(&mut self) {
        if self.bg_edit.is_some() {
            return self.set_status("正在处理上一次绘制，请稍候", true);
        }
        if self.tool == Tool::Pen {
            return self.pen_finish(false);
        }
        match self.tool {
            Tool::DrawLand | Tool::DrawLake => {
                if self.draft.len() < 3 {
                    return;
                }
                let kind = if self.tool == Tool::DrawLand { FaceKind::Land } else { FaceKind::Water };
                let intent = if kind == FaceKind::Water {
                    DrawIntent::CarveWater
                } else {
                    match self.land_intent() {
                        Ok(i) => i,
                        Err(e) => return self.set_status(e, true),
                    }
                };
                let province = match intent {
                    DrawIntent::Subdivide { parent, .. } => Some(parent),
                    _ => None,
                };
                let label = if province.is_some() { "绘制下级地区" }
                    else if kind == FaceKind::Land { "绘制陆地" } else { "绘制湖泊" };
                let pts = self.densified(true);
                let new_country = (kind == FaceKind::Land && self.draw_mode == DrawMode::NewCountry).then(|| self.new_country_name());
                // fast path: a closed shape inside open water, or a lake inside
                // one land face, keeps every existing face id
                let container = self.doc.topo.face_at(pts[0]);
                let in_water = self.doc.topo.faces[&container].kind == FaceKind::Water;
                if province.is_none() && (in_water || kind == FaceKind::Water) {
                    let mut d = self.doc.clone();
                    let created = new_country.as_ref().map(|n| d.add_entity(map_core::EntityKind::Country, n));
                    if let Ok(f) = d.add_ring_face(&pts, kind) {
                        let ok = match (created, intent) {
                            (Some(x), _) => d.assign(f, Some(x)).is_ok(),
                            (None, DrawIntent::Expand(x) | DrawIntent::Take(x)) if kind == FaceKind::Land => d.assign(f, Some(x)).is_ok(),
                            _ => true,
                        };
                        if ok {
                            let before = std::mem::replace(&mut self.doc, d);
                            self.history.push(label, before, &self.doc);
                            self.dirty = true;
                            self.selected_face = Some(f);
                            self.selected = created;
                            if created.is_some() {
                                self.brush = created;
                            }
                            self.clear_draft();
                            return self.set_status(format!("{label}：完成"), false);
                        }
                    }
                }
                // crosses existing boundaries: boolean overlay + rebuild, off the UI thread
                let mut doc = self.doc.clone();
                let (intent, created) = if let Some(parent) = province {
                    let n = doc.entities.values().filter(|e| e.parent == Some(parent) && e.kind == map_core::EntityKind::Country).count() + 1;
                    let noun = if doc.entities.get(&parent).is_some_and(|e| e.parent.is_none()) { "省份" } else { "下级地区" };
                    let child = match doc.add_subdivision(parent, &format!("新{noun}{n}")) {
                        Ok(id) => id,
                        Err(e) => return self.set_status(e, true),
                    };
                    (DrawIntent::Subdivide { parent, child }, Some(child))
                } else if let Some(name) = new_country {
                    let id = doc.add_entity(map_core::EntityKind::Country, &name);
                    (DrawIntent::Take(id), Some(id))
                } else {
                    (intent, None)
                };
                let handle = std::thread::spawn(move || map_core::overlay::apply(&doc, &pts, intent));
                self.bg_edit = Some(BgEdit { label, generation: self.history.generation(),
                    year: self.doc.year, layer: self.doc.active_layer, started: Instant::now(),
                    pts: self.draft.clone(), created, handle });
                self.clear_draft();
                self.set_status(format!("{label}：正在计算…"), false);
            }
            Tool::Split => {
                let pts = self.densified(false);
                self.clear_draft();
                if pts.len() >= 2 {
                    self.edit("切分", |d| d.split_face(&pts));
                }
            }
            Tool::Redraw => {
                let pts = self.densified(false);
                let tol = self.last_draft_view.map_or(1e-3, |vg| vg.deg_per_px(vg.rect.center(), 2.0));
                let plan = match map_core::redraw::plan(&self.doc, &pts, tol) {
                    Ok(p) => p,
                    // keep the draft so it can be fixed
                    Err(e) => return self.set_status(format!("重画国界：{e}"), true),
                };
                let doc = self.doc.clone();
                let handle = std::thread::spawn(move || map_core::redraw::apply(&doc, &plan));
                let label = "重画国界";
                self.bg_edit = Some(BgEdit { label, generation: self.history.generation(),
                    year: self.doc.year, layer: self.doc.active_layer, started: Instant::now(),
                    pts: self.draft.clone(), created: None, handle });
                self.clear_draft();
                self.set_status(format!("{label}：正在计算…"), false);
            }
            Tool::Hatch => {
                let ring = self.drafted();
                self.clear_draft();
                if ring.len() >= 3 {
                    let id = map_core::doc::LineId(self.doc.topo.alloc());
                    if self.edit("绘制斜线区域", |d| {
                        d.areas.insert(id, map_core::area::AreaMark::new(ring));
                        Ok(())
                    }).is_some() {
                        self.style_new(map_core::features::FeatureRef::Area(id));
                        self.selected_area = Some(id);
                        self.selected = None;
                        self.selected_river = None;
                        self.tab = crate::panels::Tab::Countries;
                        self.set_status("已添加斜线区域：在国家页面板中命名、调整样式和年份（不改变领土）", false);
                    }
                }
            }
            Tool::River => {
                let (first, last) = (self.draft.first().copied(), self.draft.last().copied());
                let mut pts = self.densified(false);
                self.clear_draft();
                // started or ended exactly on a line end of the same kind:
                // continue that line (its end stays where it was)
                let route_mode = self.route_mode;
                let joined = first.and_then(|p| self.doc.line_end_exact(p, route_mode).map(|e| (e, p, false)))
                    .or_else(|| last.and_then(|p| self.doc.line_end_exact(p, route_mode).map(|e| (e, p, true))));
                if let (Some(((id, at_last), end, reversed)), true) = (joined, pts.len() >= 2) {
                    if reversed {
                        pts.reverse();
                    }
                    pts[0] = end;
                    let kind = if route_mode { "线路" } else { "河流" };
                    if self.edit(&format!("延长{kind}"), |d| d.extend_line(id, at_last, &pts)).is_some() {
                        self.selected_river = Some(id);
                        self.selected_area = None;
                        let name = self.doc.rivers.get(&id).map(|r| r.name.clone()).unwrap_or_default();
                        self.set_status(format!("已延长{kind} {name}"), false);
                    }
                    return;
                }
                if pts.len() >= 2 {
                    let route = self.route_mode.then_some(self.route_style);
                    let (kind, label) = if route.is_some() { ("线路", "绘制线路") } else { ("河流", "绘制河流") };
                    let n = self.doc.rivers.values().filter(|r| r.route.is_some() == route.is_some()).count() + 1;
                    let id = map_core::doc::LineId(self.doc.topo.alloc());
                    if self.edit(label, |d| {
                        d.rivers.insert(id, map_core::doc::LineFeature { name: format!("{kind}{n}"), pts, route, legend: String::new(), show_name: false, hidden: false });
                        Ok(())
                    }).is_some() && route.is_some() {
                        self.style_new(map_core::features::FeatureRef::Route(id));
                        // ready to name and style it
                        self.selected_river = Some(id);
                        self.selected_area = None;
                    }
                }
            }
            _ => {}
        }
    }

    /// Collect a finished background edit (called every frame).
    pub fn poll_bg_edit(&mut self, ctx: &egui::Context) {
        let Some(bg) = &self.bg_edit else { return };
        if !bg.handle.is_finished() {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
            return;
        }
        let bg = self.bg_edit.take().unwrap();
        let label = bg.label;
        let result = bg.handle.join().unwrap_or_else(|_| Err("计算线程意外退出".into()));
        let changed = self.history.generation() != bg.generation || self.pending.is_some()
            || self.doc.year != bg.year || self.doc.active_layer != bg.layer;
        match result {
            Ok(_) if changed => {
                self.draft = bg.pts;
                self.sync_snaps();
                self.set_status(format!("{label}：计算期间地图已被修改，结果作废，草稿已恢复，可回车重新提交"), true);
            }
            Ok((new, r)) => {
                let before = std::mem::replace(&mut self.doc, new);
                self.history.push(label, before, &self.doc);
                self.dirty = true;
                self.selected_face = None;
                self.selected = None;
                if bg.created.is_some() {
                    self.selected = bg.created;
                    self.brush = bg.created;
                }
                let name = |e: Option<map_core::EntityId>| e.and_then(|e| self.doc.entities.get(&e)).map(|x| x.name.clone()).unwrap_or("无主地".into());
                let mut msg = format!("{label}：");
                if r.added_km2 > 0.0 {
                    msg += &format!("新增陆地 {:.0} km²；", r.added_km2);
                }
                for (from, a) in &r.taken {
                    msg += &format!("从{}取得 {:.0} km²；", name(*from), a);
                }
                if r.carved_km2 > 0.0 {
                    msg += &format!("变为水域 {:.0} km²；", r.carved_km2);
                }
                msg += &format!("{:.0} ms", bg.started.elapsed().as_secs_f64() * 1e3);
                self.set_status(msg, false);
            }
            Err(e) => {
                self.draft = bg.pts;
                self.sync_snaps();
                self.set_status(format!("{label}失败：{e}（草稿已恢复，可修改后重新提交）"), true);
            }
        }
    }

    /// Draft, handles, snapping/tracing hints, fill preview and area.
    pub fn draw_draft(&mut self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        let draw_tool = matches!(self.tool, Tool::DrawLand | Tool::DrawLake | Tool::River | Tool::Split | Tool::Redraw | Tool::Hatch);
        if let Some(bg) = &self.bg_edit {
            let t = bg.started.elapsed().as_secs_f32();
            let dots = ".".repeat(1 + (t * 3.0) as usize % 3);
            painter.text(vg.rect.center_top() + Vec2::new(0.0, 28.0), egui::Align2::CENTER_TOP, format!("{}：正在计算{dots} {:.1} s", bg.label, t), egui::FontId::proportional(14.0), Color32::from_rgb(200, 70, 10));
        }
        if !draw_tool {
            return;
        }
        if self.brush_paint && matches!(self.tool, Tool::DrawLand | Tool::DrawLake) {
            return self.draw_brush(painter, vg, hover);
        }
        let accent = Color32::from_rgb(230, 80, 20);
        let snapped_col = Color32::from_rgb(20, 150, 90);
        let screen: Vec<Option<Pos2>> = self.draft.iter().map(|p| vg.to_screen(*p)).collect();
        let closed = matches!(self.tool, Tool::DrawLand | Tool::DrawLake | Tool::Hatch);
        let styled: Option<Vec<Option<Pos2>>> = if self.tool == Tool::Hatch && self.draft.len() >= 2 {
            // a hatched area keeps the placed points and its sides follow the
            // map's grid (as it will be drawn), so preview that outline
            let mut ring = self.styled_draft(vg).unwrap_or_else(|| self.draft.clone());
            ring.push(ring[0]);
            Some(crate::river_ui::densify_ll(&ring).iter().map(|p| vg.to_screen(*p)).collect())
        } else {
            self.styled_draft(vg).map(|s| s.iter().map(|p| vg.to_screen(*p)).collect())
        };
        // the styled outline is what will be committed
        if let Some(st) = &styled {
            let m = st.len();
            let segs = if closed && m >= 3 { m } else { m.saturating_sub(1) };
            for i in 0..segs {
                if let (Some(a), Some(b)) = (st[i], st[(i + 1) % m]) {
                    if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                        painter.line_segment([a, b], Stroke::new(1.6, Color32::from_rgb(120, 40, 10)));
                    }
                }
            }
        }
        // fill preview of the closed shape
        let fill_src = styled.clone().unwrap_or_else(|| screen.clone());
        if closed && self.draft.len() >= 3 && fill_src.iter().all(|s| s.is_some()) {
            let pts: Vec<Pos2> = fill_src.iter().map(|s| s.unwrap()).collect();
            let wraps = pts.windows(2).any(|w| (w[0].x - w[1].x).abs() > vg.rect.width() * 0.5);
            if !wraps {
                let flat: Vec<f64> = pts.iter().flat_map(|p| [p.x as f64, p.y as f64]).collect();
                if let Ok(tri) = earcutr::earcut(&flat, &[], 2) {
                    let fill = self.preview_color();
                    let mut mesh = egui::Mesh::default();
                    for p in &pts {
                        mesh.colored_vertex(*p, fill);
                    }
                    mesh.indices = tri.into_iter().map(|i| i as u32).collect();
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        }
        // segments (closing segment dashed-light), then points
        let n = screen.len();
        let segs = if closed && n >= 3 { n } else { n.saturating_sub(1) };
        for i in 0..segs {
            if let (Some(a), Some(b)) = (screen[i], screen[(i + 1) % n]) {
                if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                    // with a style the straight draft lines are only guides
                    let w = if styled.is_some() { 0.8 } else if i + 1 == n { 1.0 } else { 2.0 };
                    painter.line_segment([a, b], Stroke::new(w, accent));
                }
            }
        }
        // the next segment to the cursor: traced along the boundary or straight
        if let (Some(h), Some(last)) = (hover, screen.last().copied().flatten()) {
            if self.draft_drag.is_none() {
                let hs = vg.to_screen(h);
                let tol = hs.map_or(0.5, |s| vg.deg_per_px(s, GRAB_PX));
                let (q, s) = self.snap(h, tol);
                let ctrl = painter.ctx().input(|i| i.modifiers.command);
                let key = (self.draft.last().copied().unwrap(), q);
                let traced = if s && !ctrl {
                    if self.trace_preview.as_ref().is_none_or(|(k, _)| *k != key) {
                        let path = self.trace_path(q, tol);
                        self.trace_preview = Some((key, path));
                    }
                    self.trace_preview.as_ref().and_then(|(_, p)| p.clone())
                } else {
                    None
                };
                match traced {
                    Some(path) => {
                        let sp: Vec<Option<Pos2>> = path.iter().map(|p| vg.to_screen(*p)).collect();
                        for w in sp.windows(2) {
                            if let (Some(a), Some(b)) = (w[0], w[1]) {
                                if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                                    painter.line_segment([a, b], Stroke::new(3.0, snapped_col));
                                }
                            }
                        }
                    }
                    None => {
                        if let Some(b) = vg.to_screen(q) {
                            if (last.x - b.x).abs() < vg.rect.width() * 0.5 {
                                painter.line_segment([last, b], Stroke::new(1.5, accent.gamma_multiply(0.7)));
                            }
                        }
                    }
                }
            }
        }
        for (i, p) in screen.iter().enumerate() {
            let Some(p) = p else { continue };
            // traced and freehand runs have many points: draw only real corners
            let corner = i == 0 || i + 1 == n || !(self.is_snapped(i) && self.is_snapped(i.saturating_sub(1)) && self.is_snapped((i + 1).min(n - 1)));
            if corner {
                painter.circle_filled(*p, 4.0, if self.is_snapped(i) { snapped_col } else { accent });
            }
        }
        for (_, m) in self.midpoints(vg) {
            painter.circle_stroke(m, 3.5, Stroke::new(1.2, accent));
        }
        if self.tool == Tool::Redraw {
            self.draw_redraw_preview(painter, vg, hover);
        }
        // snapping indicator under the cursor
        if let Some(h) = hover {
            if let Some(hs) = vg.to_screen(h) {
                if self.tool != Tool::River {
                    if let Some(s) = self.doc.topo.snap_boundary(h, vg.deg_per_px(hs, GRAB_PX)).and_then(|s| vg.to_screen(s)) {
                        painter.circle_stroke(s, 6.0, Stroke::new(2.0, snapped_col));
                    }
                } else if let Some((q, what)) = self.doc.line_snap(h, vg.deg_per_px(hs, GRAB_PX), self.route_mode) {
                    // what a click here would join or continue
                    if let Some(s) = vg.to_screen(q) {
                        painter.circle_stroke(s, 6.0, Stroke::new(2.0, snapped_col));
                        let starting = self.draft.is_empty();
                        let hint = match what {
                            map_core::lines::LineSnap::End(..) if starting => "从此处延长",
                            map_core::lines::LineSnap::End(..) => "接到这条线的端点（完成后延长它）",
                            map_core::lines::LineSnap::City => "城市",
                            map_core::lines::LineSnap::On(_) => "接到这条线上",
                        };
                        painter.text(s + Vec2::new(9.0, -9.0), egui::Align2::LEFT_BOTTOM, hint, egui::FontId::proportional(12.0), snapped_col);
                    }
                }
            }
        }
        // area and what will happen
        if closed && self.draft.len() >= 3 {
            let ring = self.styled_cache.as_ref().filter(|_| styled.is_some()).map(|(_, p)| p.clone()).unwrap_or_else(|| self.draft.clone());
        let planar = geom::ring_to_planar(&ring);
            let area = geom::spherical_area(&planar, self.doc.radius()).abs();
            let what = match (self.tool, self.land_intent()) {
                (Tool::DrawLake, _) => "变为水域".to_string(),
                (_, Ok(DrawIntent::AddLand)) => "新增陆地（不动已有陆地）".into(),
                (_, Ok(DrawIntent::Expand(x))) => format!("扩张：{}", self.doc.entities.get(&x).map_or("", |e| e.name.as_str())),
                (_, Ok(DrawIntent::Subdivide { parent, .. })) => format!("划分直属陆地 · {}", self.doc.entities.get(&parent).map_or("", |e| e.name.as_str())),
                (_, Ok(DrawIntent::Take(_))) if self.draw_mode == DrawMode::NewCountry => format!("新建国家：{}", self.new_country_name()),
                (_, Ok(DrawIntent::Take(x))) => format!("夺取：{}", self.doc.entities.get(&x).map_or("", |e| e.name.as_str())),
                (_, Err(e)) => e.to_string(),
                _ => String::new(),
            };
            let year = self.doc.year.map(|y| format!("（{y} 年起）")).unwrap_or_default();
            if let Some(at) = screen.last().copied().flatten() {
                let text = format!("约 {} km² · {what}{year}\n回车完成，Esc 取消", map_core::doc::format_num(area.round()));
                painter.text(at + Vec2::new(12.0, 12.0), egui::Align2::LEFT_TOP, text, egui::FontId::proportional(12.0), Color32::from_gray(20));
            }
        }
    }

    /// While redrawing a border: when the cursor is on a boundary, show
    /// the land that would change hands and who would get it.
    fn draw_redraw_preview(&mut self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        let Some(h) = hover.filter(|_| !self.draft.is_empty() && self.draft_drag.is_none()) else { return };
        let Some(hs) = vg.to_screen(h) else { return };
        let Some(end) = self.doc.topo.snap_boundary(h, vg.deg_per_px(hs, GRAB_PX)) else { return };
        let mut line = self.draft.clone();
        line.push(end);
        if self.redraw_preview.as_ref().is_none_or(|(k, _)| *k != line) {
            // as it will be committed, so the area shown is the area moved
            let pts = screen_densified(vg, line.clone(), false);
            let plan = map_core::redraw::plan(&self.doc, &pts, vg.deg_per_px(hs, 2.0));
            self.redraw_preview = Some((line, plan));
        }
        let Some((_, plan)) = &self.redraw_preview else { return };
        self.paint_plan(painter, vg, plan, hs, "单击此处完成");
    }

    /// Editing vertices with a year selected: a drag (or a deletion) is a
    /// redraw of that stretch of border from the year on; the shared
    /// geometry is not touched until the territory change is applied.
    pub fn dated_vertex_input(&mut self, resp: &egui::Response, vg: &ViewGeom, p: LonLat, tol: f64) {
        use egui::PointerButton::{Primary, Secondary};
        use map_core::redraw::{self, DragFrom};
        let from_at = |doc: &Document, q: LonLat| -> Option<DragFrom> {
            if let Some((v, _)) = doc.topo.nearest_vertex(q, tol) {
                Some(DragFrom::Vertex(v))
            } else {
                doc.topo.nearest_on_edge(q, tol).map(|(edge, seg, _)| DragFrom::Segment { edge, seg })
            }
        };
        if resp.drag_started_by(Primary) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            let q = origin.and_then(|o| vg.to_lonlat(o)).unwrap_or(p);
            self.dated_drag = from_at(&self.doc, q).map(|f| (f, None));
        }
        if resp.dragged_by(Primary) {
            if let Some((from, last)) = &self.dated_drag {
                if last.as_ref().is_none_or(|(at, _)| *at != p) {
                    let plan = redraw::drag_line(&self.doc.topo, *from, p).and_then(|l| redraw::plan(&self.doc, &l, tol * 0.25));
                    let from = *from;
                    self.dated_drag = Some((from, Some((p, plan))));
                }
            }
        }
        if resp.drag_stopped() {
            if let Some((_, Some((_, plan)))) = self.dated_drag.take() {
                self.apply_plan("移动国界", plan);
            }
        }
        if resp.clicked_by(Secondary) {
            // deleting a point straightens the border there: also a dated change
            if let Some((v, _)) = self.doc.topo.nearest_vertex(p, tol) {
                let plan = redraw::drag_line(&self.doc.topo, DragFrom::Vertex(v), p).and_then(|l| redraw::plan(&self.doc, &[l[0], l[2]], tol * 0.25));
                self.apply_plan("删除国界顶点", plan);
            }
        }
    }

    fn apply_plan(&mut self, label: &str, plan: Result<map_core::redraw::Plan, String>) {
        let plan = match plan {
            Ok(p) => p,
            Err(e) => return self.set_status(format!("{label}：{e}"), true),
        };
        let year = self.doc.year.map(|y| format!("（{y} 年起）")).unwrap_or_default();
        let name = |d: &Document, e: Option<map_core::EntityId>, none: &str| e.and_then(|e| d.entities.get(&e)).map_or(none.to_string(), |x| x.name.clone());
        let what = match plan.gainer {
            Some(g) => format!("{}从{}取得", name(&self.doc, Some(g), ""), name(&self.doc, plan.loser, "海域")),
            None => format!("{}的土地变为水域", name(&self.doc, plan.loser, "无主地")),
        };
        if let Some(r) = self.edit(label, |d| {
            let (n, r) = map_core::redraw::apply(d, &plan)?;
            *d = n;
            Ok(r)
        }) {
            let km2 = r.taken.iter().map(|(_, a)| a).sum::<f64>() + r.added_km2 + r.carved_km2;
            self.set_status(format!("{label}{year}：{what} {:.0} km²", km2), false);
        }
    }

    /// The land a redraw plan would move, filled in the gainer's colour,
    /// with the old boundary and a label at `hs`.
    pub fn paint_plan(&self, painter: &egui::Painter, vg: &ViewGeom, plan: &Result<map_core::redraw::Plan, String>, hs: Pos2, tail: &str) {
        let text = match plan {
            Ok(p) => {
                let pts: Vec<Option<Pos2>> = p.ring.iter().map(|q| vg.to_screen(*q)).collect();
                let wraps = pts.windows(2).any(|w| matches!((w[0], w[1]), (Some(a), Some(b)) if (a.x - b.x).abs() > vg.rect.width() * 0.5));
                let c = match p.gainer {
                    Some(g) => self.doc.entities.get(&g).map_or(self.doc.style.land, |e| e.color),
                    None => self.doc.style.lake,
                };
                if pts.iter().all(|q| q.is_some()) && !wraps {
                    let pts: Vec<Pos2> = pts.into_iter().flatten().collect();
                    let flat: Vec<f64> = pts.iter().flat_map(|q| [q.x as f64, q.y as f64]).collect();
                    if let Ok(tri) = earcutr::earcut(&flat, &[], 2) {
                        let mut mesh = egui::Mesh::default();
                        for q in &pts {
                            mesh.colored_vertex(*q, Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 150));
                        }
                        mesh.indices = tri.into_iter().map(|i| i as u32).collect();
                        painter.add(egui::Shape::mesh(mesh));
                    }
                }
                let old: Vec<Option<Pos2>> = p.old.iter().map(|q| vg.to_screen(*q)).collect();
                for w in old.windows(2) {
                    if let (Some(a), Some(b)) = (w[0], w[1]) {
                        if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                            painter.line_segment([a, b], Stroke::new(1.2, Color32::from_gray(90)));
                        }
                    }
                }
                let name = |e: Option<map_core::EntityId>, none: &str| e.and_then(|e| self.doc.entities.get(&e)).map_or(none.to_string(), |x| x.name.clone());
                let area = geom::spherical_area(&geom::ring_to_planar(&p.ring), self.doc.radius()).abs();
                let what = match p.gainer {
                    Some(g) => format!("从{}移交给{}", name(p.loser, "海域"), name(Some(g), "")),
                    None => format!("{}的土地变为水域", name(p.loser, "无主地")),
                };
                let year = self.doc.year.map(|y| format!("（{y} 年起，更早的地图不变）")).unwrap_or_else(|| "（无年份：当前地图）".into());
                format!("约 {} km² · {what}{year}\n{tail}", map_core::doc::format_num(area.round()))
            }
            Err(e) => e.clone(),
        };
        painter.text(hs + Vec2::new(14.0, 14.0), egui::Align2::LEFT_TOP, text, egui::FontId::proportional(12.0), Color32::from_gray(20));
    }

    pub fn preview_fill(&self) -> Color32 {
        self.preview_color()
    }

    fn preview_color(&self) -> Color32 {
        let c = match (self.tool, self.land_intent()) {
            (Tool::DrawLake, _) => self.doc.style.lake,
            (_, Ok(DrawIntent::Expand(x) | DrawIntent::Take(x))) => self.doc.entities.get(&x).map_or(self.doc.style.land, |e| e.color),
            _ => self.doc.style.land,
        };
        Color32::from_rgba_unmultiplied(c[0], c[1], c[2], 110)
    }
}
