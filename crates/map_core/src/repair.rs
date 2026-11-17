//! Repair import: turn imperfect polygon data into a clean shared-edge
//! topology, and report every change that was made.
//!
//! Pipeline (all in the lon/lat strip [-180, 180] × [-90, 90]):
//! 1. split polygons at the antimeridian / close polar rings (boolean
//!    intersection with the strip),
//! 2. cluster vertices closer than the tolerance onto one position,
//! 3. snap vertices onto nearby segments of other polygons (inserting them),
//!    so nearly-coincident borders end up with identical vertex sequences,
//! 4. resolve overlaps: each polygon minus the earlier polygons it overlaps
//!    (the earlier feature wins; every conflict is reported),
//! 5. re-node the boolean output (shared intersection points),
//! 6. strict build ([`crate::build`]),
//! 7. fill slivers: gaps thinner than a few tolerances merge into the
//!    neighbour that shares the longest border with them; thick gaps stay
//!    as lakes.

use crate::build::{self, BuildReport, SourcePolygon};
use crate::geom::{self, LonLat, P2};
use crate::index::GridIndex;
use crate::topo::{FaceId, FaceKind, Topology, BACKGROUND};
use geo::BooleanOps;
use std::collections::HashMap;

/// Metres per degree of latitude (used to express tolerances in metres).
const M_PER_DEG: f64 = 111_320.0;

