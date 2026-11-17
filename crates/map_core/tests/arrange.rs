//! Several drawn features worked on together: moved, turned and scaled on
//! the sphere, lined up, spaced, copied and pasted, deleted, locked.

use map_core::area::AreaMark;
use map_core::arrange::{centre_of, rotate_about, scale_about, Align};
use map_core::doc::{Annotation, Document, LineFeature, LineId};
use map_core::features::FeatureRef;
use map_core::measure::great_circle;
use map_core::symbol::{MapSymbol, SymbolKind};
use map_core::LonLat;

const R: f64 = 6371.0;

fn dist(a: LonLat, b: LonLat) -> f64 {
    great_circle(a, b, R)
}

/// A symbol, a text, a route and an area, around (lon, 40°N).
fn world(lon: f64) -> (Document, Vec<FeatureRef>) {
    let mut d = Document::default();
    let mut id = || LineId(d.topo.alloc());
    let (s, t, l, a) = (id(), id(), id(), id());
    d.symbols.insert(s, MapSymbol::new(SymbolKind::Peak, LonLat::new(lon, 40.0)));
    let mut text = Annotation::new("海", LonLat::new(lon + 4.0, 42.0));
    text.angle = 10.0;
    d.annotations.insert(t, text);
    d.rivers.insert(l, LineFeature { name: "路".into(), pts: vec![LonLat::new(lon - 3.0, 38.0), LonLat::new(lon, 39.0), LonLat::new(lon + 3.0, 38.0)], route: Some(Default::default()), legend: String::new(), show_name: false, hidden: false });
    let mut area = AreaMark::new(vec![LonLat::new(lon + 6.0, 36.0), LonLat::new(lon + 9.0, 36.0), LonLat::new(lon + 9.0, 39.0), LonLat::new(lon + 6.0, 39.0)]);
    area.extra = vec![vec![LonLat::new(lon + 7.0, 37.0), LonLat::new(lon + 8.0, 37.0), LonLat::new(lon + 8.0, 38.0)]];
    d.areas.insert(a, area);
    (d, vec![FeatureRef::Symbol(s), FeatureRef::Text(t), FeatureRef::Route(l), FeatureRef::Area(a)])
}

/// Every distance between the features' points, as a fingerprint of the
/// group's shape.
fn shape(d: &Document, fs: &[FeatureRef]) -> Vec<f64> {
    let pts: Vec<LonLat> = fs.iter().flat_map(|f| d.feature_points(*f)).collect();
    let mut out = Vec::new();
    for i in 0..pts.len() {
        for j in i + 1..pts.len() {
            out.push(dist(pts[i], pts[j]));
        }
    }
    out
}

fn close(a: &[f64], b: &[f64], tol: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tol)
}

#[test]
fn turning_and_scaling_points() {
    let c = LonLat::new(10.0, 50.0);
    let p = LonLat::new(10.0, 52.0);
    // a quarter turn counter-clockwise: due north becomes due west
    let q = rotate_about(p, c, 90.0);
    assert!((dist(c, q) - dist(c, p)).abs() < 1e-6);
    assert!(q.lon < c.lon && (q.lat - 50.0).abs() < 0.1, "{q:?}");
    let back = rotate_about(q, c, -90.0);
    assert!(dist(back, p) < 1e-6);
    // twice as far along the same great circle
    let far = scale_about(p, c, 2.0);
    assert!((dist(c, far) - 2.0 * dist(c, p)).abs() < 1e-6);
    assert!((far.lon - 10.0).abs() < 1e-9 && far.lat > 53.9);
    assert_eq!(scale_about(c, c, 3.0), c);
    assert!(dist(centre_of(&[LonLat::new(179.0, 0.0), LonLat::new(-179.0, 0.0)]).unwrap(), LonLat::new(180.0, 0.0)) < 1e-6, "across the antimeridian");
}

#[test]
fn a_group_keeps_its_shape_when_moved_or_turned() {
    for lon in [20.0, 176.0] {
        let (mut d, fs) = world(lon);
        let before = shape(&d, &fs);
        let c = d.features_centre(&fs).unwrap();
        // moved far north-east, across the antimeridian for the second world
        d.move_features(&fs, c, LonLat::new(lon + 30.0, 65.0)).unwrap();
        assert!(close(&shape(&d, &fs), &before, 1e-6), "moving keeps every distance");
        assert!(dist(d.features_centre(&fs).unwrap(), LonLat::new(lon + 30.0, 65.0)) < 1e-6);
        let c = d.features_centre(&fs).unwrap();
        d.rotate_features(&fs, c, 37.0).unwrap();
        assert!(close(&shape(&d, &fs), &before, 1e-6), "turning keeps every distance");
        assert!(dist(d.features_centre(&fs).unwrap(), c) < 1e-6);
        // the straight text turned with the group
        let FeatureRef::Text(t) = fs[1] else { unreachable!() };
        assert!((d.annotations[&t].angle - 47.0).abs() < 1e-4);
        // scaled: every distance from the centre doubles
        let far: Vec<f64> = fs.iter().flat_map(|f| d.feature_points(*f)).map(|p| dist(c, p)).collect();
        d.scale_features(&fs, c, 2.0).unwrap();
        let now: Vec<f64> = fs.iter().flat_map(|f| d.feature_points(*f)).map(|p| dist(c, p)).collect();
        assert!(far.iter().zip(&now).all(|(a, b)| (b - 2.0 * a).abs() < 1e-6));
        assert!(d.scale_features(&fs, c, 0.0).is_err() && d.rotate_features(&fs, c, f64::NAN).is_err());
        // every longitude stays in range
        assert!(fs.iter().flat_map(|f| d.feature_points(*f)).all(|p| (-180.0..=180.0).contains(&p.lon)));
    }
}

