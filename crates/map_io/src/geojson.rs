//! GeoJSON (RFC 7946) export and a simple import.
//!
//! Export splits geometry at the antimeridian as RFC 7946 recommends.
//! Import (P0) adds each polygon as an independent island/lake; building
//! shared-border topology from adjacent polygons is planned for P1.

use crate::scene::{split_polyline, split_strip};
use map_core::doc::{Document, EntityKind, PlanarPolygon};
use map_core::geom::{self, P2};
use map_core::topo::{FaceKind, BACKGROUND};
use map_core::LonLat;
use serde_json::{json, Map, Value};

fn polygon_coords(poly: &PlanarPolygon) -> Vec<Value> {
    let mut rings = vec![poly.outer.clone()];
    rings.extend(poly.holes.iter().cloned());
    split_strip(&rings)
        .into_iter()
        .map(|part| {
            Value::Array(
                part.into_iter()
                    .map(|mut r: Vec<P2>| {
                        r.push(r[0]);
                        Value::Array(r.iter().map(|p| json!([round(p[0]), round(p[1])])).collect())
                    })
                    .collect(),
            )
        })
        .collect()
}

fn round(v: f64) -> f64 {
    (v * 1e7).round() / 1e7
}

fn geometry(parts: Vec<Value>) -> Value {
    if parts.len() == 1 {
        json!({"type": "Polygon", "coordinates": parts[0]})
    } else {
        json!({"type": "MultiPolygon", "coordinates": parts})
    }
}

/// The background face as a polygon: the whole lon/lat rectangle minus the
/// top-level rings.
fn background_polygon(doc: &Document) -> PlanarPolygon {
    let (_, holes) = doc.topo.face_planar(BACKGROUND);
    PlanarPolygon { outer: vec![[-180.0, -90.0], [180.0, -90.0], [180.0, 90.0], [-180.0, 90.0]], holes }
}

/// One feature per country (dissolved), with all numeric/text attributes.
pub fn export_countries(doc: &Document) -> String {
    let stats = map_core::stats::Stats::new(doc);
    let areas = stats.areas().clone();
    let mut feats = Vec::new();
    for id in doc.countries() {
        let mut parts: Vec<Value> = doc.dissolve(id).iter().flat_map(polygon_coords).collect();
        if doc.owner.get(&BACKGROUND) == Some(&id) {
            parts.extend(polygon_coords(&background_polygon(doc)));
        }
        if parts.is_empty() {
            continue;
        }
        let e = &doc.entities[&id];
        let mut props = Map::new();
        props.insert("code".into(), json!(e.code));
        props.insert("name".into(), json!(e.name));
        props.insert("color".into(), json!(format!("#{:02x}{:02x}{:02x}", e.color[0], e.color[1], e.color[2])));
        props.insert("area_km2".into(), json!(areas.get(&id).copied().unwrap_or(0.0)));
        for (fid, f) in doc.fields.iter() {
            let v = if f.expr.is_some() {
                stats.value(id, &f.key).map(|x| json!(x))
            } else {
                doc.raw_value(id, *fid).map(|v| match v {
                    map_core::Value::Num(x) => json!(x),
                    map_core::Value::Bool(b) => json!(b),
                    map_core::Value::Text(t) => json!(t),
                })
            };
            props.insert(f.key.clone(), v.unwrap_or(Value::Null));
            // with a year selected, where each value comes from
            if doc.year.is_some() {
                let tag = if f.expr.is_some() { stats.stat(id, &f.key).tag() } else { doc.source_of(id, &f.key).tag() };
                props.insert(format!("{}_source", f.key), json!(tag));
            }
        }
        feats.push(json!({"type": "Feature", "properties": props, "geometry": geometry(parts)}));
    }
    serde_json::to_string(&json!({"type": "FeatureCollection", "features": feats})).unwrap()
}

