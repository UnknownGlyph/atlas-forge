//! Country names: where each one goes, kept up to date incrementally
//! (after an edit only the countries the [`ChangeSet`] touches are worked
//! out again).
//!
//! An automatic name follows a [`LabelPath`]: a gentle curve along the
//! country's long axis through its largest piece, as long and as roomy as
//! the land allows. It is worked out in a plane touching the sphere at the
//! country, east-west scaled by the cosine of the latitude there, so a
//! level line in the plane is a parallel: straight on every flat map, and
//! running round the globe on the globe. The views draw the name along it
//! (bent on a flat map, painted onto the surface on the globe); the
//! exports bend it the same way.

use crate::change::ChangeSet;
use crate::doc::{Document, EntityId, EntityKind, PlanarPolygon};
use crate::geom::{self, LonLat, P2};
use crate::topo::BACKGROUND;
use std::collections::BTreeMap;

/// A name's glyph size for the room across its line (as a share of it).
pub const ROOM: f64 = 0.55;
/// The share of its line a name may take at most.
pub const ALONG: f64 = 0.85;

/// Rough width of a text in em (CJK full width, ASCII about half).
pub fn name_em(name: &str) -> f64 {
    name.trim().chars().map(|c| if c.is_ascii() { 0.56 } else { 1.0 }).sum()
}

/// How large a name `em` wide can be on a line `len` long with `room`
/// across it (all three in the same unit as the answer).
pub fn name_size(len: f64, room: f64, em: f64) -> f64 {
    (room * ROOM).min(len * ALONG / em.max(0.5))
}

/// Space to add between the letters of a name of `n` letters, `width`
/// wide at `size`, so that it spreads along a line `len` long — up to
/// most of its length, never more than a letter's width apart.
pub fn name_spacing(len: f64, width: f64, size: f64, n: usize) -> f64 {
    if n < 2 {
        return 0.0;
    }
    ((len * 0.72 - width) / (n - 1) as f64).clamp(0.0, size * 0.9)
}

/// The plane touching the sphere at `o` (lon unwrapped like the points it
/// is used with): x east, y north, in degrees of arc.
fn scale_at(o: LonLat) -> f64 {
    o.lat.to_radians().cos().max(0.05)
}

/// The line a country's name follows.
#[derive(Clone, Debug, PartialEq)]
pub struct LabelPath {
    /// Where the plane touches the sphere (the middle of the line).
    pub origin: LonLat,
    /// The line in that plane, west to east (degrees of arc, y north).
    pub pts: Vec<P2>,
    /// Distance along the line at each point.
    cum: Vec<f64>,
    /// Room across the line (degrees of arc), over the part a name takes.
    pub height: f64,
    /// The width of the name it was made for (em).
    pub em: f64,
}

impl LabelPath {
    fn new(origin: LonLat, pts: Vec<P2>, height: f64, em: f64) -> Self {
        let mut cum = vec![0.0];
        for w in pts.windows(2) {
            cum.push(cum.last().unwrap() + (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]));
        }
        LabelPath { origin: LonLat::new(geom::wrap_lon(origin.lon), origin.lat), pts, cum, height, em }
    }

    /// A straight line through `at`, turned `angle` degrees counter-clockwise
    /// from east, `half` long each way.
    pub fn straight(at: LonLat, angle: f64, half: f64, height: f64, em: f64) -> Self {
        let (s, c) = angle.to_radians().sin_cos();
        let (c, s) = if c < 0.0 { (-c, -s) } else { (c, s) };
        LabelPath::new(at, vec![[-half * c, -half * s], [half * c, half * s]], height, em)
    }

    pub fn length(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }

    /// The point of the plane at distance `s` along the line.
    fn plane_at(&self, s: f64) -> P2 {
        let n = self.pts.len();
        if n < 2 {
            return self.pts.first().copied().unwrap_or([0.0, 0.0]);
        }
        let s = s.clamp(0.0, self.length());
        let i = self.cum.partition_point(|c| *c < s).clamp(1, n - 1);
        let seg = self.cum[i] - self.cum[i - 1];
        let t = if seg > 0.0 { (s - self.cum[i - 1]) / seg } else { 0.0 };
        let (a, b) = (self.pts[i - 1], self.pts[i]);
        [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
    }

    /// The point `s` along the line and `t` across it (to its left, which
    /// is up when the line is read west to east), on the sphere.
    pub fn point(&self, s: f64, t: f64) -> LonLat {
        let h = (self.length() / 40.0).max(1e-6);
        let (a, b) = (self.plane_at(s - h), self.plane_at(s + h));
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let n = dx.hypot(dy);
        let (tx, ty) = if n > 0.0 { (dx / n, dy / n) } else { (1.0, 0.0) };
        let p = self.plane_at(s);
        self.to_sphere([p[0] - ty * t, p[1] + tx * t])
    }

    fn to_sphere(&self, q: P2) -> LonLat {
        let k = scale_at(self.origin);
        LonLat::new(geom::wrap_lon(self.origin.lon + q[0] / k), (self.origin.lat + q[1]).clamp(-90.0, 90.0))
    }

    /// The line on the sphere, `n` + 1 points from its west end.
    pub fn line(&self, n: usize) -> Vec<LonLat> {
        let n = n.max(1);
        (0..=n).map(|i| self.point(self.length() * i as f64 / n as f64, 0.0)).collect()
    }

    /// The middle of the line.
    pub fn middle(&self) -> LonLat {
        self.point(self.length() * 0.5, 0.0)
    }

    /// How large its name can be (degrees of arc).
    pub fn fit(&self) -> f64 {
        name_size(self.length(), self.height, self.em)
    }
}

