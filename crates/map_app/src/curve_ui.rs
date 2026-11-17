//! Curves in the interface: the pen tool, which draws rivers, routes and
//! hatched areas as Bézier curves (click for a corner, press and drag for
//! a smooth point with handles), and editing the anchors and handles of a
//! curve that is selected (select and pen tools) or under the pointer
//! (vertex tool). The curves themselves are `map_core::curve`.

use crate::app::App;
use crate::view::{Tool, ViewGeom};
use eframe::egui::{self, Color32, Pos2, RichText, Stroke};
use map_core::curve::{Curve, CurveNode, Handle};
use map_core::doc::LineId;
use map_core::features::FeatureRef;
use map_core::geom::LonLat;

const ACCENT: Color32 = Color32::from_rgb(40, 110, 200);
/// How near (screen points) the pointer must be to take an anchor or handle.
const GRIP_PX: f32 = 8.0;

/// What the pen draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenTarget {
    River,
    Route,
    Area,
}

impl PenTarget {
    pub fn name(self) -> &'static str {
        match self {
            PenTarget::River => "河流",
            PenTarget::Route => "线路",
            PenTarget::Area => "斜线区域",
        }
    }
}

/// A part of a curve that can be dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Grip {
    Anchor(usize),
    Handle(usize, Handle),
}

/// An anchor or handle being dragged: which curve, which part.
pub struct CurveDrag {
    id: LineId,
    grip: Grip,
}

fn handle_of(n: &CurveNode, h: Handle) -> Option<LonLat> {
    match h {
        Handle::In => n.h_in,
        Handle::Out => n.h_out,
    }
}

impl App {
    /// The curve whose anchors and handles are shown and can be dragged:
    /// the selected line or area when it is a curve, else (vertex and pen
    /// tools) the curve last worked on.
    pub fn curve_target(&self) -> Option<LineId> {
        if !matches!(self.tool, Tool::Select | Tool::Pen | Tool::EditVertex) {
            return None;
        }
        let ok = |id: &LineId| {
            self.doc.curves.contains_key(id) && {
                let f = if self.doc.areas.contains_key(id) { FeatureRef::Area(*id) } else { FeatureRef::River(*id) };
                self.doc.pickable(f)
            }
        };
        let selected = self.selected_river.or(self.selected_area).filter(|_| self.picked.len() <= 1);
        selected.filter(ok).or_else(|| self.curve_focus.filter(|_| self.tool != Tool::Select).filter(ok))
    }