#[derive(Clone, Debug)]
pub struct RepairOptions {
    /// Snap distance in metres.
    pub snap_m: f64,
    /// Merge thin gaps (thickness ≤ 3 × snap distance) into a neighbour.
    pub fill_slivers: bool,
    /// Planet radius (km) for area reporting.
    pub radius_km: f64,
    /// First id the rebuilt topology may allocate.
    pub first_id: u64,
    /// Run the boolean overlap-removal step. Callers whose polygons are
    /// disjoint by construction (e.g. the drawing overlay) can skip it.
    pub resolve_overlaps: bool,
    /// Points that must stay exactly where they are and become nodes
    /// (where a rebuilt region joins untouched topology).
    pub forced_junctions: Vec<LonLat>,
    /// Preferred positions (strip coordinates): vertices within the snap
    /// distance of one go exactly onto it (the first of several close
    /// together wins; forced junctions before all of them). A drawn polygon's own corners,
    /// which the boolean kernel rounds (~1e-8°): without them a border
    /// drawn along a grid line is left a hair off it, and the next polygon
    /// drawn exactly along the same line leaves slivers.
    pub prefer: Vec<P2>,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self { snap_m: 100.0, fill_slivers: true, radius_km: 6371.0088, first_id: 1, resolve_overlaps: true, forced_junctions: vec![], prefer: vec![] }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Overlap {
    /// Feature that kept the area (earlier in the input).
    pub kept_by: usize,
    /// Feature that lost it.
    pub removed_from: usize,
    pub area_km2: f64,
    pub at: LonLat,
}

#[derive(Clone, Debug, Default)]
pub struct RepairReport {
    pub snapped_vertices: usize,
    pub max_shift_m: f64,
    pub inserted_vertices: usize,
    pub overlaps: Vec<Overlap>,
    /// Features that were completely covered by earlier ones.
    pub swallowed: Vec<usize>,
    pub slivers_filled: usize,
    pub sliver_area_km2: f64,
    /// (feature, area before km², area after km²), largest relative change first.
    pub area_changes: Vec<(usize, f64, f64)>,
    pub build: BuildReport,
    pub problems: Vec<String>,
}

pub struct Repaired {
    pub topo: Topology,
    /// Faces of each feature after repair.
    pub faces: HashMap<usize, Vec<FaceId>>,
    pub report: RepairReport,
}

type Ring = Vec<P2>;

struct Part {
    feature: usize,
    rings: Vec<Ring>, // outer first, no closing point
}

fn to_geo(rings: &[Ring]) -> geo::Polygon<f64> {
    let ls = |r: &Ring| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    geo::Polygon::new(ls(&rings[0]), rings[1..].iter().map(ls).collect())
}

fn from_geo(p: &geo::Polygon<f64>) -> Vec<Ring> {
    let ring = |ls: &geo::LineString<f64>| {
        let mut v: Ring = ls.coords().map(|c| [c.x, c.y]).collect();
        if v.len() > 1 && v.first() == v.last() {
            v.pop();
        }
        v
    };
    let mut out = vec![ring(p.exterior())];
    out.extend(p.interiors().iter().map(ring));
    out
}

fn parts_area(rings: &[Ring], r: f64) -> f64 {
    if rings.is_empty() || rings[0].len() < 3 {
        return 0.0;
    }
    let mut a = geom::spherical_area(&rings[0], r).abs();
    for h in &rings[1..] {
        a -= geom::spherical_area(h, r).abs();
    }
    a.max(0.0)
}

fn mp_area(mp: &geo::MultiPolygon<f64>, r: f64) -> f64 {
    mp.0.iter().map(|p| parts_area(&from_geo(p), r)).sum()
}

fn ring_bbox(rings: &[Ring]) -> [f64; 4] {
    geom::bbox(&rings[0])
}

/// Normalise, drop repeated points and zero-width spikes (e.g. the detour
/// some datasets draw from a polar coast down the antimeridian to the pole:
/// once ±180 coincide it is a spike), before converting to planar form.
fn clean_source_ring(r: &[LonLat]) -> Vec<LonLat> {
    let same = |a: LonLat, b: LonLat| geom::geo_dist_deg(a, b) < 1e-12;
    let mut v: Vec<LonLat> = Vec::with_capacity(r.len());
    for p in r {
        let p = p.normalized();
        if v.last().is_none_or(|q| !same(*q, p)) {
            v.push(p);
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        while v.len() > 1 && same(v[0], *v.last().unwrap()) {
            v.pop();
        }
        let n = v.len();
        if n < 3 {
            break;
        }
        for i in 0..n {
            if same(v[(i + n - 1) % n], v[(i + 1) % n]) {
                let (tip, base) = (i, (i + 1) % n);
                let (hi, lo) = if tip > base { (tip, base) } else { (base, tip) };
                v.remove(hi);
                v.remove(lo);
                v.dedup_by(|a, b| same(*a, *b));
                changed = true;
                break;
            }
        }
    }
    v
}

/// Step 1: polygon → parts inside the strip.
fn to_parts(poly: &SourcePolygon, feature: usize) -> Vec<Part> {
    let Some(outer) = poly.rings.first() else { return vec![] };
    let mut outer = clean_source_ring(outer);
    if outer.len() < 3 {
        return vec![];
    }
    // a ring around a pole has no clockwise/counter-clockwise reading; its
    // direction decides which pole is inside. Enclose the nearer one, as
    // the strict builder does (eastward winding encloses the north pole).
    let w = geom::ring_winding(&outer);
    if w.abs() > 180.0 {
        let mean_lat = outer.iter().map(|p| p.lat).sum::<f64>() / outer.len() as f64;
        if (mean_lat >= 0.0) != (w > 0.0) {
            outer.reverse();
        }
        // from the antimeridian, so the cap is one part
        outer = geom::start_at_seam(&outer);
    }
    let po = geom::ring_to_planar(&outer);
    let ob = geom::bbox(&po);
    let mut rings = vec![po];
    for h in &poly.rings[1..] {
        let h = clean_source_ring(h);
        if h.len() < 3 {
            continue;
        }
        let mut u = geom::unwrap_path(&h, None);
        let k = (((ob[0] + ob[2]) * 0.5 - u[0][0]) / 360.0).round() * 360.0;
        for p in &mut u {
            p[0] += k;
        }
        rings.push(u);
    }
    crate::seam::split(&rings).into_iter().flat_map(cut_round).map(|rs| Part { feature, rings: rs }).collect()
}

/// A part inside the strip, cut in two along the prime meridian if it
/// reaches all the way round (see [`wraps_round`]).
fn cut_round(rings: Vec<Ring>) -> Vec<Vec<Ring>> {
    let mut parts = if wraps_round(&rings) { crate::seam::cut_at(&rings, 0.0) } else { vec![rings] };
    for p in &mut parts {
        for r in p.iter_mut() {
            pole_steps(r);
        }
    }
    parts
}

/// Points added along a stretch of a pole so that no step is as long as
/// half the globe: between two points 180° apart there is no shorter way
/// round, and the ring would be read as going round the other side. (Only
/// pieces of land around a pole that are cut along meridians have such
/// stretches. A cap's own run along the whole pole, from one side of the
/// antimeridian to the other, is left alone: it closes up to nothing.)
fn pole_steps(r: &mut Ring) {
    let mut i = 0;
    while i < r.len() {
        let (a, b) = (r[i], r[(i + 1) % r.len()]);
        let d = b[0] - a[0];
        if a[1] == b[1] && a[1].abs() == 90.0 && d.abs() >= 120.0 && d.abs() < 360.0 {
            let n = (d.abs() / 90.0).ceil() as usize;
            for k in 1..n {
                r.insert(i + k, [a[0] + d * k as f64 / n as f64, a[1]]);
            }
            i += n;
        } else {
            i += 1;
        }
    }
}

/// Does a part inside the strip reach all the way round and meet itself
/// along the antimeridian? (Land around a pole with something else at the
/// pole itself: a sea, another country.) As one face it would have the same
/// face on both sides of that stretch of the seam, which a face cannot
/// have, so such a part is cut in two along the prime meridian.
///
/// A cap over the pole also runs up one side of the seam and down the
/// other, but those two runs meet at the pole: a zero-width spike, removed
/// later, that leaves one ring around the pole.
fn wraps_round(rings: &[Ring]) -> bool {
    // latitude spans of the segments on x = -180 and on x = 180
    let mut side: [Vec<(f64, f64)>; 2] = [Vec::new(), Vec::new()];
    for r in rings {
        let n = r.len();
        for i in 0..n {
            let (a, b) = (r[i], r[(i + 1) % n]);
            if a[0] == b[0] && a[0].abs() == 180.0 && a[1] != b[1] {
                side[(a[0] > 0.0) as usize].push((a[1].min(b[1]), a[1].max(b[1])));
            }
        }
    }
    side[0].iter().any(|w| {
        side[1].iter().any(|e| {
            let (lo, hi) = (w.0.max(e.0), w.1.min(e.1));
            hi - lo > 1e-9 && hi < 90.0 - 1e-9 && lo > -90.0 + 1e-9
        })
    })
}

fn clean_ring(r: &mut Ring) {
    r.dedup();
    while r.len() > 1 && r.first() == r.last() {
        r.pop();
    }
    // zero-width spikes a, b, a
    let mut changed = true;
    while changed && r.len() >= 3 {
        changed = false;
        let n = r.len();
        for i in 0..n {
            if r[(i + n - 1) % n] == r[(i + 1) % n] {
                let (tip, base) = (i, (i + 1) % n);
                let (hi, lo) = if tip > base { (tip, base) } else { (base, tip) };
                r.remove(hi);
                r.remove(lo);
                changed = true;
                break;
            }
        }
        r.dedup();
        while r.len() > 1 && r.first() == r.last() {
            r.pop();
        }
    }
}

fn tidy(parts: &mut Vec<Part>) {
    for p in parts.iter_mut() {
        for r in p.rings.iter_mut() {
            clean_ring(r);
        }
        let outer_ok = p.rings.first().is_some_and(|r| r.len() >= 3);
        if !outer_ok {
            p.rings.clear();
        } else {
            let outer = p.rings.remove(0);
            p.rings.retain(|r| r.len() >= 3);
            p.rings.insert(0, outer);
        }
    }
    parts.retain(|p| !p.rings.is_empty());
}

/// Points on the antimeridian (lon exactly ±180) of either side are the
/// same places, but clustering sees +180 and -180 as far apart: line up
/// their latitudes first. Latitudes within `tol` of each other form a group
/// and take an anchor's latitude when one is among them (a node of the
/// untouched topology around a local rebuild), else their most common
/// value (the exact one an uncut neighbour kept, rather than a boolean
/// result's rounding of it).
fn unify_seam(parts: &mut [Part], tol: f64, anchors: &[P2]) {
    let pinned: std::collections::HashSet<u64> = anchors.iter().filter(|q| q[0].abs() == 180.0).map(|q| q[1].to_bits()).collect();
    let mut lats: Vec<f64> = parts.iter().flat_map(|p| p.rings.iter().flatten()).filter(|q| q[0].abs() == 180.0).map(|q| q[1]).collect();
    lats.extend(anchors.iter().filter(|q| q[0].abs() == 180.0).map(|q| q[1]));
    if lats.len() < 2 {
        return;
    }
    lats.sort_by(|a, b| a.total_cmp(b));
    // groups of consecutive latitudes closer than `tol`; each to its mode
    let mut map: HashMap<u64, f64> = HashMap::new();
    let mut i = 0;
    while i < lats.len() {
        let mut j = i + 1;
        while j < lats.len() && lats[j] - lats[j - 1] <= tol {
            j += 1;
        }
        let group = &lats[i..j];
        if group.first() != group.last() {
            let (mut best, mut best_n, mut k) = (group[0], 0, 0);
            while k < group.len() {
                let n = group[k..].iter().take_while(|v| **v == group[k]).count();
                if n > best_n {
                    (best, best_n) = (group[k], n);
                }
                k += n;
            }
            if let Some(a) = group.iter().find(|v| pinned.contains(&v.to_bits())) {
                best = *a;
            }
            for v in group {
                map.insert(v.to_bits(), best);
            }
        }
        i = j;
    }
    if map.is_empty() {
        return;
    }
    for part in parts.iter_mut() {
        for ring in part.rings.iter_mut() {
            for q in ring.iter_mut() {
                if q[0].abs() == 180.0 {
                    if let Some(v) = map.get(&q[1].to_bits()) {
                        q[1] = *v;
                    }
                }
            }
        }
    }
}

/// Step 2: cluster vertices within `tol` onto the first one seen
/// (`anchors` are preferred positions seeded before any vertex).
fn cluster(parts: &mut [Part], tol: f64, anchors: &[P2], rep: &mut RepairReport) {
    let cell = tol.max(1e-12);
    let mut grid: HashMap<(i64, i64), Vec<P2>> = HashMap::new();
    let key = |p: &P2| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    for a in anchors {
        grid.entry(key(a)).or_default().push(*a);
    }
    for part in parts.iter_mut() {
        for ring in part.rings.iter_mut() {
            for p in ring.iter_mut() {
                let (kx, ky) = key(p);
                let mut best: Option<(f64, P2)> = None;
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        if let Some(list) = grid.get(&(kx + dx, ky + dy)) {
                            for a in list {
                                let d = ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).sqrt();
                                if d <= tol && best.is_none_or(|b| d < b.0) {
                                    best = Some((d, *a));
                                }
                            }
                        }
                    }
                }
                match best {
                    Some((d, a)) => {
                        if d > 0.0 {
                            rep.snapped_vertices += 1;
                            rep.max_shift_m = rep.max_shift_m.max(d * M_PER_DEG);
                        }
                        *p = a;
                    }
                    None => grid.entry((kx, ky)).or_default().push(*p),
                }
            }
        }
    }
}

