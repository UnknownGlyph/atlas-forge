//! Curves: rivers, routes and hatched areas drawn with Bézier curves
//! (the pen of drawing programs) instead of straight segments.
//!
//! A curve is a list of anchor points, each with up to two handles (where
//! the curve leaves the anchor and where it arrives at it). Between two
//! anchors the line is a cubic Bézier curve in the lon/lat plane, the
//! plane every other line of the map is drawn in; a segment whose two
//! handles are both missing is a straight segment, as before.
//!
//! The curve is the shape's source and its points are derived from it: a
//! line's `pts` (an area's `ring`) is the curve flattened into short
//! straight pieces. Everything that reads points (drawing, export, length,
//! picking, snapping, names along the line) therefore works unchanged.
//! Curves are kept beside the features in `Document::curves`, by feature
//! id, so features and files without curves are exactly as they were. An
//! edit that changes the points directly (dragging one of the flattened
//! points, restyling) makes the line an ordinary one again, keeping its
//! current shape; moving, turning, scaling and copying carry the curve.

use crate::doc::{Document, LineId};
use crate::geom::{lon_delta, wrap_lon, LonLat};
use crate::topo::TResult;
use serde::{Deserialize, Serialize};

/// One anchor of a curve, with the handles that shape the curve on its
/// two sides (absolute positions; none: the curve comes straight in or
/// goes straight out there).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurveNode {
    pub at: LonLat,
    /// Shapes the segment arriving from the previous anchor.
    #[serde(default)]
    pub h_in: Option<LonLat>,
    /// Shapes the segment leaving to the next anchor.
    #[serde(default)]
    pub h_out: Option<LonLat>,
    /// The two handles stay in one line through the anchor (a smooth
    /// point); otherwise they move independently (a corner).
    #[serde(default)]
    pub smooth: bool,
}

