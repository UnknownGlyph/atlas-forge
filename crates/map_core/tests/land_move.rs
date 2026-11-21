use map_core::{
    build::{self, SourcePolygon},
    doc::{EntityKind, LineFeature, LineId},
    guides::{Axis, Guide},
    topo::{FaceKind, GeoOffset},
    Document, LonLat,
};

fn point(x: f64, y: f64) -> LonLat {
    LonLat::new(x, y)
}

fn ring(w: f64, s: f64, e: f64, n: f64) -> Vec<LonLat> {
    vec![point(w, s), point(e, s), point(e, n), point(w, n)]
}

fn map() -> (Document, [map_core::FaceId; 4]) {
    let lake = ring(2.0, 3.0, 4.0, 5.0);
    let (topo, ids, _) = build::build(&[
        SourcePolygon {
            rings: vec![ring(0.0, 0.0, 8.0, 10.0), lake.clone()],
        },
        SourcePolygon {
            rings: vec![ring(8.0, 0.0, 16.0, 10.0)],
        },
        SourcePolygon { rings: vec![lake] },
        SourcePolygon {
            rings: vec![ring(24.0, 0.0, 33.0, 10.0)],
        },
    ])
    .unwrap();
    let ids: [_; 4] = ids
        .into_iter()
        .map(Option::unwrap)
        .collect::<Vec<_>>()
        .try_into()
        .unwrap();
    let mut doc = Document::default();
    doc.topo = topo;
    doc.topo.faces.get_mut(&ids[2]).unwrap().kind = FaceKind::Water;
    let first = doc.add_entity(EntityKind::Country, "西岸");
    let second = doc.add_entity(EntityKind::Country, "东岸");
    doc.assign(ids[0], Some(first)).unwrap();
    doc.assign(ids[1], Some(second)).unwrap();
    doc.set_year(Some(2900));
    doc.assign(ids[1], Some(first)).unwrap();
    doc.set_year(None);
    (doc, ids)
}

#[test]
fn a_land_block_carries_shared_history_lakes_cities_and_references() {
    let (mut doc, [west, east, lake, island]) = map();
    let city = doc.add_city(point(6.0, 4.0), "古城");
    doc.guides.push(Guide::new(Axis::Latitude, 4.0, "固定纬线"));
    let owner_changes = doc.owner_changes.clone();
    let old_island = doc.topo.faces[&island].clone();
    let moved = doc
        .translate_land(
            &[west],
            GeoOffset {
                lon: -3.0,
                lat: -13.0,
            },
        )
        .unwrap();
    assert_eq!(moved.faces, [west, east, lake].into_iter().collect());
    assert_eq!(doc.owner_changes, owner_changes);
    assert_eq!(doc.topo.faces[&island], old_island);
    assert_eq!(doc.topo.face_at(point(0.0, -9.0)), lake);
    assert_eq!(doc.entities[&city].pos, Some(point(3.0, -9.0)));
    assert_eq!(doc.guides[0].degrees, 4.0);
    doc.topo.validate().unwrap();
}

#[test]
fn collision_or_link_to_unselected_land_leaves_the_whole_document_alone() {
    let (mut doc, [west, _, _, island]) = map();
    let original = doc.clone();
    let error = doc
        .translate_land(
            &[west],
            GeoOffset {
                lon: 23.0,
                lat: 2.0,
            },
        )
        .unwrap_err();
    assert!(error.contains("相交") || error.contains("覆盖"), "{error}");
    assert_eq!(doc, original);
    let touching = doc
        .translate_land(&[west], GeoOffset { lon: 8.0, lat: 0.0 })
        .unwrap_err();
    assert!(
        touching.contains("接触") || touching.contains("相交"),
        "{touching}"
    );
    assert_eq!(doc, original);

    let id = LineId(doc.topo.alloc());
    doc.rivers.insert(
        id,
        LineFeature {
            name: "跨海联络".into(),
            pts: vec![point(5.0, 4.0), point(28.0, 4.0)],
            route: Some(Default::default()),
            legend: String::new(),
            show_name: false,
            hidden: false,
        },
    );
    let before = doc.clone();
    let offset = GeoOffset {
        lon: 0.0,
        lat: -12.0,
    };
    let error = doc.translate_land(&[west], offset).unwrap_err();
    assert!(
        error.contains("跨海联络") && error.contains("未选中"),
        "{error}"
    );
    assert_eq!(doc, before);
    doc.translate_land(&[west, island], offset).unwrap();
    assert_eq!(
        doc.rivers[&id].pts,
        vec![point(5.0, -8.0), point(28.0, -8.0)]
    );
}

