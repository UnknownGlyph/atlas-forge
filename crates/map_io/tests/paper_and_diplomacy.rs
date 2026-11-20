use map_core::{
    diplomacy::{Agreement, Kind},
    doc::EntityKind,
    paper::Paper,
    world_book::{Link, Node, WorldBook, ROOT},
    Document, LonLat,
};
use map_io::{geojson, png, project, scene, scene::ExportOptions};
use std::collections::BTreeSet;

#[test]
fn canvas_and_event_relations_survive_board_switches() {
    let mut doc = Document::new("远洋洲", 9650.0);
    let country = doc.add_entity(EntityKind::Country, "远洋");
    let other = doc.add_entity(EntityKind::Country, "皮斯");
    let event = doc.add_event("互助协约缔结", 2766, None);
    let mut agreement = Agreement {
        name: "远洋互助协约".into(),
        kind: Kind::Alliance,
        parties: BTreeSet::from([country, other]),
        from: Some(2766),
        until: Some(2897),
        event: Some(event),
        ..Agreement::default()
    };
    let agreement_id = doc.save_agreement(None, agreement.clone()).unwrap();
    assert!(!agreement.visible(Some(2765)));
    assert!(agreement.visible(Some(2766)));
    assert!(agreement.visible(Some(2897)));
    assert!(!agreement.visible(Some(2898)));
    assert!(doc.change_years().contains(&2898));
    agreement.until = Some(2765);
    assert!(doc.save_agreement(Some(agreement_id), agreement).is_err());
    let paper = Paper::default();
    let book = WorldBook::edit(&mut doc);
    let board = book.add_map(Document::new_paper("港口图", paper));
    book.add_link(Link::new(Node::Event(ROOT, event), Node::Map(board), true))
        .unwrap();
    WorldBook::switch(&mut doc, board, None).unwrap();
    assert_eq!(doc.paper, Some(paper));
    let bytes = project::to_bytes(&doc).unwrap();
    assert_eq!(&bytes[8..12], &6_u32.to_le_bytes());
    let mut restored = project::from_bytes(&bytes).unwrap();
    assert!(restored.paper.is_none());
    let record = &restored.diplomacy.as_ref().unwrap().records[&agreement_id];
    assert_eq!(record.event, Some(event));
    assert_eq!(record.until, Some(2897));
    WorldBook::switch(&mut restored, board, None).unwrap();
    assert_eq!(restored.paper, Some(paper));
    assert_eq!(restored.style.ocean, [255; 4]);
}

#[test]
fn white_paper_exports_in_rectangular_pixel_space() {
    let paper = Paper {
        width: 400,
        height: 300,
    };
    let doc = Document::new_paper("独立画布", paper);
    let opts = ExportOptions {
        width: paper.width,
        height: paper.height,
        graticule: true,
        scale_bar: true,
        north_arrow: true,
        ..Default::default()
    };
    let scene = scene::build_scene(&doc, &map_core::theme::evaluate(&doc), &opts);
    assert_eq!((scene.width, scene.height), (400, 300));
    assert!(scene.lines.is_empty() && scene.furniture.is_empty());
    let image = png::render(&scene, None).unwrap();
    assert_eq!(image.data().get(..4).unwrap(), &[255, 255, 255, 255]);
    let [x, y] = paper.half_extents();
    assert_eq!(paper.pixel(LonLat::new(-x, y)), [0.0, 0.0]);
    assert_eq!(paper.pixel(LonLat::new(x, -y)), [400.0, 300.0]);
}

