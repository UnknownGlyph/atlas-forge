//! Hatched areas: free shapes drawn over the map for disputed lands,
//! occupation zones, claims, cultural regions... They are layout, not
//! territory (ownership is unchanged), and like symbols may carry a year
//! range. The hatching is computed here from the shape on screen so the
//! app and the exports draw the same stripes.

use crate::doc::{Document, LineId};
use crate::topo::TResult;
use crate::geom::{self, LonLat};
use crate::symbol::in_years;
use serde::{Deserialize, Serialize};

/// Width of the hatch lines and the outline, in pixels (scaled in export).
pub const HATCH_WIDTH: f32 = 1.2;
pub const OUTLINE_WIDTH: f32 = 1.6;
/// Size of an area's name in pixels (scaled in export).
pub const NAME_PX: f32 = 14.0;
/// Radius of the dots of a dotted area, in pixels (scaled in export).
pub const DOT_RADIUS: f32 = 1.4;

/// How a hatched area is filled. Different patterns tell neighbouring or
/// overlapping zones apart (two occupying powers, a claim on a dispute).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pattern {
    /// Parallel stripes.
    #[default]
    Lines,
    /// Stripes both ways (the angle and the angle plus 90°).
    Cross,
    /// A staggered grid of dots along the stripes.
    Dots,
    /// Little peaks on the dots' grid, upright (a mountain range).
    Mountains,
    /// Little trees on the dots' grid, upright (a forest).
    Trees,
}

impl Pattern {
    pub const ALL: [Pattern; 5] = [Pattern::Lines, Pattern::Cross, Pattern::Dots, Pattern::Mountains, Pattern::Trees];
    pub fn name(self) -> &'static str {
        match self {
            Pattern::Lines => "斜线",
            Pattern::Cross => "交叉线",
            Pattern::Dots => "点",
            Pattern::Mountains => "山地",
            Pattern::Trees => "森林",
        }
    }
}

