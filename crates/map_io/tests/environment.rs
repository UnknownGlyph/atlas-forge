use map_core::{
    climate::Region,
    environment::{self, Field, Layer, Project, Version},
    Document, LonLat,
};
use std::sync::Arc;

#[test]
fn local_environment_roundtrips_pins_and_exports_only_its_region() {
    let mut doc = Document::new("Island study", 6371.0);
    let mut project = Project::default();
    project.settings.resolution = 64;
    project.settings.region = Region {
        west: 170.0,
        east: -173.0,
        south: 2.0,
        north: 18.0,
    };
    project.settings.generate_unknown = false;
    let point = LonLat::new(177.5, 9.0);
    let result = environment::generate(&doc, &project).unwrap();
    assert!(result.grid.index(LonLat::new(-179.0, 5.0)).is_some());
    assert!(result.grid.index(LonLat::new(30.0, 5.0)).is_none());
    assert_eq!(
        result.at(&Layer::Standard(Field::Elevation), point),
        Some(0.0)
    );
    project.current = Some(Arc::new(result));
    project
        .edit_at(point, Layer::Standard(Field::Elevation), Some(3000.0), true)
        .unwrap();
    project
        .edit_at(point, Layer::Standard(Field::Rain), Some(1400.0), false)
        .unwrap();
    let result = environment::generate(&doc, &project).unwrap();
    let i = result.grid.index(point).unwrap();
    assert_eq!(
        result.value(&Layer::Standard(Field::Elevation), i),
        Some(3000.0)
    );
    assert_eq!(result.value(&Layer::Standard(Field::Rain), i), Some(1400.0));
    assert!(result.value(&Layer::Standard(Field::Slope), i).unwrap() > 0.0);
    let area = result.suitability.len();
    assert!(area >= 3);
    let result = Arc::new(result);
    project.saved.push(Version {
        name: "Recorded regional study".into(),
        result: result.clone(),
    });
    project.current = Some(result);
    doc.environment = Some(Arc::new(project));

    let doc = map_io::project::from_bytes(&map_io::project::to_bytes(&doc).unwrap()).unwrap();
    let project = doc.environment.as_ref().unwrap();
    assert_eq!(project.saved.len(), 1);
    assert_eq!(project.edits.values().filter(|e| e.pinned).count(), 1);
    let out = map_io::scene::build_scene(
        &doc,
        &map_core::theme::evaluate(&doc),
        &map_io::scene::ExportOptions {
            width: 640,
            height: 360,
            projection: map_core::projection::Projection::Equirectangular,
            lon0: 179.0,
            ..Default::default()
        },
    );
    let overlay = out.shade.unwrap();
    let drawn = overlay
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[3] > 0)
        .count();
    assert!(
        drawn > 50 && drawn < 640 * 360 / 10,
        "{drawn} pixels covered"
    );
}

#[test]
fn global_drainage_is_acyclic_and_geographic_inputs_change_results() {
    let mut doc = Document::new("World", 6371.0);
    let mut project = Project::default();
    project.settings.resolution = 64;
    let first = environment::generate(&doc, &project).unwrap();
    let mut lake_routes = 0;
    for origin in 0..first.grid.len() {
        let mut remaining = first.grid.len();
        let mut i = origin;
        while let Some(next) = first.downstream[i] {
            assert!(remaining > 0, "a path recirculated from cell {origin}");
            let runoff = |cell| first.value(&Layer::Standard(Field::Runoff), cell).unwrap();
            assert!(runoff(i) <= runoff(next as usize) + 0.01);
            i = next as usize;
            remaining -= 1;
            lake_routes += 1;
        }
    }
    assert!(lake_routes > 0);
    project.settings.rainfall = 2.0;
    doc.set_year(Some(1810));
    let second = environment::generate(&doc, &project).unwrap();
    assert_eq!(second.year, Some(1810));
    let rain_at =
        |g: &environment::Generation, i| g.value(&Layer::Standard(Field::Rain), i).unwrap();
    for i in 0..first.grid.len() {
        assert!((rain_at(&second, i) - rain_at(&first, i) * 2.0).abs() < 0.02);
    }
}
