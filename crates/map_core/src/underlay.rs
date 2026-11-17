//! Reference images under the map (scanned historical maps, world templates)
//! and their georeferencing: image pixel ⇄ lon/lat.
//!
//! An image is taken to be drawn in one of the supported projections with a
//! known central meridian; pixel = affine(projected x, y). The affine part
//! comes from control points (pixel ↔ lon/lat pairs):
//! 0 points → the initial placement (a lon/lat box), 1 → that placement
//! shifted, 2 → similarity (shift, uniform scale, rotation), ≥ 3 → full
//! affine (similarity again when the points are nearly collinear).

use crate::geom::{self, LonLat};
use crate::projection::Projection;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Encoded image file bytes (PNG, JPEG, …), shared between undo snapshots
/// and stored as a binary blob in the project file.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Blob(pub Arc<Vec<u8>>);

impl Serialize for Blob {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Blob;
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }
            fn visit_bytes<E>(self, v: &[u8]) -> Result<Blob, E> {
                Ok(Blob(Arc::new(v.to_vec())))
            }
            fn visit_byte_buf<E>(self, v: Vec<u8>) -> Result<Blob, E> {
                Ok(Blob(Arc::new(v)))
            }
            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut a: A) -> Result<Blob, A::Error> {
                let mut v = Vec::with_capacity(a.size_hint().unwrap_or(0));
                while let Some(b) = a.next_element()? {
                    v.push(b);
                }
                Ok(Blob(Arc::new(v)))
            }
        }
        d.deserialize_bytes(V)
    }
}

/// A pixel of the image and where it really is.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ControlPoint {
    /// Image pixel (x right, y down; pixel centres at .5).
    pub px: [f64; 2],
    pub ll: LonLat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Underlay {
    pub name: String,
    pub image: Blob,
    /// Pixel size of the decoded image.
    pub size: [u32; 2],
    /// Projection the image is drawn in, and its central meridian.
    pub proj: Projection,
    pub lon0: f64,
    /// Initial placement: west, south, east, north (degrees). East/west are
    /// taken relative to `lon0` without wrapping, so -180..180 is the world.
    pub bounds: [f64; 4],
    pub points: Vec<ControlPoint>,
    pub opacity: f32,
    pub visible: bool,
    /// Draw over land fills (under borders) instead of only over the sea.
    pub above_land: bool,
}

/// Image pixel = `m` · [x, y, 1], with (x, y) the projected position of
/// (lon - lon0, lat) in sphere units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Georef {
    pub proj: Projection,
    pub lon0: f64,
    pub m: [[f64; 3]; 2],
}

impl Georef {
    pub fn xy(&self, ll: LonLat) -> [f64; 2] {
        self.proj.forward_deg(geom::wrap_lon(ll.lon - self.lon0), ll.lat)
    }

    pub fn pixel_of(&self, ll: LonLat) -> [f64; 2] {
        let [x, y] = self.xy(ll);
        let m = &self.m;
        [m[0][0] * x + m[0][1] * y + m[0][2], m[1][0] * x + m[1][1] * y + m[1][2]]
    }

    /// Projected (x, y) = inverse · [px, py, 1]; None if degenerate.
    pub fn inverse(&self) -> Option<[[f64; 3]; 2]> {
        let m = &self.m;
        let det = m[0][0] * m[1][1] - m[0][1] * m[1][0];
        if det.abs() < 1e-300 || !det.is_finite() {
            return None;
        }
        let (a, b, c, d) = (m[1][1] / det, -m[0][1] / det, -m[1][0] / det, m[0][0] / det);
        Some([[a, b, -(a * m[0][2] + b * m[1][2])], [c, d, -(c * m[0][2] + d * m[1][2])]])
    }