/// Little upright pictures, drawn as strokes, at each of `pts` (`s` the
/// grid's spacing): a peak, or a tree (a crown over a trunk).
fn icons(pts: &[[f32; 2]], s: f32, trees: bool) -> Vec<[[f32; 2]; 2]> {
    let (w, h) = (s * 0.32, s * 0.3);
    let mut out = Vec::with_capacity(pts.len() * 4);
    for (i, p) in pts.iter().enumerate() {
        if trees {
            let (top, l, r) = ([p[0], p[1] - h], [p[0] - w * 0.7, p[1] + h * 0.3], [p[0] + w * 0.7, p[1] + h * 0.3]);
            out.extend([[top, l], [l, r], [r, top], [[p[0], p[1] + h * 0.3], [p[0], p[1] + h * 0.8]]]);
        } else {
            // every other peak a little higher: a range rather than a grid
            let k = if i % 2 == 0 { 1.0 } else { 0.75 };
            let top = [p[0], p[1] - h * k];
            out.extend([[[p[0] - w, p[1] + h * 0.5], top], [top, [p[0] + w, p[1] + h * 0.5]]]);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaMark {
    /// Written at its middle when not empty.
    #[serde(default)]
    pub name: String,
    /// The outline, not closed (the last point joins the first).
    pub ring: Vec<LonLat>,
    /// More rings filled with it, even-odd: other pieces (islands) and
    /// holes (lakes, enclaves), as when made from a country's outline.
    /// Only `ring` is edited point by point.
    #[serde(default)]
    pub extra: Vec<Vec<LonLat>>,
    pub color: [u8; 4],
    /// Distance between stripes in pixels, and their angle in degrees
    /// (counter-clockwise from horizontal, as seen).
    pub spacing: f32,
    pub angle: f32,
    /// A light wash of the colour under the stripes.
    #[serde(default)]
    pub tint: bool,
    #[serde(default)]
    pub pattern: Pattern,
    #[serde(default)]
    pub dashed: bool,
    /// What the area stands for in the export's legend ("争议地区"...);
    /// empty keeps it out. Areas with the same text and look share a row.
    #[serde(default)]
    pub legend: String,
    /// Hidden: not drawn, exported or put in the legend, but kept (and
    /// listed in the 要素 tab, where it is shown again).
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
}

impl AreaMark {
    pub fn new(ring: Vec<LonLat>) -> Self {
        Self { name: String::new(), ring, extra: Vec::new(), color: [150, 30, 30, 255], spacing: 8.0, angle: 45.0, tint: false, pattern: Pattern::Lines, dashed: true, legend: String::new(), hidden: false, from: None, to: None }
    }

    /// Whether it shows when viewing `year` (`None`: no year selected, all show).
    pub fn shown_in(&self, year: Option<i32>) -> bool {
        !self.hidden && in_years(self.from, self.to, year)
    }

    /// Every ring: the outline first, then the extra pieces and holes.
    pub fn rings(&self) -> Vec<&Vec<LonLat>> {
        std::iter::once(&self.ring).chain(self.extra.iter()).filter(|r| r.len() >= 3).collect()
    }

    /// The colour of the wash under the stripes.
    pub fn tint_color(&self) -> [u8; 4] {
        [self.color[0], self.color[1], self.color[2], 45]
    }
}

/// Stripes `spacing` apart at `angle` degrees (counter-clockwise as seen,
/// y down) across the area inside `rings` (outer and holes, even-odd), as
/// segments. Stripes sit on multiples of `spacing` from the origin, so
/// they do not shimmer as the shape changes.
pub fn hatch(rings: &[Vec<[f32; 2]>], spacing: f32, angle: f32) -> Vec<[[f32; 2]; 2]> {
    let spacing = spacing.max(1.0);
    let (sn, cs) = angle.to_radians().sin_cos();
    // along the stripes, and across them
    let d = [cs, -sn];
    let n = [sn, cs];
    let dot = |p: [f32; 2], v: [f32; 2]| p[0] * v[0] + p[1] * v[1];
    let edges: Vec<([f32; 2], [f32; 2])> = rings
        .iter()
        .filter(|r| r.len() >= 3)
        .flat_map(|r| (0..r.len()).map(move |i| (r[i], r[(i + 1) % r.len()])))
        .map(|(a, b)| ([dot(a, n), dot(a, d)], [dot(b, n), dot(b, d)]))
        .collect();
    let Some(lo) = edges.iter().map(|e| e.0[0]).reduce(f32::min) else { return vec![] };
    let hi = edges.iter().map(|e| e.0[0]).fold(f32::MIN, f32::max);
    let mut out = Vec::new();
    let mut c = (lo / spacing).ceil() * spacing;
    let mut ts = Vec::new();
    while c < hi {
        ts.clear();
        for (a, b) in &edges {
            if (a[0] <= c) != (b[0] <= c) {
                ts.push(a[1] + (c - a[0]) / (b[0] - a[0]) * (b[1] - a[1]));
            }
        }
        ts.sort_by(f32::total_cmp);
        for p in ts.chunks_exact(2) {
            let at = |t: f32| [c * n[0] + t * d[0], c * n[1] + t * d[1]];
            out.push([at(p[0]), at(p[1])]);
        }
        c += spacing;
    }
    out
}

/// Dots `spacing` apart along stripes `spacing` apart at `angle`, every
/// other row shifted by half a step, inside `rings`. Like the stripes they
/// sit on multiples of `spacing`, so they do not shimmer.
pub fn dots(rings: &[Vec<[f32; 2]>], spacing: f32, angle: f32) -> Vec<[f32; 2]> {
    let spacing = spacing.max(1.0);
    let (sn, cs) = angle.to_radians().sin_cos();
    let (d, n) = ([cs, -sn], [sn, cs]);
    let dot = |p: [f32; 2], v: [f32; 2]| p[0] * v[0] + p[1] * v[1];
    let mut out = Vec::new();
    for [p, q] in hatch(rings, spacing, angle) {
        let c = dot(p, n);
        let row = (c / spacing).round() as i64;
        let shift = if row.rem_euclid(2) == 1 { spacing * 0.5 } else { 0.0 };
        let (t0, t1) = (dot(p, d).min(dot(q, d)), dot(p, d).max(dot(q, d)));
        let mut t = ((t0 - shift) / spacing).ceil() * spacing + shift;
        while t <= t1 {
            out.push([c * n[0] + t * d[0], c * n[1] + t * d[1]]);
            t += spacing;
        }
    }
    out
}

/// An area's fill in `pattern`: the strokes and the dots, with the room
/// for a name (`gap`, [x0, y0, x1, y1]) left empty. Shared by the app, the
/// exports and the legend.
pub fn fill_pattern(rings: &[Vec<[f32; 2]>], pattern: Pattern, spacing: f32, angle: f32, gap: Option<[f32; 4]>) -> (Vec<[[f32; 2]; 2]>, Vec<[f32; 2]>) {
    let (mut segs, mut pts) = match pattern {
        Pattern::Lines => (hatch(rings, spacing, angle), vec![]),
        Pattern::Cross => {
            let mut s = hatch(rings, spacing, angle);
            s.extend(hatch(rings, spacing, angle + 90.0));
            (s, vec![])
        }
        Pattern::Dots => (vec![], dots(rings, spacing, angle)),
        Pattern::Mountains => (icons(&dots(rings, spacing, angle), spacing, false), vec![]),
        Pattern::Trees => (icons(&dots(rings, spacing, angle), spacing, true), vec![]),
    };
    if let Some(g) = gap {
        segs = cut_out(segs, g);
        pts.retain(|p| !(p[0] > g[0] && p[0] < g[2] && p[1] > g[1] && p[1] < g[3]));
    }
    (segs, pts)
}

/// The stripes with the part inside `rect` ([x0, y0, x1, y1]) cut out,
/// so a name written there is not crossed by them.
pub fn cut_out(segs: Vec<[[f32; 2]; 2]>, rect: [f32; 4]) -> Vec<[[f32; 2]; 2]> {
    let mut out = Vec::with_capacity(segs.len());
    for [p, q] in segs {
        // the part of p..q inside the rectangle, as t0..t1 (Liang–Barsky)
        let d = [q[0] - p[0], q[1] - p[1]];
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        let mut inside = true;
        for (pk, qk) in [(-d[0], p[0] - rect[0]), (d[0], rect[2] - p[0]), (-d[1], p[1] - rect[1]), (d[1], rect[3] - p[1])] {
            if pk == 0.0 {
                if qk < 0.0 {
                    inside = false;
                }
            } else {
                let r = qk / pk;
                if pk < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
            }
        }
        if !inside || t0 >= t1 {
            out.push([p, q]);
            continue;
        }
        let at = |t: f32| [p[0] + d[0] * t, p[1] + d[1] * t];
        if t0 > 0.0 {
            out.push([p, at(t0)]);
        }
        if t1 < 1.0 {
            out.push([at(t1), q]);
        }
    }
    out
}

/// Rings filled even-odd sorted into polygons for renderers that need
/// outer rings with their holes (triangulation): (outer, its holes), by
/// index. A ring inside an odd number of others is a hole of the smallest
/// ring containing it.
pub fn group_rings(rings: &[Vec<[f32; 2]>]) -> Vec<(usize, Vec<usize>)> {
    let inside = |p: [f32; 2], r: &[[f32; 2]]| {
        let mut c = false;
        for i in 0..r.len() {
            let (a, b) = (r[i], r[(i + 1) % r.len()]);
            if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < a[0] + (p[1] - a[1]) / (b[1] - a[1]) * (b[0] - a[0]) {
                c = !c;
            }
        }
        c
    };
    let area = |r: &[[f32; 2]]| (0..r.len()).map(|i| r[i][0] * r[(i + 1) % r.len()][1] - r[(i + 1) % r.len()][0] * r[i][1]).sum::<f32>().abs() * 0.5;
    let containers: Vec<Vec<usize>> = (0..rings.len())
        .map(|i| (0..rings.len()).filter(|&j| j != i && !rings[i].is_empty() && inside(rings[i][0], &rings[j])).collect())
        .collect();
    let mut out: Vec<(usize, Vec<usize>)> = (0..rings.len()).filter(|&i| containers[i].len() % 2 == 0).map(|i| (i, vec![])).collect();
    for i in (0..rings.len()).filter(|&i| containers[i].len() % 2 == 1) {
        if let Some(&owner) = containers[i].iter().filter(|j| containers[**j].len() % 2 == 0).min_by(|a, b| area(&rings[**a]).total_cmp(&area(&rings[**b]))) {
            if let Some(g) = out.iter_mut().find(|g| g.0 == owner) {
                g.1.push(i);
            }
        }
    }
    out
}

/// Area-weighted centre of a ring (pixels), for the name; the mean of the
/// points when the ring has no area.
pub fn centre(ring: &[[f32; 2]]) -> Option<[f32; 2]> {
    let n = ring.len();
    if n == 0 {
        return None;
    }
    let (mut a, mut cx, mut cy) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let (p, q) = (ring[i], ring[(i + 1) % n]);
        let cr = (p[0] as f64) * (q[1] as f64) - (q[0] as f64) * (p[1] as f64);
        a += cr;
        cx += (p[0] + q[0]) as f64 * cr;
        cy += (p[1] + q[1]) as f64 * cr;
    }
    if a.abs() < 1e-9 {
        let m = ring.iter().fold([0.0, 0.0], |s, p| [s[0] + p[0], s[1] + p[1]]);
        return Some([m[0] / n as f32, m[1] / n as f32]);
    }
    Some([(cx / (3.0 * a)) as f32, (cy / (3.0 * a)) as f32])
}

impl Document {
    /// The hatched area whose outline passes nearest to `p` within `tol`
    /// degrees (any year).
    pub fn area_edge_at(&self, p: LonLat, tol: f64) -> Option<LineId> {
        let mut best: Option<(f64, LineId)> = None;
        for (id, a) in self.areas.iter().filter(|(id, a)| !a.hidden && !self.group_hides(**id)) {
            for r in a.rings() {
                for i in 0..r.len() {
                    let (_, d, _) = geom::closest_on_segment(p, r[i], r[(i + 1) % r.len()]);
                    if d <= tol && best.is_none_or(|b| d < b.0) {
                        best = Some((d, *id));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// The outline of `e` and everything under it (its provinces...) in
    /// `year`, as rings: outer rings, largest first, then holes.
    pub fn entity_rings(&self, e: crate::EntityId, year: Option<i32>) -> Vec<Vec<LonLat>> {
        let under = |mut x: crate::EntityId| {
            for _ in 0..32 {
                if x == e {
                    return true;
                }
                match self.parent_at(x, year) {
                    Some(p) => x = p,
                    None => return false,
                }
            }
            false
        };
        let faces: std::collections::HashSet<crate::FaceId> = self
            .topo
            .faces
            .keys()
            .filter(|f| **f != crate::BACKGROUND)
            .filter(|f| self.effective_owner_at(**f, year).is_some_and(under))
            .copied()
            .collect();
        let mut polys = self.dissolve_faces(&faces);
        polys.sort_by(|a, b| geom::signed_area(&b.outer).abs().total_cmp(&geom::signed_area(&a.outer).abs()));
        let to_ll = |r: &Vec<geom::P2>| r.iter().map(|p| LonLat::new(geom::wrap_lon(p[0]), p[1])).collect::<Vec<LonLat>>();
        let mut rings: Vec<Vec<LonLat>> = polys.iter().map(|p| to_ll(&p.outer)).collect();
        rings.extend(polys.iter().flat_map(|p| p.holes.iter().map(to_ll)));
        rings
    }

    /// A hatched area outlining `e` in `year` (its pieces and holes), named
    /// after it, in a darker shade of its colour; returns its id. The area
    /// is a marking only: territory is unchanged, and the outline stays as
    /// it was in that year when the borders later change.
    pub fn area_from_entity(&mut self, e: crate::EntityId, year: Option<i32>) -> TResult<LineId> {
        let ent = self.entities.get(&e).ok_or("国家不存在")?;
        let mut rings = self.entity_rings(e, year).into_iter();
        let ring = rings.next().ok_or_else(|| match year {
            Some(y) => format!("{} 在 {y} 年没有领土", ent.name),
            None => format!("{} 没有领土", ent.name),
        })?;
        let mut a = AreaMark::new(ring);
        a.extra = rings.collect();
        a.name = match year {
            Some(y) => format!("{}（{y} 年）", ent.name),
            None => ent.name.clone(),
        };
        let c = ent.color;
        a.color = [(c[0] as f32 * 0.55) as u8, (c[1] as f32 * 0.55) as u8, (c[2] as f32 * 0.55) as u8, 255];
        let id = LineId(self.topo.alloc());
        self.areas.insert(id, a);
        Ok(id)
    }

    /// Like `area_edge_at`, with the side (from point `i` to the next,
    /// the last side closing the ring) and the closest point on it.
    pub fn area_on_edge(&self, p: LonLat, tol: f64) -> Option<(LineId, usize, LonLat)> {
        let mut best: Option<(f64, LineId, usize, LonLat)> = None;
        for (id, a) in self.areas.iter().filter(|(id, a)| !a.hidden && !self.group_hides(**id)) {
            let n = a.ring.len();
            for i in 0..n {
                let (q, d, _) = geom::closest_on_segment(p, a.ring[i], a.ring[(i + 1) % n]);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, *id, i, q));
                }
            }
        }
        best.map(|(_, id, i, q)| (id, i, q))
    }

    /// The outline point of a hatched area nearest to `p` within `tol`
    /// degrees, with its distance.
    pub fn area_point_at(&self, p: LonLat, tol: f64) -> Option<(LineId, usize, f64)> {
        let mut best: Option<(LineId, usize, f64)> = None;
        for (id, a) in self.areas.iter().filter(|(id, a)| !a.hidden && !self.group_hides(**id)) {
            for (i, q) in a.ring.iter().enumerate() {
                let d = geom::geo_dist_deg(p, *q);
                if d <= tol && best.is_none_or(|b| d < b.2) {
                    best = Some((*id, i, d));
                }
            }
        }
        best
    }

    /// Move point `i` of an area's outline.
    pub fn move_area_point(&mut self, id: LineId, i: usize, to: LonLat) -> TResult<()> {
        let pt = self.areas.get_mut(&id).ok_or("区域不存在")?.ring.get_mut(i).ok_or("点不存在")?;
        *pt = to;
        self.points_changed(id);
        Ok(())
    }

    /// Add a point on side `side` (between point `side` and the next);
    /// returns its index.
    pub fn insert_area_point(&mut self, id: LineId, side: usize, at: LonLat) -> TResult<usize> {
        let a = self.areas.get_mut(&id).ok_or("区域不存在")?;
        if side >= a.ring.len() {
            return Err("边不存在".into());
        }
        a.ring.insert(side + 1, at);
        self.points_changed(id);
        Ok(side + 1)
    }

    /// Remove point `i`; an area keeps at least three.
    pub fn delete_area_point(&mut self, id: LineId, i: usize) -> TResult<()> {
        let a = self.areas.get_mut(&id).ok_or("区域不存在")?;
        if a.ring.len() <= 3 {
            return Err("区域至少需要三个点（可改为删除整个区域）".into());
        }
        if i >= a.ring.len() {
            return Err("点不存在".into());
        }
        a.ring.remove(i);
        self.points_changed(id);
        Ok(())
    }
}

/// How two hatched areas combine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AreaOp {
    /// Both together.
    Union,
    /// The first with the second cut out.
    Difference,
    /// Only where they overlap.
    Intersection,
}

impl AreaOp {
    pub fn name(self) -> &'static str {
        match self {
            AreaOp::Union => "合并",
            AreaOp::Difference => "减去",
            AreaOp::Intersection => "相交",
        }
    }
}

/// An area's rings as a polygon set, longitudes continued from `lon0` so
/// two areas near the antimeridian share one frame.
fn area_polygons(a: &AreaMark, lon0: f64) -> geo::MultiPolygon<f64> {
    let rings: Vec<Vec<geom::P2>> = a
        .rings()
        .into_iter()
        .map(|r| {
            let start = lon0 + geom::wrap_lon(r[0].lon - lon0);
            geom::unwrap_path(r, Some(start))
        })
        .collect();
    let f: Vec<Vec<[f32; 2]>> = rings.iter().map(|r| r.iter().map(|p| [p[0] as f32, p[1] as f32]).collect()).collect();
    let ls = |r: &Vec<geom::P2>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    geo::MultiPolygon::new(group_rings(&f).into_iter().map(|(outer, holes)| geo::Polygon::new(ls(&rings[outer]), holes.iter().map(|h| ls(&rings[*h])).collect())).collect())
}

impl Document {
    /// Combine hatched area `other` into `target` (`op`): the target keeps
    /// its name, look, years and legend and takes the resulting outline
    /// (largest piece first, then other pieces and holes); `other` is used
    /// up. An empty result changes nothing and is an error.
    pub fn combine_areas(&mut self, target: LineId, other: LineId, op: AreaOp) -> TResult<()> {
        use geo::{Area, BooleanOps};
        if target == other {
            return Err("请选择另一个区域".into());
        }
        let a = self.areas.get(&target).ok_or("区域不存在")?.clone();
        let b = self.areas.get(&other).ok_or("区域不存在")?.clone();
        let lon0 = a.ring.first().ok_or("区域没有点")?.lon;
        let (pa, pb) = (area_polygons(&a, lon0), area_polygons(&b, lon0));
        let out = match op {
            AreaOp::Union => pa.union(&pb),
            AreaOp::Difference => pa.difference(&pb),
            AreaOp::Intersection => pa.intersection(&pb),
        };
        let mut polys: Vec<geo::Polygon<f64>> = out.0.into_iter().filter(|p| p.unsigned_area() > 1e-12).collect();
        if polys.is_empty() {
            return Err(match op {
                AreaOp::Intersection => "两个区域没有重叠".into(),
                _ => "结果为空（区域被完全减去）".into(),
            });
        }
        polys.sort_by(|x, y| y.unsigned_area().total_cmp(&x.unsigned_area()));
        // geo's rings repeat their first point at the end
        let to_ll = |ls: &geo::LineString<f64>| {
            let mut v: Vec<LonLat> = ls.0.iter().map(|c| LonLat::new(geom::wrap_lon(c.x), c.y)).collect();
            if v.len() > 1 && v.first() == v.last() {
                v.pop();
            }
            v
        };
        let mut rings = polys.iter().map(|p| to_ll(p.exterior()));
        let main = rings.next().ok_or("结果为空")?;
        let mut extra: Vec<Vec<LonLat>> = rings.collect();
        extra.extend(polys.iter().flat_map(|p| p.interiors().iter().map(to_ll)));
        let t = self.areas.get_mut(&target).ok_or("区域不存在")?;
        t.ring = main;
        t.extra = extra.into_iter().filter(|r| r.len() >= 3).collect();
        self.areas.remove(&other);
        self.points_changed(target);
        self.points_changed(other);
        self.style_of.remove(&other);
        Ok(())
    }
}
