use map_core::{
    doc::{EntityKind, LayerKind, POLITICAL},
    history::History,
    overlay::{self, DrawIntent},
    Document, FaceKind, LonLat,
};
use map_io::project;

fn rect(left: f64, bottom: f64, right: f64, top: f64) -> [LonLat; 4] {
    [
        LonLat::new(left, bottom),
        LonLat::new(right, bottom),
        LonLat::new(right, top),
        LonLat::new(left, top),
    ]
}

#[test]
fn layer_rename_preserves_identity_members_and_undo() {
    let mut doc = Document::default();
    let land = doc
        .add_ring_face(&rect(0.0, 0.0, 8.0, 8.0), FaceKind::Land)
        .unwrap();
    let layer = doc.add_layer_of("分区层 1", LayerKind::Exclusive);
    let shares = doc.add_layer_of("文化", LayerKind::Shares);
    doc.set_active_layer(layer).unwrap();
    let unit = doc.add_entity(EntityKind::Country, "东部");
    doc.assign(land, Some(unit)).unwrap();
    let mut undo = History::default();
    undo.apply(&mut doc, "重命名分区层", |d| {
        d.rename_layer(layer, "  实际控制  ")
    })
    .unwrap();
    assert_eq!(doc.layer_name(layer), "实际控制");
    assert!(doc.rename_layer(layer, "  ").is_err());
    assert!(doc
        .rename_layer(map_core::doc::LayerId(u64::MAX), "不存在")
        .is_err());
    undo.undo(&mut doc).unwrap();
    assert_eq!(doc.layer_name(layer), "分区层 1");
    undo.redo(&mut doc).unwrap();
    doc.rename_layer(shares, "语言").unwrap();
    doc.set_active_layer(POLITICAL).unwrap();
    let mut loaded = project::from_bytes(&project::to_bytes(&doc).unwrap()).unwrap();
    assert_eq!(loaded.layer_name(layer), "实际控制");
    assert_eq!(loaded.layer_name(shares), "语言");
    loaded.set_active_layer(layer).unwrap();
    assert_eq!(loaded.owner.get(&land), Some(&unit));
    assert_eq!(loaded.entities[&unit].layer, layer);
}

#[test]
fn provincial_outline_cannot_claim_neighbours_water_or_existing_provinces() {
    let mut doc = Document::default();
    doc.add_ring_face(&rect(0.0, 0.0, 20.0, 12.0), FaceKind::Land)
        .unwrap();
    doc.split_face(&[LonLat::new(10.0, 0.0), LonLat::new(10.0, 12.0)])
        .unwrap();
    let country = doc.add_entity(EntityKind::Country, "西国");
    let neighbour = doc.add_entity(EntityKind::Country, "东国");
    let west = doc.topo.face_at(LonLat::new(1.0, 1.0));
    let east = doc.topo.face_at(LonLat::new(14.0, 1.0));
    doc.assign(west, Some(country)).unwrap();
    doc.assign(east, Some(neighbour)).unwrap();
    doc.add_ring_face(&rect(3.0, 3.0, 5.0, 5.0), FaceKind::Water)
        .unwrap();
    let mut draft = doc.clone();
    let existing = draft.add_subdivision(country, "旧省").unwrap();
    let (mut doc, _) = overlay::apply(
        &draft,
        &rect(6.0, 3.0, 8.0, 6.0),
        DrawIntent::Subdivide {
            parent: country,
            child: existing,
        },
    )
    .unwrap();
    doc.set_year(Some(100));
    let province = doc.add_subdivision(country, "新省").unwrap();
    let (mut result, report) = overlay::apply(
        &doc,
        &rect(-2.0, 2.0, 15.0, 8.0),
        DrawIntent::Subdivide {
            parent: country,
            child: province,
        },
    )
    .unwrap();
    assert_eq!(report.added_km2, 0.0);
    assert_eq!(report.carved_km2, 0.0);
    assert_eq!(
        report
            .taken
            .iter()
            .map(|(owner, _)| *owner)
            .collect::<Vec<_>>(),
        vec![Some(country)]
    );
    result.topo.validate().unwrap();
    let probe = |x, y| result.topo.face_at(LonLat::new(x, y));
    let target = probe(2.0, 4.0);
    assert_eq!(result.owner_at(target, Some(99)), Some(country));
    assert_eq!(result.owner_at(target, Some(100)), Some(province));
    assert!(!result.exists_in(province, Some(99)));
    assert_eq!(result.parent_at(province, Some(100)), Some(country));
    assert_eq!(result.owner.get(&probe(7.0, 4.0)), Some(&existing));
    assert_eq!(result.owner.get(&probe(12.0, 4.0)), Some(&neighbour));
    for water in [probe(4.0, 4.0), probe(-1.0, 4.0)] {
        assert_eq!(result.topo.faces[&water].kind, FaceKind::Water);
    }
    let neighbour_face = probe(12.0, 4.0);
    let count = result.entities.len();
    assert!(result
        .subdivision_on_face(country, "越权", neighbour_face)
        .is_err());
    assert_eq!(result.entities.len(), count);
    result.set_year(Some(99));
    assert_eq!(result.owner.get(&target), Some(&country));
}

#[test]
fn nested_subdivisions_keep_all_ancestors_after_reload() {
    let mut doc = Document::default();
    let land = doc
        .add_ring_face(&rect(0.0, 0.0, 10.0, 10.0), FaceKind::Land)
        .unwrap();
    let mut units = vec![doc.add_entity(EntityKind::Country, "世界国")];
    doc.assign(land, units.last().copied()).unwrap();
    for level in 1..=24 {
        let unit = doc
            .subdivision_on_face(*units.last().unwrap(), &format!("行政区 {level}"), land)
            .unwrap();
        units.push(unit);
    }
    let loaded = project::from_bytes(&project::to_bytes(&doc).unwrap()).unwrap();
    assert_eq!(loaded.top_ancestor(units[24]), units[0]);
    assert_eq!(loaded.ancestors(units[24]).count(), 25);
    for unit in units {
        assert_eq!(loaded.faces_of_tree(unit), vec![land]);
    }
}