/// One feature per topological face (land and water), with its owner.
pub fn export_faces(doc: &Document) -> String {
    let mut feats = Vec::new();
    for (fid, f) in doc.topo.faces.iter() {
        // a sea background is "everything else"; a land one is real territory
        if *fid == BACKGROUND && f.kind == FaceKind::Water {
            continue;
        }
        let poly = if *fid == BACKGROUND {
            background_polygon(doc)
        } else {
            let (outer, holes) = doc.topo.face_planar(*fid);
            PlanarPolygon { outer, holes }
        };
        let parts = polygon_coords(&poly);
        let owner = doc.owner.get(fid).and_then(|e| doc.entities.get(e));
        feats.push(json!({
            "type": "Feature",
            "properties": {
                "face_id": fid.0,
                "kind": if f.kind == FaceKind::Land { "land" } else { "water" },
                "owner_code": owner.map(|e| e.code.clone()),
                "owner_name": owner.map(|e| e.name.clone()),
            },
            "geometry": geometry(parts),
        }));
    }
    serde_json::to_string(&json!({"type": "FeatureCollection", "features": feats})).unwrap()
}

/// One feature per river or route: a LineString, or a MultiLineString when
/// it crosses the antimeridian, with its name, kind and length.
pub fn export_rivers(doc: &Document) -> String {
    let mut feats = Vec::new();
    for (id, r) in doc.rivers.iter() {
        if r.pts.len() < 2 {
            continue;
        }
        let parts: Vec<Value> = split_polyline(&geom::unwrap_path(&r.pts, None))
            .into_iter()
            .filter(|p| p.len() >= 2)
            .map(|p| Value::Array(p.iter().map(|q| json!([round(q[0]), round(q[1])])).collect()))
            .collect();
        let geometry = match parts.len() {
            0 => continue,
            1 => json!({"type": "LineString", "coordinates": parts[0]}),
            _ => json!({"type": "MultiLineString", "coordinates": parts}),
        };
        feats.push(json!({
            "type": "Feature",
            "properties": {"name": r.name, "kind": if r.route.is_some() { "route" } else { "river" }, "length_km": (doc.river_length_km(*id) * 10.0).round() / 10.0},
            "geometry": geometry,
        }));
    }
    serde_json::to_string(&json!({"type": "FeatureCollection", "features": feats})).unwrap()
}

/// Line features as rivers (or routes, for `"kind": "route"`, in the default
/// route style): LineString / MultiLineString, with the pieces of a line
/// split at the antimeridian joined back into one.
fn rivers_from(feats: &[Value]) -> Vec<map_core::doc::LineFeature> {
    let mut out = Vec::new();
    for (i, f) in feats.iter().enumerate() {
        let g = &f["geometry"];
        let lines: Vec<&Value> = match g["type"].as_str() {
            Some("LineString") => vec![&g["coordinates"]],
            Some("MultiLineString") => g["coordinates"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
            _ => continue,
        };
        let mut parts: Vec<Vec<LonLat>> = lines.into_iter().filter_map(ring_points).filter(|p| p.len() >= 2).collect();
        // join a piece ending on the antimeridian to the one starting there
        let seam = |a: LonLat, b: LonLat| a.lon.abs() >= 179.9999999 && b.lon.abs() >= 179.9999999 && (a.lat - b.lat).abs() < 1e-7;
        'join: loop {
            for x in 0..parts.len() {
                for y in 0..parts.len() {
                    if x != y && seam(*parts[x].last().unwrap(), parts[y][0]) {
                        let tail = parts.remove(y);
                        let x = if y < x { x - 1 } else { x };
                        parts[x].extend_from_slice(&tail[1..]);
                        continue 'join;
                    }
                }
            }
            break;
        }
        let route = (f["properties"]["kind"].as_str() == Some("route")).then(map_core::doc::RouteStyle::default);
        let fallback = if route.is_some() { "线路" } else { "河流" };
        let name = NAME_KEYS.iter().find_map(|k| f["properties"][*k].as_str()).map(str::to_string).unwrap_or_else(|| format!("{fallback}{}", i + 1));
        let n = parts.len();
        for (k, pts) in parts.into_iter().enumerate() {
            let name = if n > 1 { format!("{name}（{}）", k + 1) } else { name.clone() };
            out.push(map_core::doc::LineFeature { name, pts, route, legend: String::new(), show_name: false, hidden: false });
        }
    }
    out
}

#[derive(Debug, Default)]
pub struct ImportReport {
    pub faces_added: usize,
    pub countries_added: usize,
    pub rivers_added: usize,
    pub failed: usize,
    pub messages: Vec<String>,
}

