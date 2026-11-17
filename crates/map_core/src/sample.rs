//! Deterministic sample worlds: a demo map for new users and scalable
//! synthetic worlds for benchmarks. Everything is built through the public
//! editing operations, so generation doubles as a topology stress test.

use crate::doc::{Document, EntityKind, FieldKind, LineFeature, LineId, Value};
use crate::geom::LonLat;
use crate::theme::{ClassMethod, ThemeMode};
use crate::topo::FaceKind;

/// Small xorshift RNG (deterministic, no dependency).
pub struct Rng(u64);
impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_f64(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
    pub fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next_f64()
    }
}

/// Blobby closed ring around a centre (radii in degrees).
pub fn blob(rng: &mut Rng, c: LonLat, rx: f64, ry: f64, n: usize, rough: f64) -> Vec<LonLat> {
    let phase: Vec<f64> = (0..4).map(|_| rng.range(0.0, std::f64::consts::TAU)).collect();
    (0..n)
        .map(|i| {
            let t = i as f64 / n as f64 * std::f64::consts::TAU;
            let k = 1.0 + rough * (0.5 * (3.0 * t + phase[0]).sin() + 0.3 * (5.0 * t + phase[1]).sin() + 0.2 * (9.0 * t + phase[2]).sin());
            LonLat::new(c.lon + rx * k * t.cos(), (c.lat + ry * k * t.sin()).clamp(-85.0, 85.0)).normalized()
        })
        .collect()
}

/// Cut the face containing `center` along a wiggly line. The cut ends where
/// rays from `center` towards `dir_a` / `dir_b` first meet a boundary.
fn cut(doc: &mut Document, rng: &mut Rng, center: LonLat, dir_a: LonLat, dir_b: LonLat, wiggle: f64, steps: usize) -> bool {
    let (Some(a), Some(b)) = (doc.topo.first_hit(center, dir_a), doc.topo.first_hit(center, dir_b)) else {
        return false;
    };
    let mut pts = vec![a];
    let dx = crate::geom::lon_delta(a.lon, b.lon);
    let (nx, ny) = (-(b.lat - a.lat), dx);
    let len = (nx * nx + ny * ny).sqrt().max(1e-9);
    for i in 1..steps {
        let t = i as f64 / steps as f64;
        let w = rng.range(-wiggle, wiggle) * (std::f64::consts::PI * t).sin();
        pts.push(LonLat::new(a.lon + dx * t + nx / len * w, a.lat + (b.lat - a.lat) * t + ny / len * w).normalized());
    }
    pts.push(b);
    match doc.split_face(&pts) {
        Ok(_) => true,
        Err(e) => {
            if std::env::var_os("ATLAS_DEBUG").is_some() {
                eprintln!("cut at {center:?} failed: {e}");
            }
            false
        }
    }
}

/// Cut through `center` along the line from `p` to `q` (both may lie outside).
fn cut_line(doc: &mut Document, rng: &mut Rng, p: LonLat, q: LonLat, wiggle: f64, steps: usize) -> bool {
    let c = LonLat::new(p.lon + crate::geom::lon_delta(p.lon, q.lon) * 0.5, (p.lat + q.lat) * 0.5).normalized();
    let far = |t: LonLat| LonLat::new(c.lon + crate::geom::lon_delta(c.lon, t.lon) * 3.0, (c.lat + (t.lat - c.lat) * 3.0).clamp(-89.0, 89.0)).normalized();
    cut(doc, rng, c, far(p), far(q), wiggle, steps)
}

