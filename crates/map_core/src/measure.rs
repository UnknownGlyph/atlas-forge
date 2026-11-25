//! Measuring on the map: lengths along great circles and areas of rings
//! whose sides are great circles, on the document's own sphere (its
//! radius), so a fantasy world measures in its own size.

use crate::doc::Document;
use crate::geom::{self, LonLat};

/// Great-circle distance between two points on a sphere of radius `r`.
pub fn great_circle(a: LonLat, b: LonLat, r: f64) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    let h = ((p2 - p1) * 0.5).sin().powi(2) + p1.cos() * p2.cos() * (dl * 0.5).sin().powi(2);
    2.0 * r * h.sqrt().min(1.0).asin()
}

/// Points along the great circle from `a` to `b` (both included), at most
/// `step` degrees apart.
pub fn great_circle_path(a: LonLat, b: LonLat, step: f64) -> Vec<LonLat> {
    let v = |p: LonLat| {
        let (lo, la) = (p.lon.to_radians(), p.lat.to_radians());
        [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
    };
    let (u, w) = (v(a), v(b));
    let angle = (u[0] * w[0] + u[1] * w[1] + u[2] * w[2]).clamp(-1.0, 1.0).acos();
    let n = ((angle.to_degrees() / step.max(1e-3)).ceil() as usize).max(1);
    let sin = angle.sin();
    (0..=n)
        .map(|i| {
            let t = i as f64 / n as f64;
            // the ends exactly, and no direction to follow between equal points
            if i == 0 || (sin < 1e-12 && i < n) {
                return a;
            }
            if i == n {
                return b;
            }
            let (ka, kb) = (((1.0 - t) * angle).sin() / sin, (t * angle).sin() / sin);
            let p = [ka * u[0] + kb * w[0], ka * u[1] + kb * w[1], ka * u[2] + kb * w[2]];
            LonLat::new(p[1].atan2(p[0]).to_degrees(), p[2].clamp(-1.0, 1.0).asin().to_degrees())
        })
        .collect()
}

impl Document {
    /// Length of a path along great circles, in the document's units (km).
    pub fn path_length_km(&self, pts: &[LonLat]) -> f64 {
        let r = self.radius();
        pts.windows(2).map(|w| great_circle(w[0], w[1], r)).sum()
    }

    /// Area of a ring whose sides are great circles (closed back to its
    /// first point), in the document's units squared (km²). Either winding.
    pub fn ring_area_km2(&self, ring: &[LonLat]) -> f64 {
        if ring.len() < 3 {
            return 0.0;
        }
        let mut dense: Vec<LonLat> = Vec::new();
        for i in 0..ring.len() {
            let side = great_circle_path(ring[i], ring[(i + 1) % ring.len()], 0.25);
            dense.extend_from_slice(&side[..side.len() - 1]);
        }
        let un = geom::unwrap_path(&dense, None);
        geom::spherical_area(&un, self.radius()).abs()
    }
}

/// Largest standard scale-bar distance (1, 2 or 5 times a power of ten)
/// within the given positive distance.
pub fn scale_bar_length(distance: f64) -> f64 {
    let power = 10f64.powf(distance.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|unit| unit * power)
        .find(|value| *value <= distance).unwrap_or(power)
}