/// `forced` and then those `prefer` points not within `tol` of one
/// already taken (two anchors a hair apart would split the vertices that
/// belong at one point between them).
fn with_preferred(forced: &[P2], prefer: &[P2], tol: f64) -> Vec<P2> {
    let cell = tol.max(1e-12);
    let key = |p: &P2| ((p[0] / cell).floor() as i64, (p[1] / cell).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<P2>> = HashMap::new();
    let mut out: Vec<P2> = forced.to_vec();
    for p in forced {
        grid.entry(key(p)).or_default().push(*p);
    }
    for p in prefer {
        let (kx, ky) = key(p);
        let near = (-1..=1).any(|dx| (-1..=1).any(|dy| grid.get(&(kx + dx, ky + dy)).is_some_and(|l| l.iter().any(|a| ((a[0] - p[0]).powi(2) + (a[1] - p[1]).powi(2)).sqrt() <= tol))));
        if !near {
            grid.entry((kx, ky)).or_default().push(*p);
            out.push(*p);
        }
    }
    out
}

/// Put each anchor lying on a ring's segment (within `tol`, not at its
/// ends) into that ring as a vertex. The boolean kernel drops points that
/// are collinear to within its rounding, but a forced junction (a node of
/// the untouched topology around a local rebuild) must stay a node, or one
/// rebuilt edge would run past it along two different neighbours.
fn insert_anchors(parts: &mut [Part], anchors: &[P2], tol: f64, rep: &mut RepairReport) {
    if !anchors.is_empty() {
        snap_points(parts, anchors, tol, rep);
    }
}

/// Step 3/5: insert vertices lying within `tol` of other rings' segments.
fn snap_to_segments(parts: &mut [Part], tol: f64, rep: &mut RepairReport) {
    let mut verts: Vec<P2> = parts.iter().flat_map(|p| p.rings.iter().flatten().copied()).collect();
    verts.sort_by(|a, b| a.partial_cmp(b).unwrap());
    verts.dedup();
    snap_points(parts, &verts, tol, rep);
}

/// Insert each of `points` into every ring segment it lies within `tol`
/// of (not at the segment's ends). Segments are found through a grid
/// sized from the data, so finely drawn rings stay fast.
fn snap_points(parts: &mut [Part], points: &[P2], tol: f64, rep: &mut RepairReport) {
    let mut segs: Vec<(P2, P2)> = Vec::new();
    let mut at: Vec<(usize, usize, usize)> = Vec::new();
    for (pi, part) in parts.iter().enumerate() {
        for (ri, ring) in part.rings.iter().enumerate() {
            let n = ring.len();
            for si in 0..n {
                segs.push((ring[si], ring[(si + 1) % n]));
                at.push((pi, ri, si));
            }
        }
    }
    let mut grid = crate::seggrid::SegGrid::new(&segs, tol * 2.0, false);
    // insertions per segment: (t, point)
    let mut ins: HashMap<(usize, usize, usize), Vec<(f64, P2)>> = HashMap::new();
    let mut near = Vec::new();
    for v in points {
        grid.near(*v, *v, &mut near);
        for &id in &near {
            let (a, b) = segs[id as usize];
            if *v == a || *v == b {
                continue;
            }
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let len2 = dx * dx + dy * dy;
            if len2 == 0.0 {
                continue;
            }
            let t = ((v[0] - a[0]) * dx + (v[1] - a[1]) * dy) / len2;
            if t <= 0.0 || t >= 1.0 {
                continue;
            }
            let q = [a[0] + dx * t, a[1] + dy * t];
            let d = ((q[0] - v[0]).powi(2) + (q[1] - v[1]).powi(2)).sqrt();
            if d <= tol {
                ins.entry(at[id as usize]).or_default().push((t, *v));
                if d > 0.0 {
                    rep.max_shift_m = rep.max_shift_m.max(d * M_PER_DEG);
                }
            }
        }
    }
    if ins.is_empty() {
        return;
    }
    for (pi, part) in parts.iter_mut().enumerate() {
        for (ri, ring) in part.rings.iter_mut().enumerate() {
            let n = ring.len();
            let mut out = Vec::with_capacity(n);
            for si in 0..n {
                out.push(ring[si]);
                if let Some(list) = ins.get_mut(&(pi, ri, si)) {
                    list.sort_by(|x, y| x.0.partial_cmp(&y.0).unwrap());
                    for (_, p) in list.iter() {
                        if out.last() != Some(p) {
                            out.push(*p);
                            rep.inserted_vertices += 1;
                        }
                    }
                }
            }
            *ring = out;
        }
    }
}

/// Step 4: each part minus earlier overlapping parts.
fn resolve_overlaps(parts: Vec<Part>, opts: &RepairOptions, rep: &mut RepairReport) -> Vec<Part> {
    let r = opts.radius_km;
    let mut done: Vec<(usize, geo::MultiPolygon<f64>, [f64; 4])> = Vec::new();
    let mut idx = GridIndex::default();
    let mut out = Vec::new();
    let mut feature_area_after: HashMap<usize, f64> = HashMap::new();
    for part in parts {
        let bb = ring_bbox(&part.rings);
        let me = geo::MultiPolygon::new(vec![to_geo(&part.rings)]);
        let before = mp_area(&me, r);
        let mut cur = me.clone();
        for id in idx.query(bb) {
            let (f, other, ob) = &done[id as usize];
            if *f == part.feature || ob[0] > bb[2] || bb[0] > ob[2] || ob[1] > bb[3] || bb[1] > ob[3] {
                continue;
            }
            let lost = cur.intersection(other);
            let a = mp_area(&lost, r);
            if a > 1e-6 {
                let c = lost.0.first().and_then(|p| p.exterior().coords().next().copied());
                rep.overlaps.push(Overlap {
                    kept_by: *f,
                    removed_from: part.feature,
                    area_km2: a,
                    at: c.map(|c| LonLat::new(geom::wrap_lon(c.x), c.y)).unwrap_or_default(),
                });
                cur = cur.difference(other);
            }
        }
        let after = mp_area(&cur, r);
        *feature_area_after.entry(part.feature).or_default() += after;
        let _ = before;
        let id = done.len() as u64;
        idx.insert(id, bb);
        for p in &cur.0 {
            out.push(Part { feature: part.feature, rings: from_geo(p) });
        }
        done.push((part.feature, cur, bb));
    }
    out
}

/// Run the full repair pipeline. `feature_of[i]` is the feature that
/// source polygon `i` belongs to.
pub fn repair(polys: &[SourcePolygon], feature_of: &[usize], opts: &RepairOptions) -> Result<Repaired, RepairReport> {
    let parts: Vec<Part> = polys.iter().zip(feature_of).flat_map(|(p, f)| to_parts(p, *f)).collect();
    repair_parts(parts, feature_of, opts)
}

/// Like [`repair`], for polygons that are already cut to the strip of
/// longitudes ±180: rings in planar degrees (outer first), a side on the
/// antimeridian at exactly +180 or -180. This is what the overlay works
/// in. Nothing is read back from geographic coordinates, where the two
/// sides of the antimeridian are the same place and a shape that touches
/// both (land around a pole with a lake on the antimeridian) cannot be
/// told from a different one.
pub fn repair_strip(polys: &[Vec<Vec<P2>>], feature_of: &[usize], opts: &RepairOptions) -> Result<Repaired, RepairReport> {
    let parts: Vec<Part> = polys.iter().zip(feature_of).flat_map(|(rs, f)| cut_round(rs.clone()).into_iter().map(|rings| Part { feature: *f, rings })).collect();
    repair_parts(parts, feature_of, opts)
}

fn repair_parts(mut parts: Vec<Part>, feature_of: &[usize], opts: &RepairOptions) -> Result<Repaired, RepairReport> {
    let mut rep = RepairReport::default();
    let r = opts.radius_km;
    let tol = (opts.snap_m / M_PER_DEG).max(1e-9);

    let prof = std::env::var_os("ATLAS_PROFILE").is_some();
    let mut tp = std::time::Instant::now();
    let mut lap = |name: &str| {
        if prof {
            eprintln!("  repair {name}: {:.1} ms", tp.elapsed().as_secs_f64() * 1e3);
        }
        tp = std::time::Instant::now();
    };
    let mut area_before: HashMap<usize, f64> = HashMap::new();
    for p in &parts {
        *area_before.entry(p.feature).or_default() += parts_area(&p.rings, r);
    }
    lap("t_parts");
    let forced: Vec<P2> = opts.forced_junctions.iter().map(|p| [p.lon, p.lat]).collect();
    unify_seam(&mut parts, tol, &forced);
    let seeds = with_preferred(&forced, &opts.prefer, tol);
    cluster(&mut parts, tol, &seeds, &mut rep);
    tidy(&mut parts);
    insert_anchors(&mut parts, &forced, tol, &mut rep);
    snap_to_segments(&mut parts, tol, &mut rep);
    tidy(&mut parts);
    // the boolean kernel rounds every output coordinate to its internal
    // grid (~1e-8°); snap them back to the exact pre-boolean vertices and
    // let new intersection points be inserted into the neighbours' borders
    let mut exact: Vec<P2> = parts.iter().flat_map(|p| p.rings.iter().flatten().copied()).collect();
    exact.sort_by(|a, b| a.partial_cmp(b).unwrap());
    exact.dedup();
    lap("t_snap1");
    let mut parts = if opts.resolve_overlaps { resolve_overlaps(parts, opts, &mut rep) } else { parts };
    lap("t_overlap");
    let mut noise = RepairReport::default();
    cluster(&mut parts, 1e-6, &exact, &mut noise);
    tidy(&mut parts);
    snap_to_segments(&mut parts, 1e-6, &mut noise);
    tidy(&mut parts);
    rep.inserted_vertices += noise.inserted_vertices;

    lap("t_snap2");
    let features: std::collections::BTreeSet<usize> = feature_of.iter().copied().collect();
    let present: std::collections::BTreeSet<usize> = parts.iter().map(|p| p.feature).collect();
    rep.swallowed = features.difference(&present).copied().collect();

    let src: Vec<SourcePolygon> = parts
        .iter()
        .map(|p| SourcePolygon { rings: p.rings.iter().map(|r| r.iter().map(|q| LonLat::new(geom::wrap_lon(q[0]), q[1])).collect()).collect() })
        .collect();
    // keep the +180 edge: wrap_lon maps it to -180, which build() merges with the -180 side
    let (mut topo, faces, brep) = match build::build_with(&src, opts.first_id, &opts.forced_junctions) {
        Ok(x) => x,
        Err(b) => {
            rep.problems = b.problems.clone();
            rep.build = b;
            return Err(rep);
        }
    };
    rep.build = brep;
    let mut by_feature: HashMap<usize, Vec<FaceId>> = HashMap::new();
    let mut owner_of: HashMap<FaceId, usize> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        if let Some(f) = f {
            by_feature.entry(parts[i].feature).or_default().push(*f);
            owner_of.insert(*f, parts[i].feature);
        }
    }

    lap("t_build");
    // step 7: slivers
    if opts.fill_slivers {
        let water: Vec<FaceId> = topo.faces.iter().filter(|(id, f)| **id != BACKGROUND && f.kind == FaceKind::Water).map(|(id, _)| *id).collect();
        for w in water {
            let Some(face) = topo.faces.get(&w) else { continue };
            let area = topo.face_area(w, r);
            let perim_km: f64 = std::iter::once(&face.outer)
                .chain(face.holes.iter())
                .map(|ring| {
                    let c = topo.ring_coords(ring);
                    (0..c.len()).map(|i| geom::geo_dist_deg(c[i], c[(i + 1) % c.len()])).sum::<f64>()
                })
                .sum::<f64>()
                * M_PER_DEG
                / 1000.0;
            let thickness_m = if perim_km > 0.0 { 2.0 * area / perim_km * 1000.0 } else { 0.0 };
            if thickness_m > 3.0 * opts.snap_m {
                continue;
            }
            // neighbour with the longest shared border
            let mut shared: HashMap<FaceId, f64> = HashMap::new();
            for rr in std::iter::once(&face.outer).chain(face.holes.iter()).flatten() {
                let e = &topo.edges[&rr.edge];
                let other = if rr.rev { e.left } else { e.right };
                if other != BACKGROUND && topo.faces[&other].kind == FaceKind::Land {
                    let c = topo.edge_full(e);
                    *shared.entry(other).or_default() += (1..c.len()).map(|i| geom::geo_dist_deg(c[i - 1], c[i])).sum::<f64>();
                }
            }
            let Some((&land, _)) = shared.iter().max_by(|a, b| a.1.partial_cmp(b.1).unwrap().then(b.0.cmp(a.0))) else { continue };
            if topo.merge_faces(land, w).is_ok() {
                rep.slivers_filled += 1;
                rep.sliver_area_km2 += area;
            }
        }
        // build() already validated; only re-check if merging changed anything
        if rep.slivers_filled > 0 {
            if let Err(e) = topo.validate() {
                rep.problems.push(format!("填补缝隙后拓扑无效：{e}"));
                return Err(rep);
            }
        }
    }

    lap("t_sliver");
    // area changes per feature
    let mut after: HashMap<usize, f64> = HashMap::new();
    for (f, fs) in &by_feature {
        after.insert(*f, fs.iter().filter(|x| topo.faces.contains_key(*x)).map(|x| topo.face_area(*x, r)).sum());
    }
    let mut changes: Vec<(usize, f64, f64)> = area_before.iter().map(|(f, b)| (*f, *b, after.get(f).copied().unwrap_or(0.0))).collect();
    changes.sort_by(|a, b| {
        let rel = |x: &(usize, f64, f64)| if x.1 > 0.0 { (x.2 - x.1).abs() / x.1 } else { 0.0 };
        rel(b).partial_cmp(&rel(a)).unwrap()
    });
    rep.area_changes = changes;
    by_feature.retain(|_, fs| {
        fs.retain(|f| topo.faces.contains_key(f));
        !fs.is_empty()
    });
    Ok(Repaired { topo, faces: by_feature, report: rep })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{grid_world, Rng};

    fn clean_polys() -> Vec<SourcePolygon> {
        let w = grid_world(6, 4, 4, 2);
        w.topo
            .faces
            .iter()
            .filter(|(id, _)| **id != BACKGROUND)
            .map(|(_, f)| {
                let mut rings = vec![w.topo.ring_coords(&f.outer)];
                rings.extend(f.holes.iter().map(|h| w.topo.ring_coords(h)));
                SourcePolygon { rings }
            })
            .collect()
    }

    fn total_area(polys: &[SourcePolygon], r: f64) -> f64 {
        polys.iter().map(|p| geom::spherical_area(&geom::ring_to_planar(&p.rings[0]), r).abs()).sum()
    }

    #[test]
    fn repairs_jitter_t_junctions_and_overlaps() {
        for seed in 1..=25 {
            repair_dirty_world(seed);
        }
    }

    fn repair_dirty_world(seed: u64) {
        let clean = clean_polys();
        let n = clean.len();
        let area0 = total_area(&clean, 6371.0088);
        let mut rng = Rng::new(seed);
        let mut dirty = clean.clone();
        // independent jitter of every vertex (~33 m): shared borders no longer coincide
        for p in dirty.iter_mut() {
            for r in p.rings.iter_mut() {
                for q in r.iter_mut() {
                    q.lon += rng.range(-0.0003, 0.0003);
                    q.lat += rng.range(-0.0003, 0.0003);
                }
            }
        }
        // T-junctions: collinear extra vertices on polygon 0 only
        let r0 = &mut dirty[0].rings[0];
        let mut with_mid = Vec::new();
        for i in 0..r0.len() {
            let (a, b) = (r0[i], r0[(i + 1) % r0.len()]);
            with_mid.push(a);
            with_mid.push(LonLat::new((a.lon + b.lon) * 0.5, (a.lat + b.lat) * 0.5));
        }
        *r0 = with_mid;
        // a genuine overlap: push one vertex of polygon 5 half a degree outwards
        let ring = &mut dirty[5].rings[0];
        let c = ring.iter().fold((0.0, 0.0), |s, p| (s.0 + p.lon, s.1 + p.lat));
        let c = (c.0 / ring.len() as f64, c.1 / ring.len() as f64);
        let far = (0..ring.len()).max_by(|a, b| {
            let d = |p: &LonLat| (p.lon - c.0).powi(2) + (p.lat - c.1).powi(2);
            d(&ring[*a]).partial_cmp(&d(&ring[*b])).unwrap()
        }).unwrap();
        let p = ring[far];
        ring[far] = LonLat::new(p.lon + (p.lon - c.0).signum() * 0.5, p.lat + (p.lat - c.1).signum() * 0.5);

        // strict mode refuses this data
        assert!(build::build(&dirty).is_err(), "strict build must reject dirty data");

        let feature_of: Vec<usize> = (0..n).collect();
        let out = repair(&dirty, &feature_of, &RepairOptions::default()).unwrap_or_else(|r| panic!("seed {seed}: {:?}", r.problems));
        out.topo.validate().unwrap();
        let rep = &out.report;
        assert!(rep.swallowed.is_empty(), "{:?}", rep.swallowed);
        assert_eq!(out.faces.len(), n);
        assert!(rep.snapped_vertices > 100, "{rep:?}");
        assert!(rep.max_shift_m <= 100.0 * 1.5, "{}", rep.max_shift_m);
        assert!(rep.overlaps.iter().any(|o| o.removed_from == 5 || o.kept_by == 5), "{:?}", rep.overlaps);
        // shared borders were rebuilt: plenty of land-land edges
        let internal = out.topo.edges.values().filter(|e| e.left != BACKGROUND && e.right != BACKGROUND).count();
        assert!(internal >= 30, "{internal}");
        // the repaired world covers essentially the same area
        let area1: f64 = out.topo.faces.keys().filter(|f| **f != BACKGROUND).map(|f| out.topo.face_area(*f, 6371.0088)).sum();
        assert!((area1 - area0).abs() / area0 < 0.005, "{area1} vs {area0}");
    }

    #[test]
    fn thin_gaps_are_filled_real_lakes_are_kept() {
        let sq = |x: f64, y: f64, w: f64, h: f64| vec![LonLat::new(x, y), LonLat::new(x + w, y), LonLat::new(x + w, y + h), LonLat::new(x, y + h)];
        // country A with two 1°×1° holes; country B fills the first hole but
        // stops 50 m short of its east side (an enclosed sliver); the second
        // hole stays empty (a real lake)
        let a = SourcePolygon { rings: vec![sq(0.0, 0.0, 6.0, 4.0), sq(1.0, 1.0, 1.0, 1.0), sq(4.0, 1.0, 1.0, 1.0)] };
        let b = SourcePolygon { rings: vec![sq(1.0, 1.0, 1.0 - 0.00045, 1.0)] };
        let opts = RepairOptions { snap_m: 20.0, ..Default::default() };
        let out = repair(&[a, b], &[0, 1], &opts).unwrap_or_else(|r| panic!("{:?}", r.problems));
        out.topo.validate().unwrap();
        assert_eq!(out.report.slivers_filled, 1, "{:?}", out.report);
        let lakes: Vec<_> = out.topo.faces.iter().filter(|(id, f)| **id != BACKGROUND && f.kind == FaceKind::Water).collect();
        assert_eq!(lakes.len(), 1, "the real lake must stay");
        assert!(out.topo.face_contains(*lakes[0].0, LonLat::new(4.5, 1.5)));
        // the sliver went to the neighbour with the longest shared border
        let p = LonLat::new(2.0 - 0.0002, 1.5);
        let f = out.topo.face_at(p);
        assert!(out.faces[&0].contains(&f) || out.faces[&1].contains(&f));
    }
}