/// A rectangular continent subdivided into `nx * ny` countries, each with
/// population/GDP data. `detail` = vertices per cut line.
pub fn grid_world(nx: usize, ny: usize, detail: usize, seed: u64) -> Document {
    let mut rng = Rng::new(seed);
    let mut doc = Document::new(&format!("网格世界 {nx}x{ny}"), 6371.0088);
    let (x0, x1, y0, y1) = (-150.0, 150.0, -60.0, 70.0);
    // continent outline with many vertices along each side
    let per_side = (detail * nx).max(8);
    let mut ring = Vec::new();
    for i in 0..per_side {
        ring.push(LonLat::new(x0 + (x1 - x0) * i as f64 / per_side as f64, y0));
    }
    let per_v = (detail * ny).max(8);
    for i in 0..per_v {
        ring.push(LonLat::new(x1, y0 + (y1 - y0) * i as f64 / per_v as f64));
    }
    for i in 0..per_side {
        ring.push(LonLat::new(x1 - (x1 - x0) * i as f64 / per_side as f64, y1));
    }
    for i in 0..per_v {
        ring.push(LonLat::new(x0, y1 - (y1 - y0) * i as f64 / per_v as f64));
    }
    doc.add_ring_face(&ring, FaceKind::Land).expect("continent");
    let cw = (x1 - x0) / nx as f64;
    let ch = (y1 - y0) / ny as f64;
    let ym = (y0 + y1) * 0.5;
    for i in 1..nx {
        let x = x0 + cw * i as f64;
        cut(&mut doc, &mut rng, LonLat::new(x, ym), LonLat::new(x, y0 - 5.0), LonLat::new(x, y1 + 5.0), cw * 0.15, detail * ny);
    }
    for i in 0..nx {
        let xc = x0 + cw * (i as f64 + 0.5);
        for j in 1..ny {
            let y = y0 + ch * j as f64;
            cut(&mut doc, &mut rng, LonLat::new(xc, y), LonLat::new(xc - cw, y), LonLat::new(xc + cw, y), ch * 0.15, detail.max(2));
        }
    }
    let pop = doc.add_field("population", "人口", FieldKind::Number, None).unwrap();
    let gdp = doc.add_field("gdp", "GDP (亿)", FieldKind::Number, None).unwrap();
    doc.add_field("density", "人口密度", FieldKind::Number, Some("population / area_km2".into())).unwrap();
    doc.add_field("gdp_pc", "人均 GDP", FieldKind::Number, Some("gdp * 100000000 / population".into())).unwrap();
    let faces: Vec<_> = doc.topo.faces.keys().copied().filter(|f| *f != crate::BACKGROUND).collect();
    for (n, f) in faces.into_iter().enumerate() {
        let e = doc.add_entity(EntityKind::Country, &format!("国家{}", n + 1));
        doc.assign(f, Some(e)).unwrap();
        let p = rng.range(1e5, 5e7).round();
        doc.set_value(e, pop, None, Some(Value::Num(p)));
        doc.set_value(e, gdp, None, Some(Value::Num((p * rng.range(500.0, 50000.0) / 1e8).round())));
    }
    doc
}