#[test]
fn lining_up_and_spacing() {
    let mut d = Document::default();
    let mut fs = Vec::new();
    for (lon, lat) in [(10.0, 5.0), (14.0, 9.0), (30.0, 1.0), (22.0, 7.0)] {
        let id = LineId(d.topo.alloc());
        d.symbols.insert(id, MapSymbol::new(SymbolKind::Port, LonLat::new(lon, lat)));
        fs.push(FeatureRef::Symbol(id));
    }
    let at = |d: &Document| -> Vec<(f64, f64)> { fs.iter().map(|f| d.feature_anchor(*f).unwrap()).map(|p| ((p.lon * 1e6).round() / 1e6, (p.lat * 1e6).round() / 1e6)).collect() };
    let mut a = d.clone();
    a.align_features(&fs, Align::West).unwrap();
    assert_eq!(at(&a), [(10.0, 5.0), (10.0, 9.0), (10.0, 1.0), (10.0, 7.0)]);
    let mut a = d.clone();
    a.align_features(&fs, Align::MidLat).unwrap();
    assert_eq!(at(&a), [(10.0, 5.0), (14.0, 5.0), (30.0, 5.0), (22.0, 5.0)]);
    let mut a = d.clone();
    a.align_features(&fs, Align::East).unwrap();
    assert!(at(&a).iter().all(|p| p.0 == 30.0));
    // spaced evenly by longitude between the outermost two
    let mut a = d.clone();
    a.distribute_features(&fs, true).unwrap();
    let mut lons: Vec<f64> = at(&a).iter().map(|p| p.0).collect();
    lons.sort_by(|x, y| x.total_cmp(y));
    assert!(lons.windows(2).all(|w| (w[1] - w[0] - 20.0 / 3.0).abs() < 1e-5), "{lons:?}");
    assert!(d.clone().align_features(&fs[..1], Align::West).is_err());
    assert!(d.clone().distribute_features(&fs[..2], true).is_err());
    // across the antimeridian: lined up where they are, not on the far side
    let mut d = Document::default();
    let mut fs = Vec::new();
    for lon in [178.0, -179.0, -176.0] {
        let id = LineId(d.topo.alloc());
        d.symbols.insert(id, MapSymbol::new(SymbolKind::Port, LonLat::new(lon, 0.0)));
        fs.push(FeatureRef::Symbol(id));
    }
    d.align_features(&fs, Align::West).unwrap();
    assert!(fs.iter().all(|f| (d.feature_anchor(*f).unwrap().lon - 178.0).abs() < 1e-6));
}

#[test]
fn copying_pasting_deleting_and_locking() {
    let (mut d, fs) = world(20.0);
    let clip = d.copy_features(&fs);
    assert_eq!(clip.items.len(), 4);
    // pasted elsewhere: new features of the same kinds and shape
    let to = LonLat::new(-60.0, -10.0);
    let new = d.paste_features(&clip, Some(to)).unwrap();
    assert_eq!(new.len(), 4);
    assert!(new.iter().zip(&fs).all(|(a, b)| std::mem::discriminant(a) == std::mem::discriminant(b) && a != b));
    assert!(close(&shape(&d, &new), &shape(&d, &fs), 1e-6));
    assert!(dist(d.features_centre(&new).unwrap(), to) < 1e-6);
    assert_eq!(d.features().len(), 8);
    // pasted in place into another document
    let mut other = Document::default();
    let there = other.paste_features(&clip, None).unwrap();
    assert!(close(&shape(&other, &there), &shape(&d, &fs), 1e-9));
    assert_eq!(other.feature_points(there[2]), d.feature_points(fs[2]));
    // duplicates sit beside the originals
    let dup = d.duplicate_features(&fs[..2], 1.0, -1.0).unwrap();
    assert_eq!((dup.len(), d.features().len()), (2, 10));
    // locking, and deleting takes the lock with it
    d.set_locked(fs[0], true).unwrap();
    assert!(d.is_locked(fs[0]) && !d.is_locked(fs[1]));
    assert_eq!(d.delete_features(&fs), 4);
    assert!(!d.is_locked(fs[0]) && d.locked.is_empty());
    assert_eq!(d.features().len(), 6);
    assert_eq!(d.delete_features(&fs), 0, "already gone");
    assert!(d.set_locked(fs[0], true).is_err());
}
