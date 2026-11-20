use map_core::{
    doc::Value,
    environment::{self, Field, Layer},
    playback::{study_years, Playback, Sampling},
    visualization::{Data, Plot, Project, Row, Rule, Spec, Years},
    world_book::{Link, Node, TimeLink, WorldBook, ROOT},
    worldview::{Catalog, Field as Property, Kind, Record, Schema, Series, Value as LoreValue},
    Document, EntityKind, FaceKind, FieldKind, LonLat,
};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

fn atlas() -> (Document, map_core::EntityId, u64, u32) {
    let mut d = Document::new("Kepler atlas", 6371.0);
    let a = d.add_entity(EntityKind::Country, "Port Arno");
    let b = d.add_entity(EntityKind::Country, "Lapis");
    let ring = |left| {
        [
            LonLat::new(left, -10.),
            LonLat::new(left + 16., -10.),
            LonLat::new(left + 16., 4.),
            LonLat::new(left, 4.),
        ]
    };
    let f = d.add_ring_face(&ring(2.), FaceKind::Land).unwrap();
    let other = d.add_ring_face(&ring(31.), FaceKind::Land).unwrap();
    d.assign(f, Some(a)).unwrap();
    d.assign(other, Some(b)).unwrap();
    let field = d
        .add_field("population", "Population", FieldKind::Number, None)
        .unwrap();
    d.set_value(a, field, Some(1890), Some(Value::Num(2.)));
    d.set_value(a, field, Some(1920), Some(Value::Num(11.)));
    d.set_value(b, field, Some(1930), Some(Value::Num(37.)));
    let focus = d
        .add_focus("Port coast", [0., -13., 23., 10.], Some((1870, 1960)))
        .unwrap();
    let c = Catalog::edit(&mut d);
    let schema = c.alloc().unwrap();
    c.schemas.insert(
        schema,
        Arc::new(Schema {
            name: "Human settlement".into(),
            fields: BTreeMap::from([(
                "impact".into(),
                Property {
                    name: "Habitat pressure".into(),
                    kind: Kind::Number,
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }),
    );
    let record = c.alloc().unwrap();
    c.records.insert(
        record,
        Arc::new(Record {
            schema,
            name: "Port Arno stewardship".into(),
            values: BTreeMap::from([(
                "impact".into(),
                Series {
                    base: Some(LoreValue::Number(0.10)),
                    history: BTreeMap::from([(1910, LoreValue::Number(0.9))]),
                    ..Default::default()
                },
            )]),
            ..Default::default()
        }),
    );
    let mut link = Link::new(Node::Record(record), Node::Entity(ROOT, a.0), false);
    link.time = TimeLink::Inherit;
    WorldBook::edit(&mut d).add_link(link).unwrap();
    (d, a, record, focus)
}

#[test]
fn time_and_environment_sources_update_visualized_data_and_map_exports() {
    let (mut d, country, record, focus) = atlas();
    assert_eq!(
        study_years(&d, d.focus(focus))
            .into_iter()
            .collect::<Vec<_>>(),
        vec![1890, 1910, 1920]
    );
    assert!(study_years(&d, None).contains(&1930));
    let mut generation = environment::Project::default();
    generation.settings.resolution = 64;
    generation.settings.bindings.push(environment::Binding {
        parameter: environment::Parameter::HabitatPressure,
        record,
        key: "impact".into(),
        scale: 1.0,
        offset: 0.0,
    });
    d.set_year(Some(1900));
    let before = environment::generate(&d, &generation).unwrap();
    d.set_year(Some(1920));
    let after = environment::generate(&d, &generation).unwrap();
    let layer = Layer::Standard(Field::Vegetation);
    let sample = LonLat::new(10., -5.);
    assert!(before.at(&layer, sample).unwrap() > after.at(&layer, sample).unwrap());
    generation.current = Some(Arc::new(after));
    d.environment = Some(Arc::new(generation));
    let mut project = Project::default();
    let id = project
        .insert(
            None,
            Spec {
                name: "Population pressure".into(),
                plot: Plot::Choropleth,
                data: Data::Property {
                    schema: None,
                    key: "impact".into(),
                },
                rule: Rule {
                    bounds: Some([0., 1.]),
                    ..Default::default()
                },
            },
        )
        .unwrap();
    project.overlay = Some(id);
    d.visualization = Some(Arc::new(project));
    let face = d.topo.face_at(sample);
    let newer = map_core::theme::evaluate(&d).face_colors[&face];
    d.set_year(Some(1900));
    let older = map_core::theme::evaluate(&d).face_colors[&face];
    assert_ne!(older, newer);
    let d = map_io::project::from_bytes(&map_io::project::to_bytes(&d).unwrap()).unwrap();
    assert_eq!(map_core::theme::evaluate(&d).face_colors[&face], older);
    let spec = Spec {
        name: "Population by year".into(),
        plot: Plot::Curve,
        data: Data::MapHistory {
            key: "population".into(),
            entity: country,
            years: Years {
                from: 1890,
                until: 1920,
                step: 30,
            },
        },
        rule: Rule {
            expression: "value * 1000".into(),
            ..Default::default()
        },
    };
    let graph = spec.evaluate(&d).unwrap();
    assert_eq!(
        graph.rows.iter().map(|r| r.value).collect::<Vec<_>>(),
        vec![2_000., 11_000.]
    );
    let mut icons = (*d.visualization.as_ref().unwrap().as_ref()).clone();
    let id = icons
        .insert(
            None,
            Spec {
                name: "Point data".into(),
                plot: Plot::Symbols,
                data: Data::Rows(vec![Row {
                    label: "Port".into(),
                    x: 1.0,
                    value: 15.0,
                    point: Some(sample),
                    node: None,
                }]),
                rule: Rule {
                    map_labels: true,
                    ..Default::default()
                },
            },
        )
        .unwrap();
    icons.overlay = Some(id);
    let mut d = d;
    d.visualization = Some(Arc::new(icons));
    let theme = map_core::theme::evaluate(&d);
    let scene = map_io::scene::build_scene(&d, &theme, &Default::default());
    assert_eq!(theme.data_icons.len(), 1);
    assert!(scene.labels.iter().any(|label| label.text == "Port"));
    if let Some(dir) = std::env::var_os("ATLAS_VERIFY_MEDIA_DIR").map(PathBuf::from) {
        std::fs::create_dir_all(&dir).unwrap();
        map_io::project::save(&dir.join("Kepler-QA.atlas"), &d).unwrap();
    }
}

#[test]
fn scheduled_h264_exports_cover_global_and_local_bounds() {
    let Some(dir) = std::env::var_os("ATLAS_VERIFY_MEDIA_DIR").map(PathBuf::from) else {
        return;
    };
    let (d, _, _, focus) = atlas();
    let schedule = Playback {
        from: 1880,
        to: 1940,
        sampling: Sampling::Every(20),
        duration: 1.6,
        calendar_pacing: false,
        speed: 1.0,
    }
    .schedule(&[])
    .unwrap();
    assert_eq!(schedule.iter().map(|s| s.hold_ms).sum::<u32>(), 1600);
    let opts = map_io::scene::ExportOptions {
        width: 320,
        height: 180,
        legend: false,
        labels: false,
        projection: map_core::projection::Projection::Equirectangular,
        ..Default::default()
    };
    let [a, b, c, e] = d.focus(focus).unwrap().bounds.map(f64::to_radians);
    let region = map_io::scene::ExportOptions {
        bounds: Some([a, b, c, e]),
        ..opts.clone()
    };
    std::fs::create_dir_all(&dir).unwrap();
    assert!(map_io::batch::ffmpeg_available());
    for (name, opts) in [("global", opts), ("region", region)] {
        let path = dir.join(format!("{name}.mp4"));
        let job = map_io::batch::AnimJob {
            years: vec![],
            path: path.clone(),
            kind: map_io::batch::Animation::Mp4,
            opts,
            font: None,
            frame_ms: 40,
        };
        let done = std::sync::atomic::AtomicUsize::new(0);
        let cancel = std::sync::atomic::AtomicBool::new(false);
        assert_eq!(
            map_io::batch::export_scheduled(&d, &job, &schedule, &done, &cancel).unwrap(),
            Some(path)
        );
        assert_eq!(
            done.load(std::sync::atomic::Ordering::Relaxed),
            schedule.len()
        );
    }
    assert_ne!(
        std::fs::read(dir.join("global.mp4")).unwrap(),
        std::fs::read(dir.join("region.mp4")).unwrap()
    );
}

#[test]
fn rules_read_environment_results_and_reject_feedback_into_generation() {
    use map_core::worldview::{Aggregate, Evaluation, Input, Rule as NumericRule};
    let (mut doc, _, record, _) = atlas();
    let mut project = environment::Project::default();
    project.settings.resolution = 64;
    project.current = Some(Arc::new(environment::generate(&doc, &project).unwrap()));
    doc.environment = Some(Arc::new(project.clone()));
    let catalog = Catalog::edit(&mut doc);
    let schema = catalog.records[&record].schema;
    Arc::make_mut(catalog.schemas.get_mut(&schema).unwrap())
        .fields
        .insert(
            "mean_wetness".into(),
            Property {
                kind: Kind::Number,
                name: "Wetness of the coast".into(),
                rule: Some(NumericRule {
                    expression: "local".into(),
                    inputs: BTreeMap::from([(
                        "local".into(),
                        Input::Environment {
                            layer: Layer::Standard(Field::Wetness),
                            aggregate: Aggregate::Mean,
                        },
                    )]),
                }),
                ..Default::default()
            },
        );
    let from_layer = Evaluation::new(&doc, None)
        .unwrap()
        .get(record, "mean_wetness")
        .unwrap()
        .unwrap()
        .value
        .as_number()
        .unwrap();
    assert!((0. ..=100.).contains(&from_layer));
    project.settings.bindings.push(environment::Binding {
        parameter: environment::Parameter::HabitatPressure,
        record,
        key: "mean_wetness".into(),
        scale: 0.01,
        offset: 0.0,
    });
    let error = environment::generate(&doc, &project).unwrap_err();
    assert!(error.contains("上游"));
}