/// Hand-tuned demo world showing the main features.
pub fn demo_world() -> Document {
    let mut rng = Rng::new(7);
    let mut doc = Document::new("示例世界 · 艾瑞亚", 6371.0088);
    let west = doc.add_ring_face(&blob(&mut rng, LonLat::new(-60.0, 20.0), 38.0, 30.0, 140, 0.18), FaceKind::Land).unwrap();
    let _ = west;
    doc.add_ring_face(&blob(&mut rng, LonLat::new(60.0, 5.0), 32.0, 35.0, 140, 0.2), FaceKind::Land).unwrap();
    // island chain across the antimeridian
    doc.add_ring_face(&blob(&mut rng, LonLat::new(178.0, -20.0), 9.0, 6.0, 60, 0.15), FaceKind::Land).unwrap();
    doc.add_ring_face(&blob(&mut rng, LonLat::new(150.0, 30.0), 5.0, 4.0, 40, 0.2), FaceKind::Land).unwrap();
    // inland lake
    doc.add_ring_face(&blob(&mut rng, LonLat::new(52.0, 8.0), 5.0, 4.0, 40, 0.1), FaceKind::Water).unwrap();

    let cuts = [
        (LonLat::new(-60.0, -15.0), LonLat::new(-60.0, 55.0)),
        (LonLat::new(-100.0, 22.0), LonLat::new(-60.0, 22.0)),
        (LonLat::new(-60.0, 15.0), LonLat::new(-22.0, 15.0)),
        (LonLat::new(60.0, -32.0), LonLat::new(60.0, 42.0)),
        (LonLat::new(25.0, 10.0), LonLat::new(60.0, 22.0)),
        (LonLat::new(60.0, 0.0), LonLat::new(95.0, -5.0)),
        (LonLat::new(178.0, -27.0), LonLat::new(178.0, -13.0)),
    ];
    for (a, b) in cuts {
        cut_line(&mut doc, &mut rng, a, b, 2.5, 18);
    }
    let names = ["北原王国", "南岭共和国", "西海联邦", "东陵公国", "高地邦", "河谷国", "沙洲联盟", "白鹭国", "雾港", "日界群岛", "珊瑚国", "远岛"];
    let pop = doc.add_field("population", "人口", FieldKind::Number, None).unwrap();
    // the base population is the 2020 figure; years in between are interpolated
    {
        let f = doc.fields.get_mut(&pop).unwrap();
        f.base_year = Some(2020);
        f.inherit = Some(crate::doc::Inherit::Interpolate);
    }
    let gdp = doc.add_field("gdp", "GDP (亿)", FieldKind::Number, None).unwrap();
    doc.fields.get_mut(&gdp).unwrap().base_year = Some(2020);
    doc.add_field("density", "人口密度", FieldKind::Number, Some("population / area_km2".into())).unwrap();
    doc.add_field("gdp_pc", "人均 GDP", FieldKind::Number, Some("gdp * 100000000 / population".into())).unwrap();
    let gov = doc.add_field("government", "政体", FieldKind::Category, None).unwrap();
    let govs = ["君主制", "共和制", "联邦制"];
    let land: Vec<_> =
        doc.topo.faces.iter().filter(|(id, f)| **id != crate::BACKGROUND && f.kind == FaceKind::Land).map(|(id, _)| *id).collect();
    for (i, f) in land.iter().enumerate() {
        let e = doc.add_entity(EntityKind::Country, names[i % names.len()]);
        doc.assign(*f, Some(e)).unwrap();
        let p = rng.range(2e6, 8e7).round();
        doc.set_value(e, pop, None, Some(Value::Num(p)));
        doc.set_value(e, pop, Some(1900), Some(Value::Num((p * 0.3).round())));
        doc.set_value(e, gdp, None, Some(Value::Num((p * rng.range(800.0, 40000.0) / 1e8).round())));
        doc.set_value(e, gov, None, Some(Value::Text(govs[i % 3].into())));
    }
    for (name, pos) in [("霜城", LonLat::new(-70.0, 38.0)), ("南港", LonLat::new(-55.0, 0.0)), ("东京湾", LonLat::new(80.0, 10.0)), ("日界城", LonLat::new(179.5, -20.0))] {
        doc.add_city(pos, name);
    }
    let river: Vec<LonLat> = (0..20).map(|i| LonLat::new(-85.0 + i as f64 * 1.4, 35.0 - i as f64 * 0.9 + (i as f64 * 0.8).sin() * 1.5)).collect();
    doc.rivers.insert(LineId(doc.topo.alloc()), LineFeature { name: "长河".into(), pts: river, route: None, legend: String::new(), show_name: false, hidden: false });
    doc.theme.method = ClassMethod::Quantile;
    doc.theme.mode = ThemeMode::Political;
    doc
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn demo_and_grid_are_valid() {
        let d = demo_world();
        d.topo.validate().unwrap();
        assert!(d.countries().len() >= 8, "countries: {}", d.countries().len());
        let g = grid_world(6, 4, 4, 1);
        g.topo.validate().unwrap();
        assert_eq!(g.countries().len(), 24);
    }
}
