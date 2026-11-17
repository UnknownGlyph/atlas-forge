//! Finding and editing rivers after they are drawn.

use map_core::doc::{Document, LineFeature, LineId};
use map_core::geom::LonLat;

fn ll(x: f64, y: f64) -> LonLat {
    LonLat::new(x, y)
}

fn with_river() -> (Document, LineId) {
    let mut d = Document::default();
    let id = LineId(d.topo.alloc());
    d.rivers.insert(id, LineFeature { name: "河".into(), pts: vec![ll(0.0, 0.0), ll(10.0, 0.0), ll(10.0, 10.0)], route: None, legend: String::new(), show_name: false, hidden: false });
    (d, id)
}

#[test]
fn rivers_are_found_by_point_and_by_line() {
    let (d, id) = with_river();
    assert_eq!(d.river_point_at(ll(10.1, 0.1), 0.5).map(|x| (x.0, x.1)), Some((id, 1)));
    assert!(d.river_point_at(ll(5.0, 0.1), 0.5).is_none());
    let (rid, seg, q) = d.river_at(ll(5.0, 0.2), 0.5).unwrap();
    assert_eq!((rid, seg), (id, 0));
    assert!((q.lon - 5.0).abs() < 1e-9 && q.lat.abs() < 1e-9);
    assert_eq!(d.river_at(ll(10.3, 6.0), 0.5).map(|x| x.1), Some(1));
    assert!(d.river_at(ll(5.0, 5.0), 0.5).is_none());
    // 20° along a great circle on the default sphere, roughly
    let km = d.river_length_km(id);
    let deg = std::f64::consts::PI / 180.0 * d.radius();
    assert!((km - 20.0 * deg).abs() < 0.02 * km, "{km}");
}

#[test]
fn river_points_can_be_deleted_down_to_two() {
    let (mut d, id) = with_river();
    d.delete_river_point(id, 1).unwrap();
    assert_eq!(d.rivers[&id].pts, vec![ll(0.0, 0.0), ll(10.0, 10.0)]);
    assert!(d.delete_river_point(id, 0).is_err());
    assert_eq!(d.rivers[&id].pts.len(), 2);
}