    pub fn lonlat_of(&self, px: [f64; 2]) -> Option<LonLat> {
        let i = self.inverse()?;
        let x = i[0][0] * px[0] + i[0][1] * px[1] + i[0][2];
        let y = i[1][0] * px[0] + i[1][1] * px[1] + i[1][2];
        let ll = self.proj.inverse_deg(x, y)?;
        Some(LonLat::new(geom::wrap_lon(ll[0] + self.lon0), ll[1]))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitKind {
    Placement,
    Shift,
    Similarity,
    Affine,
}

impl FitKind {
    pub fn name(self) -> &'static str {
        match self {
            FitKind::Placement => "初始放置",
            FitKind::Shift => "平移",
            FitKind::Similarity => "相似变换（平移+缩放+旋转）",
            FitKind::Affine => "仿射变换",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Fit {
    pub georef: Georef,
    pub kind: FitKind,
    /// Per control point: distance in image pixels between where the point
    /// was placed and where the fitted transform puts it.
    pub residuals_px: Vec<f64>,
    pub rms_px: f64,
}

impl Underlay {
    /// A whole-world equirectangular image (the most common template).
    pub fn new(name: &str, image: Vec<u8>, size: [u32; 2]) -> Self {
        Self {
            name: name.into(),
            image: Blob(Arc::new(image)),
            size,
            proj: Projection::Equirectangular,
            lon0: 0.0,
            bounds: [-180.0, -90.0, 180.0, 90.0],
            points: vec![],
            opacity: 0.6,
            visible: true,
            above_land: true,
        }
    }

    /// The initial placement in projection `proj` centred on `lon0`.
    pub fn placement(&self, proj: Projection, lon0: f64) -> Georef {
        let [w, s, e, n] = self.bounds;
        // for non-cylindrical projections the box is taken along the equator
        // and the central meridian: only a starting point for control points
        let lat_cap = |l: f64| if proj == Projection::Mercator { l.clamp(-Projection::MERC_MAX_LAT, Projection::MERC_MAX_LAT) } else { l };
        let xw = proj.forward_deg(w - lon0, 0.0)[0];
        let xe = proj.forward_deg(e - lon0, 0.0)[0];
        let yn = proj.forward_deg(0.0, lat_cap(n))[1];
        let ys = proj.forward_deg(0.0, lat_cap(s))[1];
        let (sx, sy) = (self.size[0] as f64 / (xe - xw), self.size[1] as f64 / (yn - ys));
        Georef { proj, lon0, m: [[sx, 0.0, -xw * sx], [0.0, -sy, yn * sy]] }
    }

    pub fn fit(&self) -> Fit {
        self.fit_with(self.proj, self.lon0)
    }

    /// Fit the control points assuming the image is in `proj` / `lon0`.
    pub fn fit_with(&self, proj: Projection, lon0: f64) -> Fit {
        let base = self.placement(proj, lon0);
        let pts: Vec<([f64; 2], [f64; 2])> = self.points.iter().map(|c| (base.xy(c.ll), c.px)).collect();
        let (m, kind) = match pts.len() {
            0 => (base.m, FitKind::Placement),
            1 => shift(&base, &pts[0]),
            2 => similarity(&pts).map(|m| (m, FitKind::Similarity)).unwrap_or_else(|| shift(&base, &pts[0])),
            _ => affine(&pts)
                .map(|m| (m, FitKind::Affine))
                .or_else(|| similarity(&pts).map(|m| (m, FitKind::Similarity)))
                .unwrap_or_else(|| shift(&base, &pts[0])),
        };
        let georef = Georef { proj, lon0, m };
        let residuals_px: Vec<f64> = self
            .points
            .iter()
            .map(|c| {
                let p = georef.pixel_of(c.ll);
                ((p[0] - c.px[0]).powi(2) + (p[1] - c.px[1]).powi(2)).sqrt()
            })
            .collect();
        let rms_px = if residuals_px.is_empty() { 0.0 } else { (residuals_px.iter().map(|r| r * r).sum::<f64>() / residuals_px.len() as f64).sqrt() };
        Fit { georef, kind, residuals_px, rms_px }
    }

    /// RMS residual for every supported projection (same central meridian),
    /// best first. Only informative with more points than the fit needs
    /// (≥ 4 for affine).
    pub fn rank_projections(&self) -> Vec<(Projection, f64)> {
        let mut r: Vec<(Projection, f64)> = Projection::ALL.iter().map(|p| (*p, self.fit_with(*p, self.lon0).rms_px)).collect();
        r.sort_by(|a, b| a.1.total_cmp(&b.1));
        r
    }
}

/// Base placement moved so that the one point lands exactly.
fn shift(base: &Georef, (xy, px): &([f64; 2], [f64; 2])) -> ([[f64; 3]; 2], FitKind) {
    let mut m = base.m;
    for r in 0..2 {
        m[r][2] = px[r] - m[r][0] * xy[0] - m[r][1] * xy[1];
    }
    (m, FitKind::Shift)
}

/// Least-squares similarity (conformal, with the pixel y axis flipped):
/// (u, -v) = α·(x + iy) + β over complex numbers.
fn similarity(pts: &[([f64; 2], [f64; 2])]) -> Option<[[f64; 3]; 2]> {
    let n = pts.len() as f64;
    let (mut px, mut py, mut wu, mut wv) = (0.0, 0.0, 0.0, 0.0);
    for (p, w) in pts {
        px += p[0] / n;
        py += p[1] / n;
        wu += w[0] / n;
        wv += -w[1] / n;
    }
    let (mut num_r, mut num_i, mut den) = (0.0, 0.0, 0.0);
    for (p, w) in pts {
        let (x, y) = (p[0] - px, p[1] - py);
        let (u, v) = (w[0] - wu, -w[1] - wv);
        // (u + iv) · conj(x + iy)
        num_r += u * x + v * y;
        num_i += v * x - u * y;
        den += x * x + y * y;
    }
    if den < 1e-18 {
        return None;
    }
    let (a, b) = (num_r / den, num_i / den);
    // β = w̄ - α p̄
    let br = wu - (a * px - b * py);
    let bi = wv - (b * px + a * py);
    Some([[a, -b, br], [-b, -a, -bi]])
}

/// Least-squares affine, solved on centred coordinates.
fn affine(pts: &[([f64; 2], [f64; 2])]) -> Option<[[f64; 3]; 2]> {
    let n = pts.len() as f64;
    let mean = |f: &dyn Fn(&([f64; 2], [f64; 2])) -> f64| pts.iter().map(f).sum::<f64>() / n;
    let (mx, my) = (mean(&|p| p.0[0]), mean(&|p| p.0[1]));
    let (mut sxx, mut sxy, mut syy) = (0.0, 0.0, 0.0);
    for (p, _) in pts {
        let (x, y) = (p[0] - mx, p[1] - my);
        sxx += x * x;
        sxy += x * y;
        syy += y * y;
    }
    let det = sxx * syy - sxy * sxy;
    // nearly collinear points determine only a similarity
    if det <= 1e-6 * sxx * syy || det < 1e-24 {
        return None;
    }
    let mut m = [[0.0; 3]; 2];
    for (r, row) in m.iter_mut().enumerate() {
        let mw = mean(&|p| p.1[r]);
        let (mut sxw, mut syw) = (0.0, 0.0);
        for (p, w) in pts {
            sxw += (p[0] - mx) * (w[r] - mw);
            syw += (p[1] - my) * (w[r] - mw);
        }
        let a = (syy * sxw - sxy * syw) / det;
        let b = (sxx * syw - sxy * sxw) / det;
        *row = [a, b, mw - a * mx - b * my];
    }
    Some(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2], tol: f64) -> bool {
        (a[0] - b[0]).abs() < tol && (a[1] - b[1]).abs() < tol
    }

    #[test]
    fn world_placement_maps_corners() {
        let u = Underlay::new("w", vec![], [3600, 1800]);
        let g = u.fit().georef;
        assert!(close(g.pixel_of(LonLat::new(-180.0, 90.0)), [0.0, 0.0], 1e-9) || close(g.pixel_of(LonLat::new(-180.0, 90.0)), [3600.0, 0.0], 1e-9));
        assert!(close(g.pixel_of(LonLat::new(0.0, 0.0)), [1800.0, 900.0], 1e-9));
        assert!(close(g.pixel_of(LonLat::new(90.0, -45.0)), [2700.0, 1350.0], 1e-9));
        let back = g.lonlat_of([2700.0, 1350.0]).unwrap();
        assert!((back.lon - 90.0).abs() < 1e-9 && (back.lat + 45.0).abs() < 1e-9);
    }

    /// Synthesize control points from a known georeference, perturb nothing,
    /// and check each fit level recovers it.
    #[test]
    fn fits_recover_known_transforms() {
        for proj in Projection::ALL {
            let mut u = Underlay::new("t", vec![], [4000, 2500]);
            u.proj = proj;
            u.lon0 = 150.0;
            // truth: rotated by 3°, scaled 900 px/unit (y 870 → not a similarity), shifted
            let (c, s) = (3f64.to_radians().cos(), 3f64.to_radians().sin());
            let truth = Georef { proj, lon0: 150.0, m: [[900.0 * c, -870.0 * s, 2100.0], [-900.0 * s, -870.0 * c, 1300.0]] };
            let lls = [LonLat::new(120.0, 30.0), LonLat::new(-170.0, -20.0), LonLat::new(160.0, 55.0), LonLat::new(100.0, -40.0), LonLat::new(178.0, 5.0)];
            u.points = lls.iter().map(|ll| ControlPoint { px: truth.pixel_of(*ll), ll: *ll }).collect();
            let f = u.fit();
            assert_eq!(f.kind, FitKind::Affine);
            assert!(f.rms_px < 1e-6, "{proj:?}: {}", f.rms_px);
            for ll in lls {
                let back = f.georef.lonlat_of(truth.pixel_of(ll)).unwrap();
                assert!(geom::geo_dist_deg(back, ll) < 1e-8, "{proj:?}");
            }
            // the right projection is ranked first
            assert_eq!(u.rank_projections()[0].0, proj);
            // a pure similarity is recovered from 2 points exactly
            let sim = Georef { proj, lon0: 150.0, m: [[800.0 * c, -800.0 * s, 2000.0], [-800.0 * s, -800.0 * c, 1200.0]] };
            u.points = lls[..2].iter().map(|ll| ControlPoint { px: sim.pixel_of(*ll), ll: *ll }).collect();
            let f = u.fit();
            assert_eq!(f.kind, FitKind::Similarity);
            for (a, b) in f.georef.m.iter().flatten().zip(sim.m.iter().flatten()) {
                assert!((a - b).abs() < 1e-6 * b.abs().max(1.0), "{proj:?}: {:?} vs {:?}", f.georef.m, sim.m);
            }
        }
    }

    #[test]
    fn degenerate_points_fall_back() {
        let mut u = Underlay::new("t", vec![], [1000, 500]);
        // one point: placement shifted so it lands exactly
        u.points = vec![ControlPoint { px: [10.0, 20.0], ll: LonLat::new(5.0, 5.0) }];
        let f = u.fit();
        assert_eq!(f.kind, FitKind::Shift);
        assert!(f.rms_px < 1e-9);
        // three collinear points: similarity, not a singular affine
        u.points = (0..3).map(|i| ControlPoint { px: [100.0 + 50.0 * i as f64, 200.0], ll: LonLat::new(10.0 * i as f64, 0.0) }).collect();
        let f = u.fit();
        assert_eq!(f.kind, FitKind::Similarity);
        assert!(f.georef.inverse().is_some());
        // two identical points: shift
        u.points = vec![u.points[0], u.points[0]];
        assert_eq!(u.fit().kind, FitKind::Shift);
    }

    #[test]
    fn blob_serializes_as_bytes() {
        let mut u = Underlay::new("t", (0..=255u8).collect(), [16, 16]);
        u.points.push(ControlPoint { px: [1.0, 2.0], ll: LonLat::new(3.0, 4.0) });
        let b = rmp_serde_roundtrip(&u);
        assert_eq!(b, u);
    }

    fn rmp_serde_roundtrip(u: &Underlay) -> Underlay {
        // serde_json has no bytes type; a sequence must also deserialize
        let s = serde_json::to_string(u).unwrap();
        serde_json::from_str(&s).unwrap()
    }
}