fn ring_points(v: &Value) -> Option<Vec<LonLat>> {
    let mut pts: Vec<LonLat> = v.as_array()?.iter().filter_map(|p| Some(LonLat::new(p[0].as_f64()?, p[1].as_f64()?))).collect();
    if pts.len() > 1 && geom::geo_dist_deg(pts[0], *pts.last().unwrap()) < 1e-12 {
        pts.pop();
    }
    Some(pts)
}

pub fn import(doc: &mut Document, text: &str) -> Result<ImportReport, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("GeoJSON 解析失败: {e}"))?;
    let features: Vec<Value> = match root["type"].as_str() {
        Some("FeatureCollection") => root["features"].as_array().cloned().unwrap_or_default(),
        Some("Feature") => vec![root.clone()],
        _ => vec![json!({"type": "Feature", "properties": {}, "geometry": root})],
    };
    let mut rep = ImportReport::default();
    for r in rivers_from(&features) {
        doc.rivers.insert(map_core::doc::LineId(doc.topo.alloc()), r);
        rep.rivers_added += 1;
    }
    for (i, f) in features.iter().enumerate() {
        let g = &f["geometry"];
        let polys: Vec<&Value> = match g["type"].as_str() {
            Some("Polygon") => vec![&g["coordinates"]],
            Some("MultiPolygon") => g["coordinates"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
            _ => continue,
        };
        let props = &f["properties"];
        let name = ["name", "NAME", "名称", "name_zh", "ADMIN"].iter().find_map(|k| props[*k].as_str()).map(str::to_string);
        let mut owner = None;
        for poly in polys {
            let Some(rings) = poly.as_array() else { continue };
            let Some(outer) = rings.first().and_then(ring_points) else { continue };
            match doc.add_ring_face(&outer, FaceKind::Land) {
                Ok(face) => {
                    rep.faces_added += 1;
                    if let Some(n) = &name {
                        let e = *owner.get_or_insert_with(|| {
                            rep.countries_added += 1;
                            doc.add_entity(EntityKind::Country, n)
                        });
                        let _ = doc.assign(face, Some(e));
                    }
                    for hole in &rings[1..] {
                        if let Some(h) = ring_points(hole) {
                            if doc.add_ring_face(&h, FaceKind::Water).is_ok() {
                                rep.faces_added += 1;
                            }
                        }
                    }
                }
                Err(e) => {
                    rep.failed += 1;
                    if rep.messages.len() < 20 {
                        rep.messages.push(format!("要素 {i}{}: {e}", name.as_ref().map(|n| format!(" ({n})")).unwrap_or_default()));
                    }
                }
            }
        }
    }
    Ok(rep)
}

/// Result of a shared-border (topology) import.
#[derive(Debug, Default)]
pub struct TopologyImport {
    /// Present when the import ran in repair mode.
    pub repair: Option<map_core::repair::RepairReport>,
    /// Display name of every input feature (by feature index).
    pub feature_names: Vec<String>,
    pub countries: usize,
    pub faces: usize,
    pub edges: usize,
    pub lakes: usize,
    pub rivers: usize,
    pub fields: usize,
    pub skipped_rings: usize,
    pub problems: Vec<String>,
}

fn features(text: &str) -> Result<Vec<Value>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| format!("GeoJSON 解析失败: {e}"))?;
    Ok(match root["type"].as_str() {
        Some("FeatureCollection") => root["features"].as_array().cloned().unwrap_or_default(),
        Some("Feature") => vec![root],
        _ => vec![json!({"type": "Feature", "properties": {}, "geometry": root})],
    })
}

const NAME_KEYS: [&str; 6] = ["name_zh", "NAME_ZH", "name", "NAME", "名称", "ADMIN"];
const CODE_KEYS: [&str; 6] = ["code", "ISO_A3", "iso_a3", "ADM0_A3", "id", "ID"];

/// Import polygons as a new world whose neighbouring countries share
/// borders (strict mode: boundaries must coincide coordinate for
/// coordinate; overlaps are reported, gaps become water).
/// How to treat imperfect data.
#[derive(Clone, Debug)]
pub enum ImportMode {
    /// Borders must coincide exactly; any inconsistency is reported.
    Strict,
    /// Snap, node, resolve overlaps and fill slivers, reporting every change.
    Repair(map_core::repair::RepairOptions),
}

pub fn import_topology(text: &str) -> Result<(Document, TopologyImport), TopologyImport> {
    import_topology_with(text, &ImportMode::Strict)
}

