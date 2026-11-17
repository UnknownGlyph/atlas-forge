//! Compatibility samples: project files written by earlier versions, kept
//! in `tests/fixtures`, with a summary of what must be read from them.
//! Every later version has to read each file to the same world: the same
//! faces, entities, areas in every sampled year and layer, values with
//! their sources, features and layouts.
//!
//! A new sample is written once, by the version whose format it records:
//!   ATLAS_WRITE_FIXTURES=1 cargo test -p map_io --test compat
//! (existing samples are never rewritten: that would defeat them).

use map_core::doc::{Document, EntityKind, LayerKind, POLITICAL};
use map_core::stats::Stats;
use map_core::topo::{FaceKind, BACKGROUND};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const YEARS: [Option<i32>; 6] = [None, Some(1000), Some(1250), Some(1500), Some(1750), Some(2000)];

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures")
}

/// What a reader must get out of a world, whatever the file's format.
fn summary(doc: &Document) -> Value {
    let mut years = Map::new();
    for y in YEARS {
        let mut layers = Map::new();
        for l in doc.layer_ids() {
            let mut d = doc.clone();
            d.set_active_layer(l).unwrap();
            d.set_year(y);
            let areas = d.areas_by_entity();
            let mut by_code: Vec<(String, f64)> = areas.iter().map(|(e, a)| (d.entities[e].code.clone(), *a)).collect();
            by_code.sort_by(|a, b| a.0.cmp(&b.0));
            layers.insert(d.layer_name(l), json!(by_code));
        }
        let mut d = doc.clone();
        d.set_active_layer(POLITICAL).unwrap();
        d.set_year(y);
        let stats = Stats::new(&d);
        let land: usize = d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land).count();
        let mut values: Vec<Value> = Vec::new();
        for (id, x) in d.entities.iter().filter(|(_, x)| x.kind == EntityKind::Country && x.layer == POLITICAL).take(6) {
            for f in d.fields.values() {
                let st = stats.stat(*id, &f.key);
                values.push(json!([x.code, f.key, st.value, st.tag()]));
            }
        }
        let parents: Vec<Value> = d.entities.values().filter(|x| x.kind == EntityKind::City).map(|x| json!([x.code, x.parent.map(|p| d.entities[&p].code.clone())])).collect();
        let shown = (d.symbols.values().filter(|s| s.shown_in(y)).count(), d.areas.values().filter(|a| a.shown_in(y)).count());
        years.insert(y.map_or("latest".to_string(), |y| y.to_string()), json!({"areas": layers, "land_faces": land, "values": values, "city_parents": parents, "symbols_and_areas_shown": shown, "to_confirm": y.map_or(0, |y| d.lifespan_conflicts(y).len())}));
    }
    let entities: Vec<Value> = doc.entities.values().map(|x| json!([x.code, x.name, format!("{:?}", x.kind), x.founded, x.ended, x.successors.len(), x.layer.0])).collect();
    json!({
        "name": doc.meta.name,
        "radius": doc.meta.planet_radius_km,
        "faces": doc.topo.faces.len() - 1,
        "edges": doc.topo.edges.len(),
        "vertices": doc.topo.edges.values().map(|e| e.pts.len()).sum::<usize>() + doc.topo.nodes.len(),
        "layers": doc.layer_ids().iter().map(|l| json!([doc.layer_name(*l), doc.layer_kind(*l) == LayerKind::Shares])).collect::<Vec<_>>(),
        "change_years": doc.change_years(),
        "entities": entities,
        "fields": doc.fields.values().map(|f| json!([f.key, f.name, format!("{:?}", f.kind), f.expr, f.policy().name(), f.base_year])).collect::<Vec<_>>(),
        "features": {
            "lines": doc.rivers.values().map(|r| json!([r.name, r.pts.len(), r.route.is_some(), r.legend, r.show_name])).collect::<Vec<_>>(),
            "symbols": doc.symbols.values().map(|s| json!([s.kind.name(), s.name, s.from, s.to])).collect::<Vec<_>>(),
            "areas": doc.areas.values().map(|a| json!([a.name, a.ring.len(), a.from, a.to])).collect::<Vec<_>>(),
            "annotations": doc.annotations.values().map(|a| json!([a.text, a.at.lon, a.at.lat])).collect::<Vec<_>>(),
        },
        "layouts": doc.layouts.iter().map(|l| json!([l.name, l.width, l.height, l.projection.name(), l.title, l.bounds])).collect::<Vec<_>>(),
        "theme": [format!("{:?}", doc.theme.mode), doc.theme.field_key.clone(), format!("{:?}", doc.theme.scale), doc.theme.classes.to_string()],
        "years": years,
    })
}

