//! Moving and copying drawn layout.

use map_core::area::AreaMark;
use map_core::copy::moved;
use map_core::doc::{Annotation, Document, LineFeature, LineId};
use map_core::geom::LonLat;
use map_core::symbol::{MapSymbol, SymbolKind};

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

/// Great-circle distance in degrees.
fn dist(a: LonLat, b: LonLat) -> f64 {
    let (p1, p2) = (a.lat.to_radians(), b.lat.to_radians());
    let dl = (b.lon - a.lon).to_radians();
    (p1.sin() * p2.sin() + p1.cos() * p2.cos() * dl.cos()).clamp(-1.0, 1.0).acos().to_degrees()
}

/// A move takes the grabbed point exactly to the cursor and keeps every
/// distance, also across the antimeridian and towards a pole.
#[test]
fn moving_keeps_true_shape_and_size() {
    let pts = vec![ll(170.0, 10.0), ll(178.0, 12.0), ll(175.0, 20.0), ll(168.0, 18.0)];
    for (from, to) in [(ll(170.0, 10.0), ll(-175.0, 15.0)), (ll(175.0, 15.0), ll(20.0, 70.0)), (ll(0.0, 0.0), ll(0.0, 0.0))] {
        let m = moved(&pts, from, to);
        let back = moved(&[from], from, to)[0];
        assert!(dist(back, to) < 1e-6, "{back:?} {to:?}");
        for i in 0..pts.len() {
            for j in 0..pts.len() {
                assert!((dist(pts[i], pts[j]) - dist(m[i], m[j])).abs() < 1e-6);
            }
        }
    }
    // the plain lon/lat shift this replaces would not keep the size
    let up: Vec<LonLat> = pts.iter().map(|p| ll(p.lon, p.lat + 50.0)).collect();
    assert!((dist(pts[0], pts[1]) - dist(up[0], up[1])).abs() > 1.0);
}

/// Copies are new objects with everything but their place kept; the
/// original is untouched; the copy has the original's true shape.
/// Points can be replaced, never below the minimum.
#[test]
fn copies_keep_everything_but_the_place() {
    let mut d = Document::default();
    let line = LineId(d.topo.alloc());
    d.rivers.insert(line, LineFeature { name: "商路".into(), pts: vec![ll(0.0, 0.0), ll(10.0, 0.0)], route: Some(Default::default()), legend: "商路".into(), show_name: false, hidden: false });
    let area = LineId(d.topo.alloc());
    let mut a = AreaMark::new(vec![ll(0.0, 0.0), ll(5.0, 0.0), ll(5.0, 5.0)]);
    a.name = "争议区".into();
    a.from = Some(1900);
    d.areas.insert(area, a);
    let sym = LineId(d.topo.alloc());
    d.symbols.insert(sym, MapSymbol::new(SymbolKind::Fort, ll(1.0, 1.0)));
    let ann = LineId(d.topo.alloc());
    d.annotations.insert(ann, Annotation::new("海", ll(2.0, 2.0)));

    let l2 = d.duplicate_line(line, 1.0, -1.0).unwrap();
    assert_ne!(l2, line);
    let p = &d.rivers[&l2].pts;
    assert!(dist(p[0], ll(1.0, -1.0)) < 1e-6 && (dist(p[0], p[1]) - 10.0).abs() < 1e-6);
    assert_eq!((d.rivers[&l2].route, &d.rivers[&l2].legend), (d.rivers[&line].route, &d.rivers[&line].legend));
    assert_eq!(d.rivers[&line].pts, vec![ll(0.0, 0.0), ll(10.0, 0.0)]);
    let a2 = d.duplicate_area(area, 1.0, -1.0).unwrap();
    let (r1, r2) = (&d.areas[&area].ring, &d.areas[&a2].ring);
    assert!((dist(r1[1], r1[2]) - dist(r2[1], r2[2])).abs() < 1e-6 && dist(r2[0], ll(1.0, -1.0)) < 1e-6);
    assert_eq!((d.areas[&a2].name.as_str(), d.areas[&a2].from), ("争议区", Some(1900)));
    let s2 = d.duplicate_symbol(sym, 1.0, -1.0).unwrap();
    assert!(dist(d.symbols[&s2].at, ll(2.0, 0.0)) < 1e-6 && d.symbols[&s2].kind == SymbolKind::Fort);
    let t2 = d.duplicate_annotation(ann, 1.0, -1.0).unwrap();
    assert!(dist(d.annotations[&t2].at, ll(3.0, 1.0)) < 1e-6 && d.annotations[&t2].text == "海");
    assert_eq!((d.rivers.len(), d.areas.len(), d.symbols.len(), d.annotations.len()), (2, 2, 2, 2));
    assert!(d.duplicate_area(LineId(9999), 0.0, 0.0).is_err());

    assert!(d.set_line_points(line, vec![ll(0.0, 0.0)]).is_err());
    assert!(d.set_area_ring(area, vec![ll(0.0, 0.0), ll(1.0, 0.0)]).is_err());
    let ring = moved(&d.areas[&area].ring.clone(), ll(0.0, 0.0), ll(0.0, 10.0));
    d.set_area_ring(area, ring).unwrap();
    assert!(dist(d.areas[&area].ring[0], ll(0.0, 10.0)) < 1e-6);
}