pub fn import_topology_with(text: &str, mode: &ImportMode) -> Result<(Document, TopologyImport), TopologyImport> {
    let mut rep = TopologyImport::default();
    let feats = features(text).map_err(|e| TopologyImport { problems: vec![e], ..Default::default() })?;
    let names: Vec<String> = feats
        .iter()
        .enumerate()
        .map(|(i, f)| NAME_KEYS.iter().find_map(|k| f["properties"][*k].as_str()).map(str::to_string).unwrap_or_else(|| format!("要素{}", i + 1)))
        .collect();
    rep.feature_names = names.clone();
    let mut polys = Vec::new();
    let mut owner_feature = Vec::new();
    for (i, f) in feats.iter().enumerate() {
        let g = &f["geometry"];
        let parts: Vec<&Value> = match g["type"].as_str() {
            Some("Polygon") => vec![&g["coordinates"]],
            Some("MultiPolygon") => g["coordinates"].as_array().map(|a| a.iter().collect()).unwrap_or_default(),
            _ => continue,
        };
        for part in parts {
            let Some(rings) = part.as_array() else { continue };
            let rings: Vec<Vec<LonLat>> = rings.iter().filter_map(ring_points).collect();
            if !rings.is_empty() {
                polys.push(map_core::build::SourcePolygon { rings });
                owner_feature.push(i);
            }
        }
    }
    // (feature, face) pairs from either pipeline
    let (topo, assigned, brep) = match mode {
        ImportMode::Strict => {
            let (topo, faces, brep) = map_core::build::build(&polys).map_err(|b| TopologyImport { problems: b.problems, skipped_rings: b.skipped_rings, ..Default::default() })?;
            let assigned: Vec<(usize, map_core::FaceId)> = faces.iter().enumerate().filter_map(|(pi, f)| f.map(|f| (owner_feature[pi], f))).collect();
            (topo, assigned, brep)
        }
        ImportMode::Repair(opts) => {
            let out = map_core::repair::repair(&polys, &owner_feature, opts)
                .map_err(|r| TopologyImport { problems: r.problems.clone(), repair: Some(r), feature_names: names.clone(), ..Default::default() })?;
            let mut assigned: Vec<(usize, map_core::FaceId)> = out.faces.iter().flat_map(|(fi, fs)| fs.iter().map(move |f| (*fi, *f))).collect();
            assigned.sort();
            let brep = out.report.build.clone();
            rep.repair = Some(out.report);
            (out.topo, assigned, brep)
        }
    };
    let mut doc = Document::default();
    doc.meta.name = "导入的世界".into();
    doc.topo = topo;
    // one country per feature, owning all of its polygons
    let mut entity_of: std::collections::HashMap<usize, map_core::EntityId> = std::collections::HashMap::new();
    for (fi, face) in assigned.iter().map(|(fi, f)| (*fi, Some(f))) {
        let Some(face) = face else { continue };
        let e = *entity_of.entry(fi).or_insert_with(|| {
            let props = &feats[fi]["properties"];
            let name = NAME_KEYS.iter().find_map(|k| props[*k].as_str()).map(str::to_string).unwrap_or_else(|| format!("要素{}", fi + 1));
            let id = doc.add_entity(EntityKind::Country, &name);
            if let Some(code) = CODE_KEYS.iter().find_map(|k| props[*k].as_str().map(str::to_string).or_else(|| props[*k].as_i64().map(|v| v.to_string()))) {
                if !code.is_empty() && code != "-99" && doc.entity_by_code(&code).is_none() {
                    doc.entities.get_mut(&id).unwrap().code = code;
                }
            }
            id
        });
        let _ = doc.assign(*face, Some(e));
    }
    // feature properties become fields (numbers and text)
    for (fi, e) in &entity_of {
        let Some(props) = feats[*fi]["properties"].as_object() else { continue };
        for (k, v) in props {
            if NAME_KEYS.contains(&k.as_str()) {
                continue;
            }
            let (kind, value) = match v {
                Value::Number(n) => (map_core::FieldKind::Number, n.as_f64().map(map_core::Value::Num)),
                Value::String(t) if !t.is_empty() => (map_core::FieldKind::Text, Some(map_core::Value::Text(t.clone()))),
                Value::Bool(b) => (map_core::FieldKind::Bool, Some(map_core::Value::Bool(*b))),
                _ => continue,
            };
            let key: String = k.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
            if key.is_empty() || key == map_core::doc::AREA_KEY {
                continue;
            }
            let fid = match doc.field_by_key(&key) {
                Some(f) => f,
                None => match doc.add_field(&key, k, kind, None) {
                    Ok(f) => {
                        rep.fields += 1;
                        f
                    }
                    Err(_) => continue,
                },
            };
            doc.set_value(*e, fid, None, value);
        }
    }
    for r in rivers_from(&feats) {
        doc.rivers.insert(map_core::doc::LineId(doc.topo.alloc()), r);
        rep.rivers += 1;
    }
    rep.countries = entity_of.len();
    rep.faces = brep.faces;
    rep.edges = brep.edges;
    rep.lakes = brep.lakes;
    rep.skipped_rings = brep.skipped_rings;
    Ok((doc, rep))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn land_background_is_exported_as_territory() {
        let mut d = Document::default();
        let x = d.add_entity(EntityKind::Country, "X");
        let sea = d.add_ring_face(&[LonLat::new(10.0, 10.0), LonLat::new(30.0, 10.0), LonLat::new(30.0, 20.0), LonLat::new(10.0, 20.0)], FaceKind::Water).unwrap();
        // a sea background is not a feature
        let v: Value = serde_json::from_str(&export_faces(&d)).unwrap();
        assert_eq!(v["features"].as_array().unwrap().len(), 1);
        d.set_background_kind(FaceKind::Land);
        d.assign(BACKGROUND, Some(x)).unwrap();
        let v: Value = serde_json::from_str(&export_faces(&d)).unwrap();
        let feats = v["features"].as_array().unwrap();
        assert_eq!(feats.len(), 2);
        let bg = feats.iter().find(|f| f["properties"]["face_id"] == json!(0)).unwrap();
        assert_eq!(bg["properties"]["kind"], json!("land"));
        // world rectangle with the sea as a hole
        assert_eq!(bg["geometry"]["coordinates"].as_array().unwrap().len(), 2);
        let c: Value = serde_json::from_str(&export_countries(&d)).unwrap();
        assert_eq!(c["features"][0]["properties"]["name"], json!("X"));
        assert!((c["features"][0]["properties"]["area_km2"].as_f64().unwrap() - (4.0 * std::f64::consts::PI * d.radius().powi(2) - d.topo.face_area(sea, d.radius()))).abs() < 1.0);
    }
    #[test]
    fn rivers_export_and_come_back() {
        let mut d = Document::default();
        let id = map_core::doc::LineId(d.topo.alloc());
        // one river crossing the antimeridian, one plain
        d.rivers.insert(id, map_core::doc::LineFeature { name: "跨线河".into(), pts: vec![LonLat::new(170.0, 0.0), LonLat::new(178.0, 2.0), LonLat::new(-176.0, 4.0), LonLat::new(-170.0, 5.0)], route: None, legend: String::new(), show_name: false, hidden: false });
        let id2 = map_core::doc::LineId(d.topo.alloc());
        d.rivers.insert(id2, map_core::doc::LineFeature { name: "小河".into(), pts: vec![LonLat::new(0.0, 0.0), LonLat::new(5.0, 5.0)], route: None, legend: String::new(), show_name: false, hidden: false });
        let text = export_rivers(&d);
        let v: Value = serde_json::from_str(&text).unwrap();
        let feats = v["features"].as_array().unwrap();
        assert_eq!(feats.len(), 2);
        let cross = feats.iter().find(|f| f["properties"]["name"] == json!("跨线河")).unwrap();
        assert_eq!(cross["geometry"]["type"], json!("MultiLineString"));
        for part in cross["geometry"]["coordinates"].as_array().unwrap() {
            for p in part.as_array().unwrap() {
                assert!(p[0].as_f64().unwrap().abs() <= 180.0);
            }
        }
        let small = feats.iter().find(|f| f["properties"]["name"] == json!("小河")).unwrap();
        assert_eq!(small["geometry"]["type"], json!("LineString"));
        assert!((small["properties"]["length_km"].as_f64().unwrap() - d.river_length_km(id2)).abs() < 0.1);
        // back in: the split river is one line again, same length
        let mut e = Document::default();
        let rep = import(&mut e, &text).unwrap();
        assert_eq!(rep.rivers_added, 2);
        let back = e.rivers.iter().find(|(_, r)| r.name == "跨线河").map(|(id, _)| *id).unwrap();
        assert_eq!(e.rivers[&back].pts.len(), 5, "the seam point joins the two pieces");
        assert!((e.river_length_km(back) - d.river_length_km(id)).abs() < 1.0);
        assert!(e.rivers.values().all(|r| r.route.is_none()));
        // a route goes out marked as one and comes back as one
        let mut d2 = d.clone();
        let rid = map_core::doc::LineId(d2.topo.alloc());
        d2.rivers.insert(rid, map_core::doc::LineFeature { name: "商路".into(), pts: vec![LonLat::new(10.0, 10.0), LonLat::new(20.0, 12.0)], route: Some(Default::default()), legend: String::new(), show_name: false, hidden: false });
        let text2 = export_rivers(&d2);
        assert!(text2.contains("\"kind\":\"route\""));
        let mut e2 = Document::default();
        import(&mut e2, &text2).unwrap();
        let r = e2.rivers.values().find(|r| r.name == "商路").unwrap();
        assert_eq!(r.route, Some(map_core::doc::RouteStyle::default()));
        assert_eq!(e2.rivers.values().filter(|r| r.route.is_none()).count(), 2);
        // rivers ride along in a shared-border import too
        let mut fc: Value = serde_json::from_str(&export_faces(&map_core::sample::grid_world(2, 1, 4, 1))).unwrap();
        fc["features"].as_array_mut().unwrap().extend(feats.iter().cloned());
        let (w, rep) = import_topology(&fc.to_string()).unwrap_or_else(|r| panic!("{:?}", r.problems));
        assert_eq!((rep.rivers, w.rivers.len()), (2, 2));
    }

    #[test]
    fn export_import_roundtrip() {
        let mut d = Document::default();
        let ring = [LonLat::new(170.0, -10.0), LonLat::new(-170.0, -10.0), LonLat::new(-170.0, 10.0), LonLat::new(170.0, 10.0)];
        let f = d.add_ring_face(&ring, FaceKind::Land).unwrap();
        let c = d.add_entity(EntityKind::Country, "跨日界线国");
        d.assign(f, Some(c)).unwrap();
        let text = export_countries(&d);
        let v: Value = serde_json::from_str(&text).unwrap();
        let g = &v["features"][0]["geometry"];
        assert_eq!(g["type"], "MultiPolygon"); // split at the antimeridian
        for part in g["coordinates"].as_array().unwrap() {
            for p in part[0].as_array().unwrap() {
                assert!(p[0].as_f64().unwrap().abs() <= 180.0);
            }
        }
        // the same countries re-imported as a shared-border world
        let faces_json = export_faces(&map_core::sample::grid_world(5, 3, 4, 1));
        let (w, rep) = import_topology(&faces_json).unwrap_or_else(|r| panic!("{:?}", r.problems));
        assert_eq!(rep.countries, 15);
        w.topo.validate().unwrap();
        assert!(w.topo.edges.values().filter(|e| e.left != BACKGROUND && e.right != BACKGROUND).count() > 15);
        // export → shared-border re-import of a world with a seam-crossing,
        // multi-country island (this used to fail: the exporter folded
        // polygons along the antimeridian)
        let demo = map_core::sample::demo_world();
        let (w, rep) = import_topology(&export_countries(&demo)).unwrap_or_else(|r| panic!("{:?}", r.problems));
        assert_eq!(rep.countries, demo.countries().len());
        w.topo.validate().unwrap();
        let area = |d: &Document| d.areas_by_entity().values().sum::<f64>();
        assert!((area(&w) - area(&demo)).abs() / area(&demo) < 1e-6, "{} vs {}", area(&w), area(&demo));
        // import a simple polygon with a hole into a new doc
        let src = r#"{"type":"FeatureCollection","features":[{"type":"Feature","properties":{"name":"A"},
          "geometry":{"type":"Polygon","coordinates":[[[0,0],[10,0],[10,10],[0,10],[0,0]],[[2,2],[2,4],[4,4],[4,2],[2,2]]]}}]}"#;
        let mut d2 = Document::default();
        let rep = import(&mut d2, src).unwrap();
        assert_eq!(rep.faces_added, 2);
        assert_eq!(rep.countries_added, 1);
        d2.topo.validate().unwrap();
    }
}
