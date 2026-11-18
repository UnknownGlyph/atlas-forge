use super::*;
use std::fmt::Write;
impl Scenario {
    /// Georeferenced equirectangular raster shared by map views and PNG.
    pub fn raster(&self, field: Field, period: Period) -> (usize, usize, Vec<u8>) {
        const SCALE: usize = 8;
        let (w, h) = (W * SCALE, H * SCALE);
        let base = self.image(field, period);
        let mut out = vec![0; w * h * 4];
        let categorical = matches!(field, Field::Zone | Field::Vegetation);
        for y in 0..h {
            for x in 0..w {
                let cell = y / SCALE * W + x / SCALE;
                let offset = (y * w + x) * 4;
                if base[cell * 4 + 3] == 0 {
                    continue;
                }
                if categorical {
                    out[offset..offset + 4].copy_from_slice(&base[cell * 4..cell * 4 + 4]);
                    continue;
                }
                let fx = (x as f32 + 0.5) / SCALE as f32 - 0.5;
                let fy = (y as f32 + 0.5) / SCALE as f32 - 0.5;
                let mut rgb = [0.; 3];
                let mut total = 0.;
                for (dy, wy) in [(0, 1. - fy.rem_euclid(1.)), (1, fy.rem_euclid(1.))] {
                    let yy = (fy.floor() as i32 + dy).clamp(0, H as i32 - 1) as usize;
                    for (dx, wx) in [(0, 1. - fx.rem_euclid(1.)), (1, fx.rem_euclid(1.))] {
                        let xx = (fx.floor() as i32 + dx).rem_euclid(W as i32) as usize;
                        let k = 4 * (yy * W + xx);
                        if base[k + 3] == 0 {
                            continue;
                        }
                        let weight = wx * wy;
                        total += weight;
                        for n in 0..3 {
                            rgb[n] += weight * f32::from(base[k + n]);
                        }
                    }
                }
                for n in 0..3 {
                    out[offset + n] = (rgb[n] / total.max(0.0001)).round() as u8;
                }
                out[offset + 3] = 255;
            }
        }
        if field == Field::Wind {
            for i in region_cells(self.region).filter(|&i| i % W % 3 == 1 && i / W % 3 == 1) {
                let a = self.value(i, field, period).to_radians();
                let (dx, dy) = (a.sin(), -a.cos());
                let (cx, cy) = (
                    (i % W * SCALE + SCALE / 2) as f32,
                    (i / W * SCALE + SCALE / 2) as f32,
                );
                for (x0, y0, x1, y1) in [
                    (-6. * dx, -6. * dy, 6. * dx, 6. * dy),
                    (6. * dx, 6. * dy, dx - 3. * dy, dy + 3. * dx),
                    (6. * dx, 6. * dy, dx + 3. * dy, dy - 3. * dx),
                ] {
                    for t in 0..=18 {
                        let f = t as f32 / 18.;
                        let x = (cx + x0 + (x1 - x0) * f).round() as usize;
                        let y = (cy + y0 + (y1 - y0) * f).round() as usize;
                        if x < w && y < h {
                            let o = 4 * (y * w + x);
                            if out[o + 3] != 0 {
                                out[o..o + 3].copy_from_slice(&[31, 41, 52]);
                            }
                        }
                    }
                }
            }
        }
        (w, h, out)
    }
    pub fn csv(&self) -> String {
        let mut out=String::from("\u{feff}lon,lat,month,surface,height_m,temp_C,precip_mm,relative_humidity_pct,wind_to_deg,ice_pct,annual_range_C,climate,vegetation,moisture_index,authored_fields,locked_fields\r\n");
        for i in region_cells(self.region) {
            let p = center(i);
            let g = self.cells[i].ground;
            let flags = |lock: bool| {
                Field::ALL
                    .into_iter()
                    .filter(|f| {
                        self.edits.get(&(i, *f)).is_some_and(|e| {
                            if lock {
                                e.locked
                            } else {
                                e.authored
                            }
                        })
                    })
                    .map(|f| f.name())
                    .collect::<Vec<_>>()
                    .join("|")
            };
            let authored = flags(false);
            let locked = flags(true);
            for m in 0..12 {
                write!(
                    &mut out,
                    "{:.4},{:.4},{},{},{:.0}",
                    p.lon,
                    p.lat,
                    m + 1,
                    g.surface.name(),
                    g.height
                )
                .unwrap();
                for f in Field::ALL {
                    let v = self.value(i, f, Period::Month(m));
                    write!(&mut out, ",{}", f.format(v)).unwrap();
                }
                write!(&mut out, ",{authored},{locked}\r\n").unwrap();
            }
        }
        out
    }
}
