//! Geographic primitives and planar algorithms on the lon/lat cylinder.
//!
//! Geometry semantics (see docs/ARCHITECTURE.md, ADR-2):
//! * coordinates are degrees, longitude normalised to [-180, 180);
//! * the edge between two consecutive points takes the direction whose
//!   longitude difference is < 180° and is a straight line in lon/lat;
//! * a ring may therefore cross the antimeridian freely. "Unwrapping" a ring
//!   produces continuous longitudes (possibly outside ±180) on which ordinary
//!   planar algorithms work.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct LonLat {
    pub lon: f64,
    pub lat: f64,
}

impl LonLat {
    pub const fn new(lon: f64, lat: f64) -> Self {
        Self { lon, lat }
    }
    pub fn normalized(self) -> Self {
        Self { lon: wrap_lon(self.lon), lat: self.lat.clamp(-90.0, 90.0) }
    }
}

/// Planar point in unwrapped lon/lat degrees.
pub type P2 = [f64; 2];

/// Wrap a longitude into [-180, 180).
pub fn wrap_lon(lon: f64) -> f64 {
    let mut l = (lon + 180.0).rem_euclid(360.0) - 180.0;
    if l >= 180.0 {
        l -= 360.0;
    }
    l
}

/// Signed shortest longitude difference `b - a`, in (-180, 180].
pub fn lon_delta(a: f64, b: f64) -> f64 {
    let d = wrap_lon(b - a);
    if d == -180.0 {
        180.0
    } else {
        d
    }
}

/// Unwrap a sequence of points so that consecutive longitudes are continuous.
/// The first point keeps `start_lon` (or its own longitude).
pub fn unwrap_path(pts: &[LonLat], start_lon: Option<f64>) -> Vec<P2> {
    let mut out = Vec::with_capacity(pts.len());
    let mut prev: Option<(f64, f64)> = None; // (raw lon, unwrapped lon)
    for p in pts {
        let u = match prev {
            None => start_lon.unwrap_or(p.lon),
            Some((raw, un)) => un + lon_delta(raw, p.lon),
        };
        out.push([u, p.lat]);
        prev = Some((p.lon, u));
    }
    out
}

/// Total longitudinal winding of a closed ring (≈0 for ordinary rings,
/// ±360 for rings enclosing a pole).
pub fn ring_winding(pts: &[LonLat]) -> f64 {
    let n = pts.len();
    (0..n).map(|i| lon_delta(pts[i].lon, pts[(i + 1) % n].lon)).sum()
}

/// A ring around a pole, turned to begin where it crosses the antimeridian
/// (a point is added there unless a vertex is on it already). Closed through
/// the pole from there ([`ring_to_planar`]) the cap lies in one piece inside
/// longitudes ±180, with nothing left over on the far side to be cut off.
/// Other rings come back as they are.
pub fn start_at_seam(pts: &[LonLat]) -> Vec<LonLat> {
    let w = ring_winding(pts);
    let n = pts.len();
    if n < 3 || w.abs() <= 180.0 {
        return pts.to_vec();
    }
    // eastward rings arrive at +180 and go on from -180; westward the reverse
    let (arrive, leave) = if w > 0.0 { (180.0, -180.0) } else { (-180.0, 180.0) };
    if let Some(i) = (0..n).find(|i| pts[*i].lon.abs() == 180.0) {
        let mut out: Vec<LonLat> = pts[i..].iter().chain(&pts[..i]).copied().collect();
        out[0].lon = leave;
        return out;
    }
    for i in 0..n {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let d = lon_delta(a.lon, b.lon);
        // the step whose unwrapped longitude passes the seam the ring's way round
        if d * w > 0.0 && (a.lon + d - arrive) * (a.lon - arrive) < 0.0 {
            let t = (arrive - a.lon) / d;
            let mut out = vec![LonLat::new(leave, a.lat + (b.lat - a.lat) * t)];
            out.extend(pts[i + 1..].iter().chain(&pts[..=i]).copied());
            return out;
        }
    }
    pts.to_vec()
}