#[test]
fn scaling_keeps_one_shared_boundary_and_rejects_conflicts_atomically() {
    let (mut doc, [west, east, lake, island]) = map();
    let city = doc.add_city(point(6.0, 4.0), "古城");
    let pivot = point(8.0, 5.0);
    let before = doc.clone();
    let err = doc.scale_land(west, pivot, 3.0).unwrap_err();
    assert!(err.contains("相交") || err.contains("覆盖"));
    assert_eq!(doc, before);
    assert!(doc.scale_land(west, pivot, -1.0).is_err());
    assert_eq!(doc, before);

    let selection = doc.scale_land(west, pivot, 0.5).unwrap();
    assert_eq!(selection.faces, [west, east, lake].into_iter().collect());
    assert_eq!(doc.topo.face_at(point(5.5, 4.5)), lake);
    assert_eq!(doc.entities[&city].pos, Some(point(7.0, 4.5)));
    assert_eq!(doc.topo.faces[&island], before.topo.faces[&island]);
    assert_eq!(doc.topo.edges.len(), before.topo.edges.len());
    assert_eq!(doc.owner_changes, before.owner_changes);
    doc.topo.validate().unwrap();
}

#[test]
fn disconnected_land_uses_one_pivot_and_one_scale_factor() {
    use map_core::topo::{land_group_center, LandTransform};

    let (mut doc, [west, east, lake, island]) = map();
    let west_city = doc.add_city(point(6.0, 4.0), "西大陆");
    let island_city = doc.add_city(point(28.0, 6.0), "岛上");
    let before = doc.clone();
    let selected = doc.topo.land_block(&[west, island]).unwrap();
    let pivot =
        land_group_center(doc.topo.block_coastlines(&selected).into_iter().flatten()).unwrap();
    assert_eq!(pivot, point(16.5, 5.0));
    let moved = doc
        .transform_land(
            &[west, island],
            LandTransform::Scale {
                pivot,
                factor: 1.25,
            },
        )
        .unwrap();
    assert_eq!(
        moved.faces,
        [west, east, lake, island].into_iter().collect()
    );
    let center = |doc: &Document, city| doc.entities[&city].pos.unwrap();
    let a = center(&doc, west_city);
    let b = center(&doc, island_city);
    assert_eq!(a, point(3.375, 3.75));
    assert_eq!(b, point(30.875, 6.25));
    assert_eq!(
        b.lon - a.lon,
        1.25 * (center(&before, island_city).lon - center(&before, west_city).lon)
    );
    assert_eq!(
        b.lat - a.lat,
        1.25 * (center(&before, island_city).lat - center(&before, west_city).lat)
    );
    assert_eq!(doc.topo.edges.len(), before.topo.edges.len());
    doc.topo.validate().unwrap();
}

#[test]
fn dragging_a_group_preserves_spherical_distance() {
    use map_core::topo::{GlobeRotation, LandTransform};

    let (mut doc, [west, _, _, island]) = map();
    let a = point(6.0, 4.0);
    let b = point(28.0, 6.0);
    let first = doc.add_city(a, "西大陆");
    let second = doc.add_city(b, "岛上");
    let target = point(57.0, 43.0);
    let rotation = LandTransform::Rotate(GlobeRotation::between(a, target).unwrap());
    doc.transform_land(&[west, island], rotation).unwrap();
    let moved_a = doc.entities[&first].pos.unwrap();
    let moved_b = doc.entities[&second].pos.unwrap();

    fn cos_distance(a: LonLat, b: LonLat) -> f64 {
        let (a_lat, b_lat) = (a.lat.to_radians(), b.lat.to_radians());
        a_lat.sin() * b_lat.sin() + a_lat.cos() * b_lat.cos() * (a.lon - b.lon).to_radians().cos()
    }
    assert!((moved_a.lon - target.lon).abs() < 1e-10);
    assert!((moved_a.lat - target.lat).abs() < 1e-10);
    assert!((cos_distance(a, b) - cos_distance(moved_a, moved_b)).abs() < 1e-12);
    doc.topo.validate().unwrap();
}

#[test]
fn group_center_unwraps_a_selection_crossing_the_date_line() {
    use map_core::topo::land_group_center;

    let pivot =
        land_group_center([point(166.0, 8.0), point(-174.0, 16.0), point(175.0, 20.0)]).unwrap();
    assert_eq!(pivot, point(176.0, 14.0));
}