/// A run of land across the axis at one step along it.
#[derive(Clone, Copy)]
struct Run {
    u: f64,
    lo: f64,
    hi: f64,
}

/// The smallest room across the line `c` (coefficients of 1, u − um,
/// (u − um)²) over the part of `runs` a name may take, measured square to
/// the line.
fn room(runs: &[Run], um: f64, c: [f64; 3]) -> f64 {
    let n = runs.len();
    let cut = (n as f64 * 0.06) as usize;
    runs[cut..n - cut]
        .iter()
        .map(|r| {
            let x = r.u - um;
            let v = c[0] + c[1] * x + c[2] * x * x;
            let slope = c[1] + 2.0 * c[2] * x;
            2.0 * (v - r.lo).min(r.hi - v) / (1.0 + slope * slope).sqrt()
        })
        .fold(f64::INFINITY, f64::min)
        .max(0.0)
}

/// The best line along direction `angle` (radians) through the rings
/// (already in the plane): its points, the room across it and how large
/// the name can be.
fn along(rings: &[Vec<P2>], angle: f64, em: f64) -> Option<(Vec<P2>, f64, f64)> {
    const N: usize = 32;
    let (dy, dx) = angle.sin_cos();
    let rot: Vec<Vec<P2>> = rings.iter().map(|r| r.iter().map(|p| [p[0] * dx + p[1] * dy, -p[0] * dy + p[1] * dx]).collect()).collect();
    let (umin, umax) = rot[0].iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), p| (a.min(p[0]), b.max(p[0])));
    if umax <= umin || umax.is_nan() {
        return None;
    }
    let du = (umax - umin) / N as f64;
    // the runs of land across each step
    let samples = std::array::from_fn(|i| umin + du * (i as f64 + 0.5));
    let crossings = geom::scan_crossings::<0, N>(&samples, rot.iter().map(Vec::as_slice));
    let cols: Vec<Vec<Run>> = samples.into_iter().zip(crossings)
        .map(|(u, mut vs)| {
            vs.sort_by(f64::total_cmp);
            vs.as_chunks::<2>().0.iter().map(|p| Run { u, lo: p[0], hi: p[1] }).collect()
        })
        .collect();
    // from the tallest run, step to each side along the run that overlaps most
    let (start, first) = cols.iter().enumerate().flat_map(|(i, c)| c.iter().map(move |r| (i, *r))).max_by(|a, b| (a.1.hi - a.1.lo).total_cmp(&(b.1.hi - b.1.lo)))?;
    let next = |from: Run, col: &[Run]| -> Option<Run> {
        col.iter().copied().map(|r| (r, r.hi.min(from.hi) - r.lo.max(from.lo))).filter(|x| x.1 > 0.0).max_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0)
    };
    let mut chain = vec![first];
    let mut cur = first;
    for col in cols[..start].iter().rev() {
        let Some(r) = next(cur, col) else { break };
        chain.insert(0, r);
        cur = r;
    }
    cur = first;
    for col in &cols[start + 1..] {
        let Some(r) = next(cur, col) else { break };
        chain.push(r);
        cur = r;
    }
    // every stretch of it, with a level, a slanted and a bent line through
    // the middles of its runs: the largest name any of them holds (a
    // simpler line counting a little larger), then of the lines holding
    // nearly as large a name the longest
    struct Try {
        score: f64,
        size: f64,
        len: f64,
        p: usize,
        q: usize,
        c: [f64; 3],
        h: f64,
    }
    let mut tries: Vec<Try> = Vec::new();
    let mut top = 0.0f64;
    // (stretches of at least a fifth of the land along the axis, the
    // longest first; a stretch whose runs could not hold a name near the
    // largest so far, on any line, is passed over)
    let shortest = (chain.len() / 5).max(2);
    for span in (shortest..chain.len()).rev() {
        for p in 0..chain.len() - span {
            let q = p + span;
            let runs = &chain[p..=q];
            let len = (q - p + 1) as f64 * du;
            let cut = (runs.len() as f64 * 0.06) as usize;
            let across = runs[cut..runs.len() - cut].iter().map(|r| r.hi - r.lo).fold(f64::INFINITY, f64::min);
            if name_size(len, across, em) * 1.04 < top * 0.97 {
                continue;
            }
            let um = (runs[0].u + runs[runs.len() - 1].u) * 0.5;
            for (k, c) in fit_middles(runs, um, len).into_iter().enumerate() {
                let h = room(runs, um, c);
                let size = name_size(len, h, em);
                let score = size * [1.04, 1.02, 1.0][k];
                top = top.max(score);
                tries.push(Try { score, size, len, p, q, c, h });
            }
        }
    }
    if top <= 0.0 || top.is_nan() {
        return None;
    }
    let best = tries.iter().filter(|t| t.score >= top * 0.97).max_by(|a, b| a.len.total_cmp(&b.len).then(a.score.total_cmp(&b.score)))?;
    let (f, h, size) = (best.c, best.h, best.size);
    let runs = &chain[best.p..=best.q];
    let um = (runs[0].u + runs[runs.len() - 1].u) * 0.5;
    let (u0, u1) = (runs[0].u - du * 0.5, runs[runs.len() - 1].u + du * 0.5);
    let pts = (0..=24)
        .map(|i| {
            let u = u0 + (u1 - u0) * i as f64 / 24.0;
            let v = f[0] + f[1] * (u - um) + f[2] * (u - um).powi(2);
            [u * dx - v * dy, u * dy + v * dx]
        })
        .collect();
    Some((pts, h, size))
}

