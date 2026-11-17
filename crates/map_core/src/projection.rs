//! Map projections (CPU reference implementation).
//!
//! Units: input radians, output in "sphere units" (R = 1). The GPU shaders in
//! `map_render` implement the same formulas; `tests::shader_constants` keeps
//! the shared constants honest.

use serde::{Deserialize, Serialize};
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Projection {
    Equirectangular,
    Mercator,
    Miller,
    EqualEarth,
}

impl Projection {
    pub const ALL: [Projection; 4] = [Self::Equirectangular, Self::Mercator, Self::Miller, Self::EqualEarth];

    pub fn name(self) -> &'static str {
        match self {
            Self::Equirectangular => "等距圆柱",
            Self::Mercator => "墨卡托",
            Self::Miller => "米勒",
            Self::EqualEarth => "Equal Earth (等积)",
        }
    }

    /// Index used by the shader.
    pub fn gpu_id(self) -> u32 {
        self as u32
    }

    pub const MERC_MAX_LAT: f64 = 85.05112878;

    pub fn forward(self, lon: f64, lat: f64) -> [f64; 2] {
        match self {
            Self::Equirectangular => [lon, lat],
            Self::Mercator => {
                let l = lat.clamp(-Self::MERC_MAX_LAT.to_radians(), Self::MERC_MAX_LAT.to_radians());
                [lon, (FRAC_PI_4 + l * 0.5).tan().ln()]
            }
            Self::Miller => [lon, 1.25 * (FRAC_PI_4 + 0.4 * lat).tan().ln()],
            Self::EqualEarth => {
                let (a1, a2, a3, a4) = EE;
                let m = 3f64.sqrt() / 2.0;
                let t = (m * lat.sin()).asin();
                let t2 = t * t;
                let t6 = t2 * t2 * t2;
                let x = 2.0 * 3f64.sqrt() * lon * t.cos() / (3.0 * (9.0 * a4 * t6 * t2 + 7.0 * a3 * t6 + 3.0 * a2 * t2 + a1));
                let y = t * (a1 + a2 * t2 + t6 * (a3 + a4 * t2));
                [x, y]
            }
        }
    }

    pub fn inverse(self, x: f64, y: f64) -> Option<[f64; 2]> {
        let r = match self {
            Self::Equirectangular => [x, y],
            Self::Mercator => [x, 2.0 * y.exp().atan() - FRAC_PI_2],
            Self::Miller => [x, ((y / 1.25).exp().atan() - FRAC_PI_4) / 0.4],
            Self::EqualEarth => {
                let (a1, a2, a3, a4) = EE;
                let m = 3f64.sqrt() / 2.0;
                let mut t = y;
                for _ in 0..12 {
                    let t2 = t * t;
                    let t6 = t2 * t2 * t2;
                    let fy = t * (a1 + a2 * t2 + t6 * (a3 + a4 * t2)) - y;
                    let fpy = a1 + 3.0 * a2 * t2 + t6 * (7.0 * a3 + 9.0 * a4 * t2);
                    let dt = fy / fpy;
                    t -= dt;
                    if dt.abs() < 1e-12 {
                        break;
                    }
                }
                let t2 = t * t;
                let t6 = t2 * t2 * t2;
                let lon = 3.0 * x * (a1 + 3.0 * a2 * t2 + t6 * (7.0 * a3 + 9.0 * a4 * t2)) / (2.0 * 3f64.sqrt() * t.cos());
                let s = t.sin() / m;
                if s.abs() > 1.0 {
                    return None;
                }
                [lon, s.asin()]
            }
        };
        (r[0].abs() <= PI + 1e-9 && r[1].abs() <= FRAC_PI_2 + 1e-9 && r[0].is_finite() && r[1].is_finite()).then_some(r)
    }

    /// Projected extent of the whole world: [xmin, ymin, xmax, ymax].
    pub fn bounds(self) -> [f64; 4] {
        let top = match self {
            Self::Mercator => self.forward(0.0, Self::MERC_MAX_LAT.to_radians())[1],
            _ => self.forward(0.0, FRAC_PI_2)[1],
        };
        let xmax = self.forward(PI, 0.0)[0];
        [-xmax, -top, xmax, top]
    }

    pub fn forward_deg(self, lon: f64, lat: f64) -> [f64; 2] {
        self.forward(lon.to_radians(), lat.to_radians())
    }
    pub fn inverse_deg(self, x: f64, y: f64) -> Option<[f64; 2]> {
        self.inverse(x, y).map(|r| [r[0].to_degrees(), r[1].to_degrees()])
    }
}

const EE: (f64, f64, f64, f64) = (1.340264, -0.081106, 0.000893, 0.003796);

/// Unit-sphere position for the globe (y up, lon 0 facing +z, east = +x).
pub fn sphere_pos(lon_deg: f64, lat_deg: f64) -> [f64; 3] {
    let (l, p) = (lon_deg.to_radians(), lat_deg.to_radians());
    [p.cos() * l.sin(), p.sin(), p.cos() * l.cos()]
}

pub fn sphere_to_lonlat(v: [f64; 3]) -> [f64; 2] {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0].atan2(v[2]).to_degrees(), (v[1] / n).clamp(-1.0, 1.0).asin().to_degrees()]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn roundtrip_all() {
        for p in Projection::ALL {
            for lon in [-179.0, -90.0, 0.0, 45.0, 179.0] {
                for lat in [-80.0, -30.0, 0.0, 10.0, 60.0, 80.0] {
                    let xy = p.forward_deg(lon, lat);
                    let back = p.inverse_deg(xy[0], xy[1]).unwrap();
                    assert!((back[0] - lon).abs() < 1e-6 && (back[1] - lat).abs() < 1e-6, "{p:?} {lon} {lat} -> {back:?}");
                }
            }
        }
    }
    #[test]
    fn sphere_roundtrip() {
        let v = sphere_pos(123.0, -45.0);
        let ll = sphere_to_lonlat(v);
        assert!((ll[0] - 123.0).abs() < 1e-9 && (ll[1] + 45.0).abs() < 1e-9);
    }
}
