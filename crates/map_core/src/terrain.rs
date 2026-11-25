//! Terrain: an elevation grid over the whole globe (metres; below zero is
//! sea floor), equirectangular, the first row at the north. It is made by
//! the new-world wizard (`crate::worldgen`) and shown as hill shading: a
//! picture laid over the map, darker on slopes turned from the light and
//! lighter on slopes turned to it. Kept behind an `Arc`, so the document's
//! undo copies share it.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Terrain {
    pub w: u32,
    pub h: u32,
    /// Row by row from the north, west to east.
    pub heights: Vec<i16>,
}

impl Terrain {
    pub fn new(w: u32, h: u32, heights: Vec<i16>) -> Self {
        assert_eq!(heights.len(), (w * h) as usize);
        Terrain { w, h, heights }
    }

    fn get(&self, i: i64, j: i64) -> f64 {
        let (w, h) = (self.w as i64, self.h as i64);
        let i = i.rem_euclid(w);
        let j = j.clamp(0, h - 1);
        self.heights[(j * w + i) as usize] as f64
    }

    /// Elevation at a place (metres), between the grid's cells.
    pub fn at(&self, lon: f64, lat: f64) -> f64 {
        let x = (lon + 180.0) / 360.0 * self.w as f64 - 0.5;
        let y = (90.0 - lat) / 180.0 * self.h as f64 - 0.5;
        let (i, j) = (x.floor() as i64, y.floor() as i64);
        let (fx, fy) = (x - x.floor(), y - y.floor());
        let a = self.get(i, j) * (1.0 - fx) + self.get(i + 1, j) * fx;
        let b = self.get(i, j + 1) * (1.0 - fx) + self.get(i + 1, j + 1) * fx;
        a * (1.0 - fy) + b * fy
    }

    /// The hill shading as straight-alpha RGBA, `w` × `h`, the first row
    /// at the north: light from the north-west, `exaggerate` times the
    /// real relief (on a planet of `radius_km`); the sea left clear.
    pub fn shade(&self, radius_km: f64, strength: f64) -> Vec<u8> {
        let (w, h) = (self.w as usize, self.h as usize);
        let mut out = vec![0u8; w * h * 4];
        if w == 0 || h == 0 { return out; }
        // metres per cell north-south
        let dy = std::f64::consts::PI * radius_km * 1000.0 / h as f64;
        let light = [-0.5f64, 0.5, 0.7];
        let ln = (light[0] * light[0] + light[1] * light[1] + light[2] * light[2]).sqrt();
        let dx: Vec<_> = (0..h).map(|j| {
            let lat = 90.0 - (j as f64 + 0.5) / h as f64 * 180.0;
            dy * lat.to_radians().cos().max(0.05) * h as f64 * 2.0 / w as f64
        }).collect();
        let slope = |i: usize, j: usize| -> (f64, f64) {
            let west = if i == 0 { w - 1 } else { i - 1 };
            let east = if i + 1 == w { 0 } else { i + 1 };
            let height = |x: usize, y: usize| f64::from(self.heights[y * w + x]);
            let gx = (height(east, j) - height(west, j)) / (2.0 * dx[j]);
            let gy = (height(i, j.saturating_sub(1)) - height(i, (j + 1).min(h - 1))) / (2.0 * dy);
            (gx, gy)
        };
        // on a world map real slopes are slight: the land's steep slopes
        // (the 95th in a hundred) are drawn as about 50°, the rest in proportion
        let mut steep = Vec::new();
        for (j, row) in self.heights.chunks_exact(w).take(h).enumerate() {
            for (i, &height) in row.iter().enumerate() {
                if height > 0 {
                    let (gx, gy) = slope(i, j);
                    steep.push(gx.hypot(gy));
                }
            }
        }
        if steep.is_empty() { return out; }
        let rank = steep.len() * 95 / 100;
        let p95 = steep.select_nth_unstable_by(rank, f64::total_cmp).1.max(1e-6);
        let exaggerate = 1.2 / p95 * strength;
        for j in 0..h {
            for i in 0..w {
                if self.heights[j * w + i] <= 0 {
                    continue;
                }
                let (gx, gy) = slope(i, j);
                let (gx, gy) = (gx * exaggerate, gy * exaggerate);
                // the surface's normal (x east, y north, z up), against the light
                let n = [-gx, -gy, 1.0];
                let nn = (n[0] * n[0] + n[1] * n[1] + 1.0).sqrt();
                let lit = (n[0] * light[0] + n[1] * light[1] + n[2] * light[2]) / (nn * ln);
                let flat = light[2] / ln;
                let d = lit - flat;
                let k = (j * w + i) * 4;
                if d < 0.0 {
                    // in shadow: dark, as see-through as the slope is gentle
                    let a = (-d * 2.2).min(0.75);
                    out[k..k + 4].copy_from_slice(&[40, 34, 30, (a * 255.0) as u8]);
                } else {
                    let a = (d * 1.6).min(0.45);
                    out[k..k + 4].copy_from_slice(&[255, 252, 240, (a * 255.0) as u8]);
                }
            }
        }
        out
    }
}

