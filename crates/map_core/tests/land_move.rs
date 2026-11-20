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