/// Convert a closed ring (first point not repeated) into a planar polygon
/// describing the region on the ring's *left*. Rings that wind around a pole
/// are closed through that pole along lat = ±90.
pub fn ring_to_planar(pts: &[LonLat]) -> Vec<P2> {
    let mut out = unwrap_path(pts, None);
    if pts.len() < 3 {
        return out;
    }
    let w = ring_winding(pts);
    if w.abs() > 180.0 {
        let first = out[0];
        let shift = if w > 0.0 { 360.0 } else { -360.0 };
        // eastward winding keeps the north pole on the left
        let pole = if w > 0.0 { 90.0 } else { -90.0 };
        out.push([first[0] + shift, first[1]]);
        out.push([first[0] + shift, pole]);
        out.push([first[0], pole]);
    }
    out
}

/// Shoelace signed area (positive = counter-clockwise).
pub fn signed_area(poly: &[P2]) -> f64 {
    let n = poly.len();
    let mut a = 0.0;
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

pub fn bbox(poly: &[P2]) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in poly {
        b[0] = b[0].min(p[0]);
        b[1] = b[1].min(p[1]);
        b[2] = b[2].max(p[0]);
        b[3] = b[3].max(p[1]);
    }
    b
}

/// Even-odd point-in-polygon test on a planar polygon.
pub fn point_in_poly(p: P2, poly: &[P2]) -> bool {
    let n = poly.len();
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let a = poly[i];
        let b = poly[j];
        if (a[1] > p[1]) != (b[1] > p[1]) {
            let x = (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0];
            if p[0] < x {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Point-in-polygon accounting for longitude periodicity (tests p, p±360).
pub fn point_in_poly_wrapped(p: P2, poly: &[P2], bb: &[f64; 4]) -> bool {
    for k in [0.0, 360.0, -360.0, 720.0, -720.0] {
        let q = [p[0] + k, p[1]];
        if q[0] >= bb[0] && q[0] <= bb[2] && q[1] >= bb[1] && q[1] <= bb[3] && point_in_poly(q, poly) {
            return true;
        }
    }
    false
}

fn orient(a: P2, b: P2, c: P2) -> f64 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

/// Proper or improper intersection of closed segments ab and cd.
pub fn segments_intersect(a: P2, b: P2, c: P2, d: P2) -> bool {
    let d1 = orient(c, d, a);
    let d2 = orient(c, d, b);
    let d3 = orient(a, b, c);
    let d4 = orient(a, b, d);
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    let on = |p: P2, q: P2, r: P2| {
        r[0] >= p[0].min(q[0]) && r[0] <= p[0].max(q[0]) && r[1] >= p[1].min(q[1]) && r[1] <= p[1].max(q[1])
    };
    (d1 == 0.0 && on(c, d, a)) || (d2 == 0.0 && on(c, d, b)) || (d3 == 0.0 && on(a, b, c)) || (d4 == 0.0 && on(a, b, d))
}

/// Parameter `t` along ab and the point where segment ab crosses cd (planar).
pub fn segment_intersection(a: P2, b: P2, c: P2, d: P2) -> Option<(f64, P2)> {
    let r = [b[0] - a[0], b[1] - a[1]];
    let s = [d[0] - c[0], d[1] - c[1]];
    let den = r[0] * s[1] - r[1] * s[0];
    if den.abs() < 1e-300 {
        return None;
    }
    let qp = [c[0] - a[0], c[1] - a[1]];
    let t = (qp[0] * s[1] - qp[1] * s[0]) / den;
    let u = (qp[0] * r[1] - qp[1] * r[0]) / den;
    ((0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u)).then(|| (t, [a[0] + r[0] * t, a[1] + r[1] * t]))
}

/// Segment intersection on the cylinder. Inputs are geographic points; each
/// segment is unwrapped along its short direction and the second is tested
/// at the longitude shifts that can overlap the first.
pub fn geo_segments_intersect(a: LonLat, b: LonLat, c: LonLat, d: LonLat) -> bool {
    let a2 = [a.lon, a.lat];
    let b2 = [a.lon + lon_delta(a.lon, b.lon), b.lat];
    let c0 = [c.lon, c.lat];
    let d0 = [c.lon + lon_delta(c.lon, d.lon), d.lat];
    for k in [-360.0, 0.0, 360.0] {
        let c2 = [c0[0] + k, c0[1]];
        let d2 = [d0[0] + k, d0[1]];
        if a2[0].max(b2[0]) < c2[0].min(d2[0]) || c2[0].max(d2[0]) < a2[0].min(b2[0]) {
            continue;
        }
        if segments_intersect(a2, b2, c2, d2) {
            return true;
        }
    }
    false
}

/// Closest point on segment ab to p (all geographic). Returns (point, distance in degrees, t).
pub fn closest_on_segment(p: LonLat, a: LonLat, b: LonLat) -> (LonLat, f64, f64) {
    // work in a frame centred on p so wrapping is irrelevant
    let ax = lon_delta(p.lon, a.lon);
    let bx = ax + lon_delta(a.lon, b.lon);
    let (ay, by) = (a.lat - p.lat, b.lat - p.lat);
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx * dx + dy * dy;
    let t = if len2 > 0.0 { (-(ax * dx + ay * dy) / len2).clamp(0.0, 1.0) } else { 0.0 };
    let x = ax + t * dx;
    let y = ay + t * dy;
    let q = LonLat::new(wrap_lon(p.lon + x), p.lat + y);
    (q, (x * x + y * y).sqrt(), t)
}

/// Distance in degrees (planar, wrap-aware) – used for snapping tolerance only.
pub fn geo_dist_deg(a: LonLat, b: LonLat) -> f64 {
    let dx = lon_delta(a.lon, b.lon);
    let dy = b.lat - a.lat;
    (dx * dx + dy * dy).sqrt()
}

/// Exact area on a sphere of radius `r` of a planar lon/lat polygon whose
/// edges are straight in lon/lat: A = r² ∮ -sin φ dλ (Green's theorem).
/// Positive for counter-clockwise polygons.
pub fn spherical_area(poly: &[P2], r: f64) -> f64 {
    let n = poly.len();
    let mut s = 0.0;
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        let dl = (q[0] - p[0]).to_radians();
        let (f1, f2) = (p[1].to_radians(), q[1].to_radians());
        // (cos f1 - cos f2) / (f2 - f1) = sin(mid) * sin(h) / h, written
        // without the cancellation that cost ~1 km² per nearly horizontal
        // segment (e.g. a parallel lifted 1e-8° by a boolean op)
        let h = 0.5 * (f2 - f1);
        let sinc = if h == 0.0 { 1.0 } else { h.sin() / h };
        let integral = dl * (0.5 * (f1 + f2)).sin() * sinc;
        s -= integral;
    }
    s * r * r
}

/// Is a closed planar ring simple (no non-adjacent segment intersections)?
pub fn ring_is_simple(poly: &[P2]) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        for j in (i + 1)..n {
            if j == i || (j + 1) % n == i || j == (i + 1) % n {
                continue;
            }
            let (c, d) = (poly[j], poly[(j + 1) % n]);
            if segments_intersect(a, b, c, d) {
                return false;
            }
        }
    }
    true
}