/// Equal, with numbers compared to a relative 1e-9 (areas go through
/// trigonometry, which may differ in the last bits between platforms).
fn same(a: &Value, b: &Value, at: &str) -> Result<(), String> {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() <= 1e-9 * x.abs().max(y.abs()).max(1.0) {
                Ok(())
            } else {
                Err(format!("{at}: {x} became {y}"))
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => x.iter().zip(y).enumerate().try_for_each(|(i, (p, q))| same(p, q, &format!("{at}[{i}]"))),
        (Value::Object(x), Value::Object(y)) if x.len() == y.len() => x.iter().try_for_each(|(k, p)| same(p, y.get(k).ok_or(format!("{at}.{k} is missing"))?, &format!("{at}.{k}"))),
        _ if a == b => Ok(()),
        _ => Err(format!("{at}: {} became {}", a.to_string().chars().take(80).collect::<String>(), b.to_string().chars().take(80).collect::<String>())),
    }
}

/// The world of the version 1 sample: the test world (layers, a share
/// layer, lifespans and successors, cities, data, every kind of feature),
/// with a layout and a thematic setting added.
fn v1_world() -> Document {
    use map_core::testworld::{history_world, TestWorldSpec};
    let (mut d, _) = history_world(&TestWorldSpec { nx: 5, ny: 3, events: 60, ..Default::default() });
    d.save_layout(map_io::scene::ExportOptions { width: 1200, height: 700, title: "{year} 年".into(), scale_bar: true, ..Default::default() }.to_layout("全图")).unwrap();
    d.theme.mode = map_core::theme::ThemeMode::Choropleth;
    d.theme.scale = map_core::theme::ClassScale::AllYears;
    d
}

#[test]
fn samples_of_earlier_versions_read_the_same() {
    let dir = fixtures();
    if std::env::var_os("ATLAS_WRITE_FIXTURES").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        let (file, expected) = (dir.join("v1_world.atlas"), dir.join("v1_world.json"));
        if !file.exists() {
            let d = v1_world();
            map_io::project::save(&file, &d).unwrap();
            std::fs::write(&expected, serde_json::to_string_pretty(&summary(&d)).unwrap()).unwrap();
            eprintln!("wrote {}", file.display());
        }
    }
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).expect("tests/fixtures").flatten() {
        let file = entry.path();
        if file.extension().and_then(|e| e.to_str()) != Some("atlas") {
            continue;
        }
        seen += 1;
        let name = file.file_name().unwrap().to_string_lossy().to_string();
        let expected: Value = serde_json::from_str(&std::fs::read_to_string(file.with_extension("json")).unwrap()).unwrap();
        let doc = map_io::project::load(&file).unwrap_or_else(|e| panic!("{name} can no longer be read: {e}"));
        doc.topo.validate().unwrap_or_else(|e| panic!("{name}: {e}"));
        same(&expected, &summary(&doc), &name).unwrap();
        // saved by this version and read again: still the same world
        let again = map_io::project::from_bytes(&map_io::project::to_bytes(&doc).unwrap()).unwrap();
        same(&expected, &summary(&again), &format!("{name} (saved again)")).unwrap();
    }
    assert!(seen >= 1, "no samples in {}", dir.display());
}