    /// The anchor or handle of curve `id` nearest to screen point `o`
    /// (handles first: they may lie on their anchor's line).
    fn grip_at(&self, id: LineId, o: Pos2, vg: &ViewGeom) -> Option<Grip> {
        let c = self.doc.curves.get(&id)?;
        let near = |p: LonLat| vg.to_screen(p).map(|s| s.distance(o)).filter(|d| *d <= GRIP_PX);
        let mut best: Option<(f32, Grip)> = None;
        let mut take = |d: Option<f32>, g: Grip| {
            if let Some(d) = d {
                if best.is_none_or(|b| d < b.0) {
                    best = Some((d, g));
                }
            }
        };
        for (i, n) in c.nodes.iter().enumerate() {
            take(near(n.at).map(|d| d - 1.0), Grip::Anchor(i));
            for h in [Handle::In, Handle::Out] {
                if let Some(p) = handle_of(n, h) {
                    take(near(p).map(|d| d - 2.0), Grip::Handle(i, h));
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The point of curve `id` nearest to screen point `o`: segment and
    /// parameter, if within reach.
    fn curve_point_at(&self, id: LineId, o: Pos2, vg: &ViewGeom) -> Option<(usize, f64)> {
        let c = self.doc.curves.get(&id)?;
        let mut best: Option<(f32, usize, f64)> = None;
        for i in 0..c.segments() {
            for k in 0..=64 {
                let t = k as f64 / 64.0;
                if let Some(s) = vg.to_screen(c.point(i, t)) {
                    let d = s.distance(o);
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, i, t));
                    }
                }
            }
        }
        best.filter(|b| b.0 <= GRIP_PX).map(|b| (b.1, b.2))
    }

    /// The curved line or area with an anchor nearest to `o` (vertex tool).
    fn curve_with_anchor_at(&self, o: Pos2, vg: &ViewGeom) -> Option<LineId> {
        let mut best: Option<(f32, LineId)> = None;
        for (id, c) in self.doc.curves.iter() {
            let f = if self.doc.areas.contains_key(id) { FeatureRef::Area(*id) } else { FeatureRef::River(*id) };
            if !self.doc.pickable(f) {
                continue;
            }
            for n in &c.nodes {
                if let Some(d) = vg.to_screen(n.at).map(|s| s.distance(o)).filter(|d| *d <= GRIP_PX) {
                    if best.is_none_or(|b| d < b.0) {
                        best = Some((d, *id));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Drags of anchors and handles. Called before the selection's own
    /// move-by-drag, so a press on an anchor moves the anchor, not the
    /// whole line. Returns whether it took the drag.
    pub fn curve_drag(&mut self, resp: &egui::Response, vg: &ViewGeom) -> bool {
        use egui::PointerButton::Primary;
        if resp.drag_started_by(Primary) && !self.space_pan && (self.tool != Tool::Pen || self.pen.is_empty()) {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            if let Some(o) = origin {
                let mut found = self.curve_target().and_then(|id| self.grip_at(id, o, vg).map(|g| (id, g)));
                if found.is_none() && self.tool == Tool::EditVertex {
                    found = self.curve_with_anchor_at(o, vg).and_then(|id| self.grip_at(id, o, vg).map(|g| (id, g)));
                }
                if let Some((id, grip)) = found {
                    self.curve_focus = Some(id);
                    self.begin_pending(if matches!(grip, Grip::Anchor(_)) { "移动锚点" } else { "调整控制柄" });
                    // Alt on a handle: the two handles part (a corner)
                    if let (Grip::Handle(i, _), true) = (grip, self.press_mods.alt) {
                        let _ = self.doc.edit_curve(id, |c| {
                            c.nodes[i].smooth = false;
                            Ok(())
                        });
                    }
                    self.curve_drag = Some(CurveDrag { id, grip });
                }
            }
        }
        let Some(d) = &self.curve_drag else { return false };
        let (id, grip) = (d.id, d.grip);
        if resp.dragged_by(Primary) {
            if let Some(p) = resp.interact_pointer_pos().and_then(|p| vg.to_lonlat(p)) {
                let r = self.doc.edit_curve(id, |c| {
                    match grip {
                        Grip::Anchor(i) => c.move_node(i, p),
                        Grip::Handle(i, h) => c.move_handle(i, h, p),
                    }
                    Ok(())
                });
                if r.is_ok() {
                    if let Some(pe) = &mut self.pending {
                        pe.changed = true;
                    }
                }
            }
        }
        if resp.drag_stopped() || !resp.ctx.input(|i| i.pointer.primary_down()) {
            self.curve_drag = None;
            self.end_pending();
        }
        true
    }

    /// Clicks on the anchors of the curve being edited: double-click an
    /// anchor to make it smooth or a corner, double-click the line to add
    /// one, right-click an anchor to remove it. Returns whether it took the click.
    pub fn curve_click(&mut self, resp: &egui::Response, vg: &ViewGeom) -> bool {
        let Some(o) = resp.interact_pointer_pos() else { return false };
        let target = self.curve_target().or_else(|| (self.tool == Tool::EditVertex).then(|| self.curve_with_anchor_at(o, vg)).flatten());
        let Some(id) = target else { return false };
        let grip = self.grip_at(id, o, vg);
        if resp.double_clicked() {
            if let Some(Grip::Anchor(i)) = grip {
                let smooth = self.doc.curves[&id].nodes[i].h_in.is_some() || self.doc.curves[&id].nodes[i].h_out.is_some();
                self.edit(if smooth { "锚点改为尖角" } else { "锚点改为平滑" }, |d| {
                    d.edit_curve(id, |c| {
                        if smooth {
                            c.make_corner(i)
                        } else {
                            c.make_smooth(i)
                        }
                        Ok(())
                    })
                });
                return true;
            }
            if let Some((seg, t)) = self.curve_point_at(id, o, vg) {
                self.edit("添加锚点", |d| {
                    d.edit_curve(id, |c| {
                        c.insert_node(seg, t);
                        Ok(())
                    })
                });
                return true;
            }
        }
        if resp.secondary_clicked() {
            if let Some(Grip::Anchor(i)) = grip {
                self.edit("删除锚点", |d| d.edit_curve(id, |c| c.remove_node(i)));
                return true;
            }
        }
        // a single click on an anchor or handle only picks it (no deselecting)
        resp.clicked() && grip.is_some()
    }

    /// The pen: click for a corner, press and drag for a smooth anchor
    /// (the drag pulls its handles); click the first anchor to close an
    /// area; double-click or Enter to finish, right-click or Backspace to
    /// take back the last anchor, Esc to give up. With nothing drawn, a
    /// click on a line or area selects it, so its curve can be edited.
    pub fn pen_input(&mut self, resp: &egui::Response, vg: &ViewGeom) {
        use egui::PointerButton::{Primary, Secondary};
        let at = |p: Option<Pos2>| p.and_then(|p| vg.to_lonlat(p));
        let near_first = |s: &App, o: Pos2| s.pen.first().and_then(|n| vg.to_screen(n.at)).is_some_and(|f| f.distance(o) <= GRIP_PX);
        let closes = self.pen_target == PenTarget::Area && self.pen.len() >= 3;
        if resp.drag_started_by(Primary) && self.curve_drag.is_none() {
            let origin = resp.ctx.input(|i| i.pointer.press_origin());
            if let (Some(o), Some(p)) = (origin, at(origin)) {
                if closes && near_first(self, o) {
                    return self.pen_finish(true);
                }
                self.pen.push(CurveNode::corner(p));
                self.pen_pull = true;
            }
        }
        if self.pen_pull {
            if resp.dragged_by(Primary) {
                if let (Some(p), Some(last)) = (at(resp.interact_pointer_pos()), self.pen.len().checked_sub(1)) {
                    let mut c = Curve { nodes: std::mem::take(&mut self.pen), closed: false };
                    c.pull_handles(last, p);
                    self.pen = c.nodes;
                }
            }
            if resp.drag_stopped() {
                self.pen_pull = false;
            }
            return;
        }
        if resp.double_clicked() {
            return self.pen_finish(false);
        }
        if resp.clicked_by(Primary) {
            let o = resp.interact_pointer_pos();
            if self.pen.is_empty() {
                // a line or area under the pointer: select it to edit its curve
                if let Some(f) = o.and_then(|o| self.feature_at(o, vg)).filter(|f| matches!(f, FeatureRef::River(_) | FeatureRef::Route(_) | FeatureRef::Area(_))) {
                    self.set_selection(vec![f]);
                    self.curve_focus = Some(f.id());
                    self.tab = crate::panels::Tab::Features;
                    let curved = self.doc.curves.contains_key(&f.id());
                    self.set_status(
                        if curved {
                            "已选中曲线：拖动锚点与控制柄修改；双击锚点切换平滑/尖角，双击线上添加锚点，右键锚点删除"
                        } else {
                            "这条线（区域）还不是曲线：在右侧面板中点“转为曲线”后即可拖动锚点与控制柄"
                        },
                        false,
                    );
                    return;
                }
            }
            if let (Some(o), Some(p)) = (o, at(o)) {
                if closes && near_first(self, o) {
                    return self.pen_finish(true);
                }
                self.pen.push(CurveNode::corner(p));
            }
        }
        if resp.clicked_by(Secondary) {
            self.pen.pop();
        }
    }

    /// Finish what the pen drew: a line (open) or an area (closed).
    pub fn pen_finish(&mut self, closing: bool) {
        self.pen_pull = false;
        let mut nodes = std::mem::take(&mut self.pen);
        // a double-click puts two anchors on one spot: one is enough
        while nodes.len() >= 2 && map_core::geom::geo_dist_deg(nodes[nodes.len() - 1].at, nodes[nodes.len() - 2].at) < 1e-9 {
            let last = nodes.pop().unwrap();
            let keep = nodes.last_mut().unwrap();
            keep.h_out = keep.h_out.or(last.h_out);
        }
        let target = self.pen_target;
        let area = target == PenTarget::Area;
        if nodes.len() < if area { 3 } else { 2 } {
            // (a lone anchor from a stray double-click is just dropped)
            if nodes.len() > 1 {
                self.set_status(if area { "区域至少需要三个锚点" } else { "线至少需要两个锚点" }, true);
            }
            return;
        }
        // (an area always closes, clicked on its first anchor or not)
        let _ = closing;
        let c = Curve { nodes, closed: area };
        let route = (target == PenTarget::Route).then_some(self.route_style);
        let label = match target {
            PenTarget::River => "钢笔绘制河流",
            PenTarget::Route => "钢笔绘制线路",
            PenTarget::Area => "钢笔绘制斜线区域",
        };
        let made = self.edit(label, |d| if area { d.add_curve_area(c) } else { d.add_curve_line(c, route) });
        if let Some(id) = made {
            let f = match target {
                PenTarget::River => FeatureRef::River(id),
                PenTarget::Route => FeatureRef::Route(id),
                PenTarget::Area => FeatureRef::Area(id),
            };
            self.style_new(f);
            self.set_selection(vec![f]);
            self.curve_focus = Some(id);
            self.tab = crate::panels::Tab::Features;
            self.set_status(format!("{label}：完成。拖动锚点与控制柄可修改；在右侧面板命名与设置样式"), false);
        }
    }

    /// The pen's drawing in progress, the anchors and handles of the curve
    /// being edited, and (vertex tool) the anchors of curves near the pointer.
    pub fn draw_curve_overlay(&self, painter: &egui::Painter, vg: &ViewGeom, hover: Option<LonLat>) {
        let poly = |pts: &[LonLat], stroke: Stroke| {
            let s: Vec<Option<Pos2>> = crate::river_ui::densify_ll(pts).into_iter().map(|p| vg.to_screen(p)).collect();
            for w in s.windows(2) {
                if let (Some(a), Some(b)) = (w[0], w[1]) {
                    if (a.x - b.x).abs() < vg.rect.width() * 0.5 {
                        painter.line_segment([a, b], stroke);
                    }
                }
            }
        };
        let anchor = |p: LonLat, filled: bool| {
            if let Some(s) = vg.to_screen(p) {
                let r = egui::Rect::from_center_size(s, egui::vec2(7.0, 7.0));
                painter.rect_filled(r, 0.0, if filled { ACCENT } else { Color32::WHITE });
                painter.rect_stroke(r, 0.0, Stroke::new(1.2, ACCENT), egui::StrokeKind::Middle);
            }
        };
        let handles = |n: &CurveNode| {
            for h in [n.h_in, n.h_out].into_iter().flatten() {
                if let (Some(a), Some(b)) = (vg.to_screen(n.at), vg.to_screen(h)) {
                    painter.line_segment([a, b], Stroke::new(1.0, ACCENT));
                    painter.circle_filled(b, 3.5, ACCENT);
                }
            }
        };
        // the pen's curve so far, and the next segment to the pointer
        if self.tool == Tool::Pen && !self.pen.is_empty() {
            let c = Curve { nodes: self.pen.clone(), closed: false };
            poly(&c.flatten(), Stroke::new(2.0, ACCENT));
            if let (Some(h), false) = (hover, self.pen_pull) {
                let last = *self.pen.last().unwrap();
                let next = Curve { nodes: vec![last, CurveNode::corner(h)], closed: false };
                poly(&next.flatten(), Stroke::new(1.0, ACCENT.gamma_multiply(0.7)));
            }
            for (i, n) in self.pen.iter().enumerate() {
                anchor(n.at, i + 1 == self.pen.len());
            }
            if let Some(n) = self.pen.last() {
                handles(n);
            }
            // the first anchor, ringed when a click there would close the area
            let first = self.pen[0].at;
            let close = self.pen_target == PenTarget::Area && self.pen.len() >= 3 && hover.and_then(|h| vg.to_screen(h)).zip(vg.to_screen(first)).is_some_and(|(h, f)| h.distance(f) <= GRIP_PX);
            if let (true, Some(s)) = (close, vg.to_screen(first)) {
                painter.circle_stroke(s, 8.0, Stroke::new(2.0, ACCENT));
            }
        }
        // the curve being edited
        if let Some(c) = self.curve_target().and_then(|id| self.doc.curves.get(&id)) {
            poly(&c.flatten().into_iter().chain(c.closed.then(|| c.nodes[0].at)).collect::<Vec<_>>(), Stroke::new(1.0, ACCENT));
            for n in &c.nodes {
                handles(n);
            }
            let active = self.curve_drag.as_ref().map(|d| d.grip);
            for (i, n) in c.nodes.iter().enumerate() {
                anchor(n.at, active == Some(Grip::Anchor(i)));
            }
        }
        // the vertex tool: anchors of curves near the pointer
        if self.tool == Tool::EditVertex {
            let Some(h) = hover else { return };
            let Some(hs) = vg.to_screen(h) else { return };
            for (id, c) in self.doc.curves.iter() {
                if Some(*id) == self.curve_target() {
                    continue;
                }
                for n in &c.nodes {
                    if vg.to_screen(n.at).is_some_and(|s| s.distance(hs) < 160.0) {
                        anchor(n.at, false);
                    }
                }
            }
        }
    }

    /// The curve part of a selected line's or area's panel.
    pub fn curve_panel(&mut self, ui: &mut egui::Ui, id: LineId) {
        let area = self.doc.areas.contains_key(&id);
        match self.doc.curves.get(&id) {
            Some(c) => {
                let (n, smooth) = (c.nodes.len(), c.nodes.iter().filter(|n| n.h_in.is_some() || n.h_out.is_some()).count());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("曲线").strong());
                    ui.weak(format!("{n} 个锚点，其中 {smooth} 个带控制柄"));
                })
                .response
                .on_hover_text("拖动锚点与控制柄修改（选择、钢笔或编辑顶点工具）；按住 Alt 拖动控制柄可把平滑点拆成尖角；双击锚点切换平滑/尖角；双击线上添加锚点；右键锚点删除");
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("全部平滑").on_hover_text("每个锚点都改为平滑点（控制柄沿前后锚点方向）").clicked() {
                        self.edit("曲线全部平滑", |d| {
                            d.edit_curve(id, |c| {
                                for i in 0..c.nodes.len() {
                                    c.make_smooth(i);
                                }
                                Ok(())
                            })
                        });
                    }
                    if ui.small_button("全部尖角").on_hover_text("去掉所有控制柄：锚点之间是直线").clicked() {
                        self.edit("曲线全部尖角", |d| {
                            d.edit_curve(id, |c| {
                                for i in 0..c.nodes.len() {
                                    c.make_corner(i);
                                }
                                Ok(())
                            })
                        });
                    }
                    if ui.small_button("转为折线").on_hover_text("不再是曲线，保持现在的形状（由许多短直线组成）；之后可像普通的线一样逐点编辑").clicked() {
                        self.edit("曲线转为折线", |d| d.drop_curve(id, true));
                    }
                });
            }
            None => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("曲线").strong());
                    let n = if area { self.doc.areas[&id].ring.len() } else { self.doc.rivers[&id].pts.len() };
                    if ui.small_button("转为平滑曲线").on_hover_text(format!("以现有的 {n} 个点为锚点，生成经过它们的平滑曲线")).clicked() {
                        self.edit("转为平滑曲线", |d| d.curve_from_points(id, true));
                    }
                    if ui.small_button("转为曲线（保持形状）").on_hover_text("每个点都成为尖角锚点，形状不变；之后可从锚点拉出控制柄").clicked() {
                        self.edit("转为曲线", |d| d.curve_from_points(id, false));
                    }
                });
            }
        }
    }

    /// The pen tool's options (in the options bar).
    pub fn pen_options(&mut self, ui: &mut egui::Ui) {
        ui.label("画");
        for t in [PenTarget::River, PenTarget::Route, PenTarget::Area] {
            if ui.selectable_value(&mut self.pen_target, t, t.name()).changed() {
                self.pen.clear();
            }
        }
        if self.pen_target == PenTarget::Route {
            ui.menu_button("线路样式…", |ui| {
                let mut st = self.route_style;
                crate::river_ui::route_style_ui(ui, &mut st);
                self.route_style = st;
            });
        }
        if !self.pen.is_empty() {
            ui.separator();
            ui.weak(format!("{} 个锚点", self.pen.len()));
        }
    }
}
