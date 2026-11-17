//! Measuring lengths and areas on the document's sphere.

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::measure::{great_circle, great_circle_path};

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

/// A quarter of the equator is a quarter of the circumference; the path
/// between two points stays on their great circle and ends on them.
#[test]
fn lengths_follow_great_circles() {
    let d = Document::default();
    let r = d.radius();
    let quarter = d.path_length_km(&[ll(0.0, 0.0), ll(45.0, 0.0), ll(90.0, 0.0)]);
    assert!((quarter - std::f64::consts::FRAC_PI_2 * r).abs() < 1e-6 * r);
    // pole to equator is the same quarter
    assert!((great_circle(ll(10.0, 90.0), ll(10.0, 0.0), r) - quarter).abs() < 1e-6 * r);
    // the great-circle path between two points at 60° N bulges towards the pole
    let p = great_circle_path(ll(-60.0, 60.0), ll(60.0, 60.0), 1.0);
    assert_eq!((p[0], *p.last().unwrap()), (ll(-60.0, 60.0), ll(60.0, 60.0)));
    assert!(p.iter().any(|q| q.lat > 65.0));
    let along: f64 = p.windows(2).map(|w| great_circle(w[0], w[1], r)).sum();
    assert!((along - great_circle(ll(-60.0, 60.0), ll(60.0, 60.0), r)).abs() < 1e-6 * r);
}

/// The triangle with corners on the equator at 0° and 90° E and at the
/// north pole is an eighth of the sphere; a small square is about
/// side²; winding does not matter.
#[test]
fn areas_of_great_circle_rings() {
    let d = Document::default();
    let r = d.radius();
    let octant = d.ring_area_km2(&[ll(0.0, 0.0), ll(90.0, 0.0), ll(0.0, 90.0)]);
    let sphere = 4.0 * std::f64::consts::PI * r * r;
    assert!((octant / (sphere / 8.0) - 1.0).abs() < 1e-3, "{octant}");
    let sq = [ll(0.0, 0.0), ll(1.0, 0.0), ll(1.0, 1.0), ll(0.0, 1.0)];
    let side = great_circle(ll(0.0, 0.0), ll(1.0, 0.0), r);
    assert!((d.ring_area_km2(&sq) / (side * side) - 1.0).abs() < 0.01);
    let mut rev = sq.to_vec();
    rev.reverse();
    assert!((d.ring_area_km2(&rev) - d.ring_area_km2(&sq)).abs() < 1e-6 * side * side);
    assert_eq!(d.ring_area_km2(&sq[..2]), 0.0);
}