/// Level, slanted and bent lines through the middles of `runs` (as
/// coefficients of 1, u − um, (u − um)²), weighted by their room.
fn fit_middles(runs: &[Run], um: f64, len: f64) -> [[f64; 3]; 3] {
    let w = |r: &Run| (r.hi - r.lo).max(1e-9);
    let m = |r: &Run| (r.lo + r.hi) * 0.5;
    let sw: f64 = runs.iter().map(w).sum();
    let level = [runs.iter().map(|r| w(r) * m(r)).sum::<f64>() / sw, 0.0, 0.0];
    // weighted least squares on the powers of x = u − um
    let mut a = [[0.0f64; 3]; 3];
    let mut b = [0.0f64; 3];
    for r in runs {
        let x = r.u - um;
        let xs = [1.0, x, x * x];
        for i in 0..3 {
            for j in 0..3 {
                a[i][j] += w(r) * xs[i] * xs[j];
            }
            b[i] += w(r) * xs[i] * m(r);
        }
    }
    let solve2 = || -> Option<[f64; 2]> {
        let det = a[0][0] * a[1][1] - a[0][1] * a[1][0];
        (det.abs() > 1e-12).then(|| [(b[0] * a[1][1] - b[1] * a[0][1]) / det, (a[0][0] * b[1] - a[1][0] * b[0]) / det])
    };
    let slant = solve2().map_or(level, |c| [c[0], c[1].clamp(-0.6, 0.6), 0.0]);
    let bent = solve3(a, b).map_or(slant, |c| {
        // no steeper than about 30° at its ends, sagging at most an eighth of its length
        let slope = c[1].clamp(-0.6, 0.6);
        let bend = c[2].clamp(-0.5 / len, 0.5 / len);
        let bend = bend.clamp(-(0.6 - slope.abs()).max(0.0) / len, (0.6 - slope.abs()).max(0.0) / len);
        [c[0], slope, bend]
    });
    [level, slant, bent]
}