impl CurveNode {
    pub fn corner(at: LonLat) -> Self {
        Self { at, h_in: None, h_out: None, smooth: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Curve {
    pub nodes: Vec<CurveNode>,
    /// The last anchor joins the first (an area's outline).
    #[serde(default)]
    pub closed: bool,
}

/// Which handle of an anchor.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    In,
    Out,
}

/// `q` with its longitude made continuous with `near` (within 180°).
fn near(q: LonLat, near: [f64; 2]) -> [f64; 2] {
    [near[0] + lon_delta(near[0], q.lon), q.lat]
}

fn ll(p: [f64; 2]) -> LonLat {
    LonLat::new(wrap_lon(p[0]), p[1].clamp(-89.999, 89.999))
}

fn lerp(a: [f64; 2], b: [f64; 2], t: f64) -> [f64; 2] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

fn dist(a: [f64; 2], b: [f64; 2]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}

/// Distance of `p` from the line through `a` and `b`.
fn off_line(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let d = dist(a, b);
    if d < 1e-12 {
        return dist(p, a);
    }
    ((b[0] - a[0]) * (a[1] - p[1]) - (a[0] - p[0]) * (b[1] - a[1])).abs() / d
}

/// The four control points of a cubic segment, unwrapped around `a`.
type Cubic = [[f64; 2]; 4];

/// Split a cubic at `t` (de Casteljau): the two halves.
fn split(c: &Cubic, t: f64) -> (Cubic, Cubic) {
    let (a, b, cc) = (lerp(c[0], c[1], t), lerp(c[1], c[2], t), lerp(c[2], c[3], t));
    let (d, e) = (lerp(a, b, t), lerp(b, cc, t));
    let m = lerp(d, e, t);
    ([c[0], a, d, m], [m, e, cc, c[3]])
}

/// Points along a cubic (without its start), close enough that the
/// straight pieces between them are within `tol` degrees of the curve.
fn flatten_cubic(c: &Cubic, tol: f64, depth: u32, out: &mut Vec<[f64; 2]>) {
    let flat = off_line(c[1], c[0], c[3]).max(off_line(c[2], c[0], c[3]));
    if flat <= tol || depth >= 10 {
        out.push(c[3]);
        return;
    }
    let (l, r) = split(c, 0.5);
    flatten_cubic(&l, tol, depth + 1, out);
    flatten_cubic(&r, tol, depth + 1, out);
}

impl Curve {
    /// A curve through the points with every anchor a corner: the same
    /// shape as the polyline.
    pub fn corners(pts: &[LonLat], closed: bool) -> Self {
        Self { nodes: pts.iter().map(|p| CurveNode::corner(*p)).collect(), closed }
    }

    /// A smooth curve through the points (each anchor gets handles along
    /// the direction from its previous to its next point, a third of the
    /// way to each: a Catmull-Rom spline). The ends of an open curve keep
    /// a handle towards their neighbour only.
    pub fn smooth_through(pts: &[LonLat], closed: bool) -> Self {
        let mut c = Self::corners(pts, closed);
        for i in 0..c.nodes.len() {
            c.make_smooth(i);
        }
        c
    }

    /// Number of segments (an open curve has one fewer than anchors).
    pub fn segments(&self) -> usize {
        let n = self.nodes.len();
        if self.closed {
            n
        } else {
            n.saturating_sub(1)
        }
    }

    /// The control points of segment `i` (from anchor `i` to the next),
    /// unwrapped around its start; a missing handle is its anchor.
    pub fn cubic(&self, i: usize) -> Cubic {
        let n = self.nodes.len();
        let (a, b) = (self.nodes[i], self.nodes[(i + 1) % n]);
        let p0 = [a.at.lon, a.at.lat];
        let p3 = near(b.at, p0);
        let p1 = a.h_out.map_or(p0, |h| near(h, p0));
        let p2 = b.h_in.map_or(p3, |h| near(h, p3));
        [p0, p1, p2, p3]
    }

    /// Is segment `i` straight (no handle on either side)?
    pub fn is_straight(&self, i: usize) -> bool {
        let n = self.nodes.len();
        self.nodes[i].h_out.is_none() && self.nodes[(i + 1) % n].h_in.is_none()
    }

    /// The curve as points: every anchor, and between them as many points
    /// as it takes to follow each curved segment closely (straight
    /// segments add none). Not closed (an area's ring: the last point
    /// joins the first).
    pub fn flatten(&self) -> Vec<LonLat> {
        let Some(first) = self.nodes.first() else { return vec![] };
        let mut out = vec![first.at];
        for i in 0..self.segments() {
            let c = self.cubic(i);
            let mut v = Vec::new();
            if self.is_straight(i) {
                v.push(c[3]);
            } else {
                // finer for small curves, never coarser than 0.02°
                let size = dist(c[0], c[1]) + dist(c[1], c[2]) + dist(c[2], c[3]);
                flatten_cubic(&c, (size * 0.002).clamp(1e-5, 0.02), 0, &mut v);
            }
            out.extend(v.into_iter().map(ll));
        }
        if self.closed && out.len() > 1 {
            // back at the first anchor
            out.pop();
        }
        out
    }

    /// The point at parameter `t` (0 to 1) of segment `i`.
    pub fn point(&self, i: usize, t: f64) -> LonLat {
        let (l, _) = split(&self.cubic(i), t);
        ll(l[3])
    }

    /// Move anchor `i` to `to`; its handles move with it.
    pub fn move_node(&mut self, i: usize, to: LonLat) {
        let Some(n) = self.nodes.get_mut(i) else { return };
        let from = [n.at.lon, n.at.lat];
        let to_u = near(to, from);
        let d = [to_u[0] - from[0], to_u[1] - from[1]];
        let shift = |h: LonLat| {
            let u = near(h, from);
            ll([u[0] + d[0], u[1] + d[1]])
        };
        n.h_in = n.h_in.map(shift);
        n.h_out = n.h_out.map(shift);
        n.at = ll(to_u);
    }

    /// Move one handle of anchor `i` to `to`. At a smooth anchor the other
    /// handle turns to stay opposite (keeping its own length).
    pub fn move_handle(&mut self, i: usize, which: Handle, to: LonLat) {
        let Some(n) = self.nodes.get_mut(i) else { return };
        let at = [n.at.lon, n.at.lat];
        let t = near(to, at);
        let (this, other) = match which {
            Handle::In => (&mut n.h_in, n.h_out),
            Handle::Out => (&mut n.h_out, n.h_in),
        };
        *this = Some(ll(t));
        if !n.smooth {
            return;
        }
        let Some(o) = other else { return };
        let len = dist(near(o, at), at);
        let dir = [at[0] - t[0], at[1] - t[1]];
        let k = dist(dir, [0.0, 0.0]);
        if k < 1e-12 {
            return;
        }
        let opp = ll([at[0] + dir[0] / k * len, at[1] + dir[1] / k * len]);
        match which {
            Handle::In => n.h_out = Some(opp),
            Handle::Out => n.h_in = Some(opp),
        }
    }

    /// Pull both handles of anchor `i` out symmetrically, the outgoing one
    /// to `to` (as the pen does while the button is held).
    pub fn pull_handles(&mut self, i: usize, to: LonLat) {
        let Some(n) = self.nodes.get_mut(i) else { return };
        let at = [n.at.lon, n.at.lat];
        let t = near(to, at);
        if dist(t, at) < 1e-9 {
            n.h_in = None;
            n.h_out = None;
            n.smooth = false;
            return;
        }
        n.h_out = Some(ll(t));
        n.h_in = Some(ll([2.0 * at[0] - t[0], 2.0 * at[1] - t[1]]));
        n.smooth = true;
    }

    /// Make anchor `i` a corner: no handles, straight on both sides.
    pub fn make_corner(&mut self, i: usize) {
        if let Some(n) = self.nodes.get_mut(i) {
            n.h_in = None;
            n.h_out = None;
            n.smooth = false;
        }
    }

    /// Make anchor `i` smooth: handles along the direction from its
    /// previous anchor to its next, a third of the way to each.
    pub fn make_smooth(&mut self, i: usize) {
        let n = self.nodes.len();
        if n < 2 || i >= n {
            return;
        }
        let at = [self.nodes[i].at.lon, self.nodes[i].at.lat];
        let prev = (i > 0 || self.closed).then(|| near(self.nodes[(i + n - 1) % n].at, at));
        let next = (i + 1 < n || self.closed).then(|| near(self.nodes[(i + 1) % n].at, at));
        let node = &mut self.nodes[i];
        match (prev, next) {
            (Some(p), Some(q)) => {
                let dir = [q[0] - p[0], q[1] - p[1]];
                let k = dist(dir, [0.0, 0.0]).max(1e-12);
                let (lp, lq) = (dist(p, at) / 3.0, dist(q, at) / 3.0);
                node.h_in = Some(ll([at[0] - dir[0] / k * lp, at[1] - dir[1] / k * lp]));
                node.h_out = Some(ll([at[0] + dir[0] / k * lq, at[1] + dir[1] / k * lq]));
                node.smooth = true;
            }
            // an end of an open curve: one handle, towards its neighbour
            (None, Some(q)) => {
                node.h_in = None;
                node.h_out = Some(ll(lerp(at, q, 1.0 / 3.0)));
                node.smooth = false;
            }
            (Some(p), None) => {
                node.h_out = None;
                node.h_in = Some(ll(lerp(at, p, 1.0 / 3.0)));
                node.smooth = false;
            }
            (None, None) => {}
        }
    }

    /// Add an anchor on segment `i` at `t` without changing the shape (the
    /// segment is split in two); returns its index.
    pub fn insert_node(&mut self, i: usize, t: f64) -> usize {
        let n = self.nodes.len();
        let straight = self.is_straight(i);
        let (l, r) = split(&self.cubic(i), t.clamp(0.0, 1.0));
        let j = (i + 1) % n;
        if !straight {
            if self.nodes[i].h_out.is_some() {
                self.nodes[i].h_out = Some(ll(l[1]));
            }
            if self.nodes[j].h_in.is_some() {
                self.nodes[j].h_in = Some(ll(r[2]));
            }
        }
        let node = if straight { CurveNode::corner(ll(l[3])) } else { CurveNode { at: ll(l[3]), h_in: Some(ll(l[2])), h_out: Some(ll(r[1])), smooth: true } };
        self.nodes.insert(i + 1, node);
        i + 1
    }

    /// Remove anchor `i` (there must be enough left: two on a line, three
    /// round an area).
    pub fn remove_node(&mut self, i: usize) -> TResult<()> {
        let min = if self.closed { 3 } else { 2 };
        if self.nodes.len() <= min {
            return Err(format!("{}至少需要 {min} 个锚点", if self.closed { "区域" } else { "线" }));
        }
        if i >= self.nodes.len() {
            return Err("锚点不存在".into());
        }
        self.nodes.remove(i);
        Ok(())
    }

    /// Every anchor and handle through `f` (moving, turning, scaling).
    pub fn map(&mut self, f: &dyn Fn(LonLat) -> LonLat) {
        for n in &mut self.nodes {
            n.at = f(n.at);
            n.h_in = n.h_in.map(f);
            n.h_out = n.h_out.map(f);
        }
    }

    fn check(&self) -> TResult<()> {
        let min = if self.closed { 3 } else { 2 };
        if self.nodes.len() < min {
            return Err(format!("{}至少需要 {min} 个锚点", if self.closed { "区域" } else { "线" }));
        }
        let ok = |p: &LonLat| p.lon.is_finite() && p.lat.is_finite();
        if !self.nodes.iter().all(|n| ok(&n.at) && n.h_in.iter().chain(n.h_out.iter()).all(ok)) {
            return Err("曲线含无效坐标".into());
        }
        Ok(())
    }
}

impl Document {
    /// The curve a river, route or hatched area is drawn with, if any.
    pub fn curve_of(&self, id: LineId) -> Option<&Curve> {
        self.curves.get(&id)
    }

    /// Give a river or route (open curve) or a hatched area (closed curve)
    /// this curve; its points become the flattened curve.
    pub fn set_curve(&mut self, id: LineId, c: Curve) -> TResult<()> {
        c.check()?;
        let pts = c.flatten();
        if let Some(r) = self.rivers.get_mut(&id) {
            if c.closed {
                return Err("线的曲线不能闭合".into());
            }
            r.pts = pts;
        } else if let Some(a) = self.areas.get_mut(&id) {
            if !c.closed {
                return Err("区域的曲线必须闭合".into());
            }
            a.ring = pts;
        } else {
            return Err("只有河流、线路和斜线区域可以是曲线".into());
        }
        self.curves.insert(id, c);
        Ok(())
    }

    /// Make a line or area an ordinary one again. With `keep_shape` its
    /// points stay as drawn (the flattened curve); without, only the
    /// anchors remain, joined by straight segments.
    pub fn drop_curve(&mut self, id: LineId, keep_shape: bool) -> TResult<()> {
        let c = self.curves.remove(&id).ok_or("这不是曲线")?;
        if !keep_shape {
            let pts: Vec<LonLat> = c.nodes.iter().map(|n| n.at).collect();
            if let Some(r) = self.rivers.get_mut(&id) {
                r.pts = pts;
            } else if let Some(a) = self.areas.get_mut(&id) {
                a.ring = pts;
            }
        }
        Ok(())
    }

    /// Turn an ordinary line or area into a curve through its points:
    /// smooth, or with every point a corner (the same shape, ready for
    /// handles to be pulled out).
    pub fn curve_from_points(&mut self, id: LineId, smooth: bool) -> TResult<()> {
        let (pts, closed) = if let Some(r) = self.rivers.get(&id) {
            (r.pts.clone(), false)
        } else if let Some(a) = self.areas.get(&id) {
            (a.ring.clone(), true)
        } else {
            return Err("只有河流、线路和斜线区域可以是曲线".into());
        };
        if pts.len() > 2000 {
            return Err(format!("这条{}有 {} 个点，太多了：曲线的每个点都是锚点。先用“风格化”的“平滑”减少点数", if closed { "区域边界" } else { "线" }, pts.len()));
        }
        let c = if smooth { Curve::smooth_through(&pts, closed) } else { Curve::corners(&pts, closed) };
        self.set_curve(id, c)
    }

    /// A new river (or, with a style, route) drawn with a curve.
    pub fn add_curve_line(&mut self, c: Curve, route: Option<crate::doc::RouteStyle>) -> TResult<LineId> {
        c.check()?;
        if c.closed {
            return Err("线的曲线不能闭合".into());
        }
        let id = LineId(self.topo.alloc());
        self.rivers.insert(id, crate::doc::LineFeature { name: String::new(), pts: vec![], route, legend: String::new(), show_name: false, hidden: false });
        self.set_curve(id, c)?;
        Ok(id)
    }

    /// A new hatched area outlined with a closed curve.
    pub fn add_curve_area(&mut self, c: Curve) -> TResult<LineId> {
        c.check()?;
        if !c.closed {
            return Err("区域的曲线必须闭合".into());
        }
        let id = LineId(self.topo.alloc());
        self.areas.insert(id, crate::area::AreaMark::new(vec![]));
        self.set_curve(id, c)?;
        Ok(id)
    }

    /// Change a curve in place (an anchor or handle moved...): the change
    /// is made on a copy and the feature's points follow.
    pub fn edit_curve(&mut self, id: LineId, f: impl FnOnce(&mut Curve) -> TResult<()>) -> TResult<()> {
        let mut c = self.curves.get(&id).ok_or("这不是曲线")?.clone();
        f(&mut c)?;
        self.set_curve(id, c)
    }

    /// The points of a line or area changed directly (not through its
    /// curve): it is an ordinary one from now on, as it now is.
    pub(crate) fn points_changed(&mut self, id: LineId) {
        self.curves.remove(&id);
    }

    /// Forget curves of features that are gone.
    pub fn prune_curves(&mut self) {
        let gone: Vec<LineId> = self.curves.keys().copied().filter(|id| !self.rivers.contains_key(id) && !self.areas.contains_key(id)).collect();
        for id in gone {
            self.curves.remove(&id);
        }
    }
}