#[cfg(test)]
mod polar {
    use super::*;
    #[test]
    fn polar_cap_survives_repair() {
        let mut ring = vec![LonLat::new(-180.0, -70.0)];
        for i in 1..36 {
            ring.push(LonLat::new(-180.0 + i as f64 * 10.0, -70.0));
        }
        ring.extend([LonLat::new(180.0, -70.0), LonLat::new(180.0, -90.0), LonLat::new(-180.0, -90.0)]);
        let poly = SourcePolygon { rings: vec![ring] };
        // the parts must cover the cap south of -70°, not the region north of it
        for p in to_parts(&poly, 0) {
            assert!(p.rings[0].iter().all(|q| q[1] <= -70.0 + 1e-9), "{:?}", p.rings[0]);
        }
        let out = repair(&[poly], &[0], &RepairOptions::default()).unwrap_or_else(|r| panic!("{:?}", r.problems));
        out.topo.validate().unwrap();
        let r = 6371.0088;
        let exact = 2.0 * std::f64::consts::PI * r * r * (1.0 - 70f64.to_radians().sin());
        let a: f64 = out.faces[&0].iter().map(|f| out.topo.face_area(*f, r)).sum();
        assert!((a - exact).abs() / exact < 1e-9, "{a} vs {exact}");
    }
}