/// Is an open polyline simple?
pub fn path_is_simple(path: &[P2]) -> bool {
    let n = path.len();
    for i in 0..n.saturating_sub(1) {
        for j in (i + 2)..n.saturating_sub(1) {
            if segments_intersect(path[i], path[i + 1], path[j], path[j + 1]) {
                return false;
            }
        }
    }
    true
}

/// Find a good interior point of a polygon with holes (used for labels):
/// scan several latitudes and take the midpoint of the widest inside-interval
/// weighted by cos(lat). Returns (point, width_deg).
pub fn interior_point(outer: &[P2], holes: &[Vec<P2>]) -> Option<(P2, f64)> {
    let bb = bbox(outer);
    if !bb[0].is_finite() {
        return None;
    }
    let mut best: Option<(P2, f64)> = None;
    let steps = 16;
    for s in 0..steps {
        let y = bb[1] + (bb[3] - bb[1]) * (s as f64 + 0.5) / steps as f64;
        let mut xs: Vec<f64> = Vec::new();
        for ring in std::iter::once(outer).chain(holes.iter().map(|h| h.as_slice())) {
            let n = ring.len();
            for i in 0..n {
                let a = ring[i];
                let b = ring[(i + 1) % n];
                if (a[1] > y) != (b[1] > y) {
                    xs.push(a[0] + (b[0] - a[0]) * (y - a[1]) / (b[1] - a[1]));
                }
            }
        }
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        for pair in xs.chunks(2) {
            if pair.len() == 2 {
                let w = (pair[1] - pair[0]) * y.to_radians().cos();
                if best.map_or(true, |b| w > b.1) {
                    best = Some(([(pair[0] + pair[1]) * 0.5, y], w));
                }
            }
        }
    }
    best
}