/// Solve a 3 × 3 system (Cramer's rule).
fn solve3(a: [[f64; 3]; 3], b: [f64; 3]) -> Option<[f64; 3]> {
    let det = |m: [[f64; 3]; 3]| m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = det(a);
    if d.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0; 3];
    for (k, o) in out.iter_mut().enumerate() {
        let mut m = a;
        for i in 0..3 {
            m[i][k] = b[i];
        }
        *o = det(m) / d;
    }
    Some(out)
}

/// The long axis of the rings (radians from east): from their second
/// moments, holes taken away.
fn long_axis(rings: &[Vec<P2>]) -> Option<f64> {
    let (mut a, mut mx, mut my, mut mxx, mut myy, mut mxy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    for (k, ring) in rings.iter().enumerate() {
        let area = geom::signed_area(ring);
        // the outer ring counts, holes are taken away, whichever way they run
        let sign = if k == 0 { area.signum() } else { -area.signum() };
        let n = ring.len();
        for i in 0..n {
            let (p, q) = (ring[i], ring[(i + 1) % n]);
            let c = (p[0] * q[1] - q[0] * p[1]) * sign;
            a += c * 0.5;
            mx += c * (p[0] + q[0]) / 6.0;
            my += c * (p[1] + q[1]) / 6.0;
            mxx += c * (p[0] * p[0] + p[0] * q[0] + q[0] * q[0]) / 12.0;
            myy += c * (p[1] * p[1] + p[1] * q[1] + q[1] * q[1]) / 12.0;
            mxy += c * (p[0] * q[1] + 2.0 * p[0] * p[1] + 2.0 * q[0] * q[1] + q[0] * p[1]) / 24.0;
        }
    }
    if a <= 0.0 || a.is_nan() {
        return None;
    }
    let (cx, cy) = (mx / a, my / a);
    let (sxx, syy, sxy) = (mxx / a - cx * cx, myy / a - cy * cy, mxy / a - cx * cy);
    Some(0.5 * (2.0 * sxy).atan2(sxx - syy))
}

/// The line for a name through one piece of land, `c` a point inside it.
fn path_through(poly: &PlanarPolygon, c: P2, em: f64) -> Option<LabelPath> {
    let o = LonLat::new(c[0], c[1]);
    let k = scale_at(o);
    let plane = |p: &P2| [(p[0] - c[0]) * k, p[1] - c[1]];
    let rings: Vec<Vec<P2>> = std::iter::once(&poly.outer).chain(poly.holes.iter()).map(|r| r.iter().map(plane).collect()).collect();
    if rings[0].len() < 3 {
        return None;
    }
    // level, along the long axis (read left to right, never steeper than
    // 65°), and halfway between; level preferred a little
    let axis = long_axis(&rings).unwrap_or(0.0);
    let axis = if axis > std::f64::consts::FRAC_PI_2 { axis - std::f64::consts::PI } else if axis < -std::f64::consts::FRAC_PI_2 { axis + std::f64::consts::PI } else { axis };
    let axis = axis.clamp(-65f64.to_radians(), 65f64.to_radians());
    let mut tries = vec![(0.0, 1.06)];
    if axis.abs() > 3f64.to_radians() {
        tries.push((axis * 0.5, 1.03));
        tries.push((axis, 1.0));
    }
    let (pts, h, _) = tries
        .into_iter()
        .filter_map(|(a, bias)| along(&rings, a, em).map(|x| (x, bias)))
        .max_by(|a, b| (a.0 .2 * a.1).total_cmp(&(b.0 .2 * b.1)))
        .map(|x| x.0)?;
    // the plane moved to touch the sphere at the line's middle
    let mid = pts[pts.len() / 2];
    let origin = LonLat::new(c[0] + mid[0] / k, (c[1] + mid[1]).clamp(-89.0, 89.0));
    let k2 = scale_at(origin);
    let pts = pts.iter().map(|p| [(c[0] + p[0] / k - origin.lon) * k2, c[1] + p[1] - origin.lat]).collect();
    Some(LabelPath::new(origin, pts, h, em))
}

impl Document {
    /// The line a country's name follows when it has not been placed by
    /// hand; `None` for a country without land or a name.
    pub fn label_path(&self, e: EntityId) -> Option<LabelPath> {
        let x = self.entities.get(&e)?;
        // a unit under another (a province) is named with its country: the
        // country's name covers the land of everything under it
        if x.parent.is_some() {
            return None;
        }
        let em = name_em(&x.name);
        if em <= 0.0 {
            return None;
        }
        let tree: std::collections::HashSet<crate::topo::FaceId> = self.faces_of_tree(e).into_iter().collect();
        let polys = if tree.len() == self.faces_of(e).len() { self.dissolve(e) } else { self.dissolve_faces(&tree) };
        let r = self.radius();
        let area = |p: &PlanarPolygon| geom::spherical_area(&p.outer, r).abs();
        let best = polys.iter().map(|p| (p, area(p))).max_by(|a, b| a.1.total_cmp(&b.1));
        let level = |(at, w): (LonLat, f64)| LabelPath::straight(at, 0.0, w * 0.5, w * 0.3, em);
        // the background (it has no outer ring) and caps over a pole: level, at the anchor
        let open_sea = self.owner.get(&BACKGROUND) == Some(&e) && best.is_none_or(|(_, a)| a < self.topo.face_area(BACKGROUND, r));
        let polar = best.is_some_and(|(b, _)| {
            let bb = geom::bbox(&b.outer);
            bb[2] - bb[0] > 359.9
        });
        if open_sea || polar {
            return self.label_anchor(e).map(level);
        }
        let (best, _) = best?;
        let (c, w) = geom::interior_point(&best.outer, &best.holes)?;
        Some(path_through(best, c, em).unwrap_or_else(|| level((LonLat::new(geom::wrap_lon(c[0]), c[1]), w))))
    }
}

#[derive(Default)]
pub struct LabelCache {
    src: Option<Document>,
    paths: BTreeMap<EntityId, LabelPath>,
}

impl LabelCache {
    /// Bring the lines up to date with `doc`; returns how many were recomputed.
    pub fn update(&mut self, doc: &Document) -> usize {
        let todo: Vec<EntityId> = match &self.src {
            Some(s) if s.owner.ptr_eq(&doc.owner) && s.topo.faces.ptr_eq(&doc.topo.faces) && s.entities.ptr_eq(&doc.entities) => return 0,
            Some(s) => {
                // a unit's change is its country's: the top of its hierarchy, then and now
                let mut v: std::collections::BTreeSet<EntityId> = ChangeSet::between(s, doc).affected_entities(s, doc);
                let tops: Vec<EntityId> = v.iter().flat_map(|e| [s.top_ancestor(*e), doc.top_ancestor(*e)]).collect();
                v.extend(tops);
                v.into_iter().collect()
            }
            None => {
                self.paths.clear();
                doc.countries()
            }
        };
        let mut n = 0;
        for e in todo {
            self.paths.remove(&e);
            if doc.entities.get(&e).is_some_and(|x| x.kind == EntityKind::Country) {
                if let Some(p) = doc.label_path(e) {
                    self.paths.insert(e, p);
                }
            }
            n += 1;
        }
        self.paths.retain(|e, _| doc.entities.contains_key(e));
        self.src = Some(doc.clone());
        n
    }

    /// The cached automatic baseline, also used to size a manually placed name.
    pub fn path(&self, entity: EntityId) -> Option<&LabelPath> {
        self.paths.get(&entity)
    }

    /// The lines, the one holding the largest name first.
    pub fn sorted(&self) -> Vec<(EntityId, LabelPath)> {
        let mut v: Vec<_> = self.paths.iter().map(|(e, p)| (*e, p.clone())).collect();
        v.sort_by(|a, b| b.1.fit().total_cmp(&a.1.fit()).then(a.0.cmp(&b.0)));
        v
    }

    pub fn reset(&mut self) {
        self.src = None;
        self.paths.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::grid_world;

    #[test]
    fn incremental_equals_full_recompute() {
        let mut d = grid_world(8, 5, 4, 9);
        let mut cache = LabelCache::default();
        assert_eq!(cache.update(&d), 40);
        assert_eq!(cache.update(&d), 0);
        // move a border vertex between two countries
        let (eid, e) = d.topo.edges.iter().find(|(_, e)| e.pts.len() > 2 && e.left != crate::BACKGROUND && e.right != crate::BACKGROUND).map(|(i, e)| (*i, e.clone())).unwrap();
        let p = e.pts[1];
        d.topo.move_vertex(crate::VertexRef::Pt { edge: eid, idx: 1 }, LonLat::new(p.lon + 0.3, p.lat + 0.2)).unwrap();
        // merge one country into another
        let cs = d.countries();
        d.merge_entities(cs[5], cs[6]).unwrap();
        let recomputed = cache.update(&d);
        assert!(recomputed <= 4, "only touched countries are recomputed, got {recomputed}");
        let mut fresh = LabelCache::default();
        fresh.update(&d);
        assert_eq!(cache.sorted(), fresh.sorted());
    }
}
