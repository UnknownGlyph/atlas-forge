//! The P0 acceptance workflow, end to end and headless:
//! create map → draw land → split into countries → edit border → save →
//! reopen → globe-ready geometry → edit statistics → thematic map → export.

use map_core::doc::{Document, EntityKind, FieldKind, Value};
use map_core::history::History;
use map_core::theme::{self, ThemeMode};
use map_core::{FaceKind, LonLat};
use map_io::scene::{build_scene, ExportOptions};

#[test]
fn full_workflow() {
    let dir = std::env::temp_dir().join(format!("atlas_wf_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = Document::new("工作流测试", 6371.0);
    let mut h = History::default();

    // draw a continent crossing the antimeridian, then split it twice
    let ring = [LonLat::new(160.0, -20.0), LonLat::new(-160.0, -20.0), LonLat::new(-160.0, 20.0), LonLat::new(160.0, 20.0)];
    let land = h.apply(&mut doc, "land", |d| d.add_ring_face(&ring, FaceKind::Land)).unwrap();
    let (a, b) = h.apply(&mut doc, "split", |d| d.split_face(&[LonLat::new(180.0, -20.0), LonLat::new(179.0, 0.0), LonLat::new(180.0, 20.0)])).unwrap();
    assert_eq!(a, land);
    let c = doc.topo.snap_boundary(LonLat::new(170.0, -20.0), 0.5).unwrap();
    let d = doc.topo.snap_boundary(LonLat::new(170.0, 20.0), 0.5).unwrap();
    h.apply(&mut doc, "split2", |x| x.split_face(&[c, LonLat::new(170.0, 0.0), d])).unwrap();
    doc.topo.validate().unwrap();

    let names = ["东国", "西国", "中国"];
    let faces: Vec<_> = doc.topo.faces.keys().copied().filter(|f| *f != map_core::BACKGROUND).collect();
    assert_eq!(faces.len(), 3);
    for (f, n) in faces.iter().zip(names) {
        let e = doc.add_entity(EntityKind::Country, n);
        doc.assign(*f, Some(e)).unwrap();
    }

    // edit the shared border: the cut's interior vertex moves for both countries
    let (v, _) = doc.topo.nearest_vertex(LonLat::new(179.0, 0.0), 0.1).unwrap();
    let before: f64 = faces.iter().map(|f| doc.topo.face_area(*f, 6371.0)).sum();
    h.apply(&mut doc, "move", |x| x.topo.move_vertex(v, LonLat::new(-178.0, 2.0))).unwrap();
    let after: f64 = faces.iter().map(|f| doc.topo.face_area(*f, 6371.0)).sum();
    assert!((before - after).abs() / before < 1e-9, "total area must be conserved by border edits");
    doc.topo.validate().unwrap();
    let _ = b;

    // save → reopen
    let path = dir.join("world.atlas");
    map_io::project::save(&path, &doc).unwrap();
    let mut doc = map_io::project::load(&path).unwrap();
    doc.topo.validate().unwrap();

    // globe/2D geometry from the reopened document
    let cache = map_render_free_check(&doc);
    assert!(cache > 0);

    // statistics + derived indicator + choropleth
    let pop = doc.add_field("population", "人口", FieldKind::Number, None).unwrap();
    doc.add_field("density", "密度", FieldKind::Number, Some("population / area_km2".into())).unwrap();
    for (i, e) in doc.countries().into_iter().enumerate() {
        doc.set_value(e, pop, None, Some(Value::Num(1e6 * (i + 1) as f64)));
    }
    doc.theme.mode = ThemeMode::Choropleth;
    doc.theme.field_key = "density".into();
    let th = theme::evaluate(&doc);
    assert_eq!(th.entity_values.len(), 3);

    // exports
    let opts = ExportOptions { width: 1200, height: 650, ..Default::default() };
    let scene = build_scene(&doc, &th, &opts);
    let svg = map_io::svg::render(&scene);
    // (a name along a bent or spread line is written glyph by glyph)
    assert!(svg.contains("<path") && (svg.contains(">东国<") || (svg.contains(">东<") && svg.contains(">国<"))));
    let font = map_io::png::find_system_font();
    map_io::png::save(&scene, font.as_deref(), &dir.join("map.png")).unwrap();
    assert!(std::fs::metadata(dir.join("map.png")).unwrap().len() > 10_000);
    let gj = map_io::geojson::export_countries(&doc);
    assert!(gj.contains("\"density\""));
    let csv = map_io::table::export(&doc);
    assert_eq!(csv.lines().count(), 4);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Triangle count the renderer would upload (CPU part only, no GPU needed).
fn map_render_free_check(doc: &Document) -> usize {
    doc.topo.faces.keys().filter(|f| **f != map_core::BACKGROUND).map(|f| doc.topo.face_planar(*f).0.len()).sum()
}
