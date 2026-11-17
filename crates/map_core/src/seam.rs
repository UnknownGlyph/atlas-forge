//! Exact splitting of unwrapped lon/lat polygons at the antimeridian.
//!
//! A boolean intersection with the strip [-180, 180] cuts the polygon, but
//! its output coordinates are rounded to the kernel's internal grid. That
//! breaks shared borders (an unclipped neighbour keeps the exact
//! coordinates) and makes the two halves' seam points disagree. So after
//! the boolean, every vertex is restored exactly: original vertices to
//! their input value, seam crossings to a value computed from the
//! *unshifted* segment, identical for both halves.

use crate::geom::{self, P2};
use geo::BooleanOps;

const SNAP: f64 = 1e-6;

/// Split rings (outer first; unwrapped lon, possibly outside ±180) into
/// parts lying inside lon ∈ [-180, 180]. Parts are lists of rings, outer first.
pub fn split(rings: &[Vec<P2>]) -> Vec<Vec<Vec<P2>>> {
    let Some(outer) = rings.first() else { return vec![] };
    if outer.len() < 3 {
        return vec![];
    }
    let b = geom::bbox(outer);
    let mut parts = Vec::new();
    for k in [-720.0, -360.0, 0.0, 360.0, 720.0] {
        if b[2] + k <= -180.0 || b[0] + k >= 180.0 {
            continue;
        }
        let shifted: Vec<Vec<P2>> = rings.iter().map(|r| r.iter().map(|p| [p[0] + k, p[1]]).collect()).collect();
        if b[0] + k >= -180.0 && b[2] + k <= 180.0 {
            // entirely inside: no boolean, coordinates untouched
            parts.push(shifted);
            continue;
        }
        let ls = |r: &Vec<P2>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
        let poly = geo::Polygon::new(ls(&shifted[0]), shifted[1..].iter().map(ls).collect());
        let strip = geo::Polygon::new(geo::LineString::from(vec![(-180.0, -90.0), (180.0, -90.0), (180.0, 90.0), (-180.0, 90.0)]), vec![]);
        for piece in poly.intersection(&strip) {
            let mut part = Vec::new();
            for ls in std::iter::once(piece.exterior()).chain(piece.interiors().iter()) {
                let mut r: Vec<P2> = ls.coords().map(|c| restore([c.x, c.y], rings, k)).collect();
                if r.len() > 1 && r.first() == r.last() {
                    r.pop();
                }
                r.dedup();
                if r.len() >= 3 {
                    part.push(r);
                }
            }
            if !part.is_empty() {
                parts.push(part);
            }
        }
    }
    parts
}

/// Cut rings lying inside the strip in two along the meridian `x`: the
/// parts west and east of it. Vertices keep their exact values and the two
/// sides get identical points on the cut, as in [`split`].
pub fn cut_at(rings: &[Vec<P2>], x: f64) -> Vec<Vec<Vec<P2>>> {
    let ls = |r: &Vec<P2>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    let Some(outer) = rings.first() else { return vec![] };
    let poly = geo::Polygon::new(ls(outer), rings[1..].iter().map(ls).collect());
    let mut parts = Vec::new();
    for (w, e) in [(-180.0, x), (x, 180.0)] {
        let half = geo::Polygon::new(geo::LineString::from(vec![(w, -90.0), (e, -90.0), (e, 90.0), (w, 90.0)]), vec![]);
        for piece in poly.intersection(&half) {
            let mut part = Vec::new();
            for ls in std::iter::once(piece.exterior()).chain(piece.interiors().iter()) {
                let mut r: Vec<P2> = ls.coords().map(|c| restore_on([c.x, c.y], rings, 0.0, &[x, -180.0, 180.0])).collect();
                if r.len() > 1 && r.first() == r.last() {
                    r.pop();
                }
                r.dedup();
                if r.len() >= 3 {
                    part.push(r);
                }
            }
            if !part.is_empty() {
                parts.push(part);
            }
        }
    }
    parts
}

/// Map a boolean-output point back to exact coordinates.
fn restore(q: P2, rings: &[Vec<P2>], k: f64) -> P2 {
    restore_on(q, rings, k, &[-180.0, 180.0])
}

/// [`restore`] for cuts along the given meridians.
fn restore_on(q: P2, rings: &[Vec<P2>], k: f64, cuts: &[f64]) -> P2 {
    let un = [q[0] - k, q[1]];
    // 1. an original vertex
    let mut best: Option<(f64, P2)> = None;
    for r in rings {
        for p in r {
            let d = (p[0] - un[0]).abs().max((p[1] - un[1]).abs());
            if d < SNAP && best.is_none_or(|b| d < b.0) {
                best = Some((d, *p));
            }
        }
    }
    if let Some((_, p)) = best {
        return [p[0] + k, p[1]];
    }
    // 2. a seam crossing: recompute from the unshifted segment
    for c in cuts.iter().copied() {
        if (q[0] - c).abs() < SNAP {
            let c_un = c - k;
            for r in rings {
                let n = r.len();
                for i in 0..n {
                    let (a, b) = (r[i], r[(i + 1) % n]);
                    if (a[0] - c_un) * (b[0] - c_un) <= 0.0 && a[0] != b[0] {
                        let y = a[1] + (c_un - a[0]) * (b[1] - a[1]) / (b[0] - a[0]);
                        if (y - q[1]).abs() < SNAP {
                            return [c, y];
                        }
                    }
                }
            }
            return [c, q[1]];
        }
    }
    // 3. pole line
    for p in [-90.0, 90.0] {
        if (q[1] - p).abs() < SNAP {
            return [q[0], p];
        }
    }
    q
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_share_exact_seam_points_and_keep_vertices() {
        // a wavy coast crossing the seam three times, with odd coordinates
        let ring = vec![
            [170.123456789, 0.1],
            [190.987654321, 0.3],
            [190.5, 4.0],
            [175.25, 4.7],
            [175.0, 6.2],
            [190.0, 6.9],
            [189.0, 10.0],
            [170.0, 10.0],
        ];
        let parts = split(&[ring.clone()]);
        assert!(parts.len() >= 3);
        // every output vertex is an input vertex (possibly shifted) or on the seam
        for p in &parts {
            for q in &p[0] {
                let is_input = ring.iter().any(|r| (r[0] == q[0] || r[0] - 360.0 == q[0]) && r[1] == q[1]);
                assert!(is_input || q[0].abs() == 180.0, "{q:?}");
            }
        }
        // seam points on the west side (x = -180) equal those on the east side (x = 180)
        let mut east: Vec<f64> = parts.iter().flat_map(|p| p[0].iter()).filter(|q| q[0] == 180.0).map(|q| q[1]).collect();
        let mut west: Vec<f64> = parts.iter().flat_map(|p| p[0].iter()).filter(|q| q[0] == -180.0).map(|q| q[1]).collect();
        east.sort_by(|a, b| a.partial_cmp(b).unwrap());
        west.sort_by(|a, b| a.partial_cmp(b).unwrap());
        east.dedup();
        west.dedup();
        assert_eq!(east, west);
        // inside the strip: returned untouched
        let inner = vec![vec![[10.000000001, 1.0], [20.0, 1.0], [20.0, 2.0]]];
        assert_eq!(split(&inner), vec![inner.clone()]);
    }
}
