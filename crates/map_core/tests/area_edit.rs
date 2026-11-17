//! Editing a hatched area's outline point by point.

use map_core::area::AreaMark;
use map_core::doc::{Document, LineId};
use map_core::geom::LonLat;

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

/// Points are found by point and by side (the closing side included),
/// moved, inserted where the side was hit, and deleted down to three.
#[test]
fn area_points_move_insert_and_delete() {
    let mut d = Document::default();
    let id = LineId(d.topo.alloc());
    d.areas.insert(id, AreaMark::new(vec![ll(0.0, 0.0), ll(10.0, 0.0), ll(10.0, 10.0), ll(0.0, 10.0)]));
    assert_eq!(d.area_point_at(ll(10.2, 9.9), 0.5).map(|x| (x.0, x.1)), Some((id, 2)));
    assert!(d.area_point_at(ll(5.0, 5.0), 0.5).is_none());
    // the closing side, from the last point back to the first
    let (hit, side, at) = d.area_on_edge(ll(-0.1, 5.0), 0.5).unwrap();
    assert_eq!((hit, side), (id, 3));
    assert!((at.lon - 0.0).abs() < 1e-9 && (at.lat - 5.0).abs() < 1e-6);
    let i = d.insert_area_point(id, side, at).unwrap();
    assert_eq!((i, d.areas[&id].ring.len()), (4, 5));
    d.move_area_point(id, i, ll(-3.0, 5.0)).unwrap();
    assert_eq!(d.areas[&id].ring[4], ll(-3.0, 5.0));
    assert!(d.move_area_point(id, 9, ll(0.0, 0.0)).is_err());
    d.delete_area_point(id, 4).unwrap();
    d.delete_area_point(id, 0).unwrap();
    assert_eq!(d.areas[&id].ring, vec![ll(10.0, 0.0), ll(10.0, 10.0), ll(0.0, 10.0)]);
    assert!(d.delete_area_point(id, 0).is_err(), "an area keeps three points");
    assert_eq!(d.area_edge_at(ll(10.1, 5.0), 0.5), Some(id));
}