impl Terrain {
    /// Where the grid's pictures lie: the whole world, equirectangular.
    pub fn georef(&self) -> crate::underlay::Georef {
        global_georef(self.w, self.h)
    }
}

/// Georeference of a global equirectangular raster with north at the top.
pub fn global_georef(w: u32, h: u32) -> crate::underlay::Georef {
    use crate::projection::Projection;
    let p = Projection::Equirectangular;
    let (xw, xe) = (p.forward_deg(-180.0, 0.0)[0], p.forward_deg(180.0, 0.0)[0]);
    let (yn, ys) = (p.forward_deg(0.0, 90.0)[1], p.forward_deg(0.0, -90.0)[1]);
    let (sx, sy) = (w as f64 / (xe - xw), h as f64 / (yn - ys));
    crate::underlay::Georef { proj: p, lon0: 0.0, m: [[sx, 0.0, -xw * sx], [0.0, -sy, yn * sy]] }
}

/// `Option<Arc<Terrain>>` saved as the terrain itself.
pub mod opt_arc {
    use super::Terrain;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::sync::Arc;

    pub fn serialize<S: Serializer>(v: &Option<Arc<Terrain>>, s: S) -> Result<S::Ok, S::Error> {
        v.as_deref().serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Arc<Terrain>>, D::Error> {
        Ok(Option::<Terrain>::deserialize(d)?.map(Arc::new))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heights_between_cells_and_shading() {
        // a cone on a 36 × 18 grid (10° cells), sea all round
        let (w, h) = (36u32, 18u32);
        let heights: Vec<i16> = (0..w * h)
            .map(|k| {
                let (i, j) = ((k % w) as f64, (k / w) as f64);
                let d = ((i - 18.0).powi(2) + (j - 9.0).powi(2)).sqrt();
                (3000.0 - d * 600.0).max(-500.0) as i16
            })
            .collect();
        let t = Terrain::new(w, h, heights);
        // the cell centre of (18, 9) is at lon 5, lat -5
        assert!((t.at(5.0, -5.0) - 3000.0).abs() < 1e-6);
        assert!(t.at(5.0, 0.0) < 3000.0 && t.at(5.0, 0.0) > t.at(5.0, 20.0));
        let img = t.shade(6371.0, 1.0);
        assert_eq!(img.len(), (w * h * 4) as usize);
        let px = |i: u32, j: u32| &img[((j * w + i) * 4) as usize..((j * w + i) * 4 + 4) as usize];
        // the slope facing the north-west light is light, the far side dark, the sea clear
        assert!(px(16, 7)[0] > 200 && px(16, 7)[3] > 0, "{:?}", px(16, 7));
        assert!(px(20, 11)[0] < 100 && px(20, 11)[3] > 0, "{:?}", px(20, 11));
        assert_eq!(px(0, 0)[3], 0);
    }
}