#[test]
fn bundled_tutorial_contains_importable_geometry_and_a_complete_project() {
    let data = include_str!("../../../docs/tutorial/provinces.geojson");
    let (imported, result) = geojson::import_topology(data).unwrap();
    assert_eq!(result.countries, 64);
    assert_eq!(
        imported
            .entities
            .values()
            .filter(|e| e.code.starts_with("YY-"))
            .count(),
        42
    );
    imported.topo.validate().unwrap();
    let doc = project::from_bytes(include_bytes!("../../../docs/tutorial/yuanyang.atlas")).unwrap();
    assert_eq!(doc.year, Some(2897));
    assert_eq!(doc.radius(), 9650.0);
    let main = doc.entity_by_code("YY").unwrap();
    assert_eq!(
        doc.entities
            .values()
            .filter(|e| e.parent == Some(main) && e.kind == EntityKind::Country)
            .count(),
        42
    );
    assert!(doc
        .layer_ids()
        .iter()
        .any(|&layer| doc.layer_name(layer) == "七大地理区"));
    assert_eq!(doc.diplomacy.as_ref().unwrap().records[&0].parties.len(), 5);
    let cities: Vec<_> = doc
        .entities
        .values()
        .filter(|e| e.kind == EntityKind::City)
        .collect();
    assert_eq!(cities.len(), 350);
    let mut city_counts = std::collections::BTreeMap::new();
    let rank = doc.field_by_key("administrative_rank").unwrap();
    let population = doc.field_by_key("population").unwrap();
    let gdp = doc.field_by_key("necessary_gdp").unwrap();
    for (id, city) in doc
        .entities
        .iter()
        .filter(|(_, e)| e.kind == EntityKind::City)
    {
        let at = city.pos.expect("每个城市都有驻地点");
        let face = doc.topo.face_at(at);
        assert_eq!(doc.owner.get(&face).copied(), city.parent, "{}", city.name);
        assert!(doc.raw_value(*id, population).unwrap().as_num().unwrap() > 0.0);
        assert!(doc.raw_value(*id, gdp).unwrap().as_num().unwrap() > 0.0);
        let value = doc.raw_value(*id, rank).unwrap().display();
        *city_counts.entry(value).or_insert(0) += 1;
    }
    assert_eq!(city_counts["直辖市"], 4);
    assert_eq!(city_counts["地级市"], 70);
    assert_eq!(city_counts["地区"], 20);
    assert_eq!(city_counts["县级市"], 234);
    for (country, provinces) in [("YY", 42), ("ML", 5), ("DS", 5), ("XJ", 6), ("PS", 6)] {
        let country = doc.entity_by_code(country).unwrap();
        let children: Vec<_> = doc
            .entities
            .iter()
            .filter(|(_, e)| e.parent == Some(country) && e.kind == EntityKind::Country)
            .map(|(&id, _)| id)
            .collect();
        assert_eq!(children.len(), provinces);
        for key in [population, gdp] {
            let total: f64 = children
                .iter()
                .map(|&id| doc.raw_value(id, key).unwrap().as_num().unwrap())
                .sum();
            let national = doc.raw_value(country, key).unwrap().as_num().unwrap();
            assert!((total - national).abs() < 0.001);
        }
    }
    let book = doc.book.as_ref().unwrap();
    assert!(book
        .boards
        .values()
        .any(|board| board.doc.as_ref().is_some_and(|d| d.paper.is_some())));
    doc.topo.validate().unwrap();
}

#[test]
fn the_redrawn_country_shares_land_borders_with_all_four_neighbours() {
    let doc = project::from_bytes(include_bytes!("../../../docs/tutorial/yuanyang.atlas")).unwrap();
    let main = doc.entity_by_code("YY").unwrap();
    let island = doc.entity_by_code("YY-DDa").unwrap();
    let shipped_country_codes: BTreeSet<_> = doc
        .entities
        .values()
        .filter(|e| {
            e.kind == EntityKind::Country && e.layer == doc.active_layer && e.parent.is_none()
        })
        .map(|e| e.code.as_str())
        .collect();
    assert_eq!(
        shipped_country_codes,
        BTreeSet::from(["YY", "PS", "ML", "XJ", "DS"])
    );
    let mut neighbours = BTreeSet::new();
    let mut detailed_coast = 0;
    for edge in doc.topo.edges.values() {
        let (left, right) = (
            doc.owner.get(&edge.left).copied(),
            doc.owner.get(&edge.right).copied(),
        );
        let sea = edge.left == map_core::BACKGROUND || edge.right == map_core::BACKGROUND;
        if sea {
            detailed_coast += edge.pts.len();
        }
        if [left, right]
            .into_iter()
            .flatten()
            .any(|id| doc.ancestors(id).any(|a| a == island))
        {
            if let (Some(a), Some(b)) = (left, right) {
                assert!(
                    doc.ancestors(a).any(|a| a == island) && doc.ancestors(b).any(|b| b == island),
                    "独岛无外部陆界"
                );
            }
        }
        let (Some(a), Some(b)) = (left, right) else {
            continue;
        };
        let (a, b) = (doc.top_ancestor(a), doc.top_ancestor(b));
        if a == main && a != b {
            neighbours.insert(doc.entities[&b].code.as_str());
        }
        if b == main && a != b {
            neighbours.insert(doc.entities[&a].code.as_str());
        }
    }
    assert_eq!(neighbours, BTreeSet::from(["PS", "ML", "XJ", "DS"]));
    assert!(
        detailed_coast > 1000,
        "detailing should preserve more than {detailed_coast} intermediate coast points"
    );
}

#[test]
fn administrative_typography_carries_through_to_the_export_scene() {
    use map_core::doc::LabelPlace;
    let mut doc = Document::new("字体示范", 6371.0);
    let nation = doc.add_entity(EntityKind::Country, "国家");
    let province = doc.add_subdivision(nation, "省级").unwrap();
    let district = doc.add_subdivision(province, "县级").unwrap();
    for (id, x) in [(nation, -40.0), (province, 0.0), (district, 40.0)] {
        doc.entities.get_mut(&id).unwrap().label = Some(LabelPlace::at(LonLat::new(x, 20.0)));
    }
    let scene = scene::build_scene(
        &doc,
        &map_core::theme::evaluate(&doc),
        &ExportOptions::default(),
    );
    let label = |name| {
        scene
            .labels
            .iter()
            .find(|label| label.text == name)
            .unwrap()
    };
    let (a, b, c) = (label("国家"), label("省级"), label("县级"));
    assert_eq!(
        [a.font.as_deref(), b.font.as_deref(), c.font.as_deref()],
        [Some("SimHei"), Some("SimSun"), Some("KaiTi")]
    );
    assert!(a.bold && !b.bold && !c.bold && a.size > b.size && b.size > c.size);
}