/// Sutherland–Hodgman against x >= c (keep_greater) or x <= c.
pub fn clip_half(poly: &[P2], c: f64, keep_greater: bool) -> Vec<P2> {
    let inside = |p: &P2| if keep_greater { p[0] >= c } else { p[0] <= c };
    let mut out = Vec::with_capacity(poly.len() + 2);
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let (ia, ib) = (inside(&a), inside(&b));
        if ia {
            out.push(a);
        }
        if ia != ib {
            let t = (c - a[0]) / (b[0] - a[0]);
            out.push([c, a[1] + (b[1] - a[1]) * t]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_and_delta() {
        assert_eq!(wrap_lon(180.0), -180.0);
        assert_eq!(wrap_lon(190.0), -170.0);
        assert_eq!(wrap_lon(-190.0), 170.0);
        assert_eq!(lon_delta(170.0, -170.0), 20.0);
        assert_eq!(lon_delta(-170.0, 170.0), -20.0);
    }

    #[test]
    fn unwrap_across_antimeridian() {
        let pts = [LonLat::new(170.0, 0.0), LonLat::new(-170.0, 0.0), LonLat::new(-160.0, 10.0)];
        let u = unwrap_path(&pts, None);
        assert_eq!(u[1][0], 190.0);
        assert_eq!(u[2][0], 200.0);
    }

    #[test]
    fn spherical_area_matches_analytic() {
        // lon/lat box: area = r² Δλ (sin φ2 - sin φ1)
        let r = 6371.0;
        let poly = [[0.0, 10.0], [30.0, 10.0], [30.0, 40.0], [0.0, 40.0]];
        let exact = r * r * 30f64.to_radians() * (40f64.to_radians().sin() - 10f64.to_radians().sin());
        let a = spherical_area(&poly, r);
        assert!((a - exact).abs() / exact < 1e-12, "{a} vs {exact}");
        // whole sphere area via a polar cap pair: north hemisphere = 2πr²
        let ring: Vec<LonLat> = (0..36).map(|i| LonLat::new(wrap_lon(i as f64 * 10.0), 0.0)).collect();
        let planar = ring_to_planar(&ring);
        let hemi = spherical_area(&planar, r);
        let exact = 2.0 * std::f64::consts::PI * r * r;
        assert!((hemi - exact).abs() / exact < 1e-12, "{hemi} vs {exact}");
    }

    #[test]
    fn pip_wrapped() {
        let ring = [LonLat::new(170.0, -10.0), LonLat::new(-170.0, -10.0), LonLat::new(-170.0, 10.0), LonLat::new(170.0, 10.0)];
        let p = ring_to_planar(&ring);
        let bb = bbox(&p);
        assert!(point_in_poly_wrapped([-175.0, 0.0], &p, &bb));
        assert!(point_in_poly_wrapped([175.0, 0.0], &p, &bb));
        assert!(!point_in_poly_wrapped([0.0, 0.0], &p, &bb));
    }

    #[test]
    fn segment_intersection_across_seam() {
        let hit = geo_segments_intersect(
            LonLat::new(175.0, 0.0),
            LonLat::new(-175.0, 0.0),
            LonLat::new(179.0, -5.0),
            LonLat::new(179.0, 5.0),
        );
        assert!(hit);
        let miss = geo_segments_intersect(
            LonLat::new(175.0, 0.0),
            LonLat::new(-175.0, 0.0),
            LonLat::new(0.0, -5.0),
            LonLat::new(0.0, 5.0),
        );
        assert!(!miss);
    }
}
