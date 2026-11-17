//! The read-only world queries. Each works on a private copy of the world
//! set to the request's context (see [`Session::view`]).

use crate::{ApiError, ApiResult, Context, Session};
use map_core::doc::{Document, EntityId, EntityKind, FieldKind, LayerKind, AREA_KEY};
use map_core::lifespan::Presence;
use map_core::stats::Stats;
use map_core::topo::{FaceKind, BACKGROUND};
use map_core::LonLat;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap};

fn kind_name(k: EntityKind) -> &'static str {
    match k {
        EntityKind::Country => "country",
        EntityKind::Region => "region",
        EntityKind::City => "city",
    }
}

fn presence_name(p: Presence) -> &'static str {
    match p {
        Presence::Exists => "exists",
        Presence::NotYet => "not_yet",
        Presence::Ended => "ended",
    }
}

/// `{id, name}` of an entity, or null.
fn ent(d: &Document, e: Option<EntityId>) -> Value {
    match e.and_then(|e| d.entities.get(&e).map(|x| (e, x))) {
        Some((e, x)) => json!({"id": e.0, "name": x.name}),
        None => Value::Null,
    }
}

fn round(v: f64) -> f64 {
    // (+ 0.0 turns -0.0 into 0.0)
    (v * 1000.0).round() / 1000.0 + 0.0
}

fn arg_i32(args: &Value, k: &str) -> Result<Option<i32>, ApiError> {
    match args.get(k) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_i64().map(|x| Some(x as i32)).ok_or_else(|| ApiError::BadRequest(format!("{k} 必须是整数"))),
    }
}

fn arg_f64(args: &Value, k: &str) -> Result<f64, ApiError> {
    args.get(k).and_then(Value::as_f64).ok_or_else(|| ApiError::BadRequest(format!("缺少数值参数 {k}")))
}

/// The entity a request names by `id`, `code` or `name`.
pub(crate) fn find_entity(d: &Document, args: &Value) -> Result<EntityId, ApiError> {
    if let Some(id) = args.get("id").and_then(Value::as_u64) {
        let e = EntityId(id);
        return d.entities.contains_key(&e).then_some(e).ok_or_else(|| ApiError::NotFound(format!("没有 id 为 {id} 的实体")));
    }
    if let Some(code) = args.get("code").and_then(Value::as_str) {
        return d.entity_by_code(code).ok_or_else(|| ApiError::NotFound(format!("没有编号为“{code}”的实体")));
    }
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        let found: Vec<EntityId> = d.entities.iter().filter(|(_, x)| x.name == name).map(|(id, _)| *id).collect();
        return match found[..] {
            [e] => Ok(e),
            [] => Err(ApiError::NotFound(format!("没有名为“{name}”的实体"))),
            _ => Err(ApiError::BadRequest(format!("有 {} 个实体名为“{name}”，请改用 id 或 code", found.len()))),
        };
    }
    Err(ApiError::BadRequest("需要 id、code 或 name".into()))
}

pub fn summary(s: &Session, ctx: &Context) -> ApiResult {
    let d = s.view(ctx);
    let doc = s.doc();
    let years = doc.change_years();
    let mut by_kind: BTreeMap<&str, usize> = BTreeMap::new();
    for x in doc.entities.values() {
        *by_kind.entry(kind_name(x.kind)).or_default() += 1;
    }
    let verts: usize = doc.topo.edges.values().map(|e| e.pts.len()).sum::<usize>() + doc.topo.nodes.len();
    let layers: Vec<Value> = doc
        .layer_ids()
        .into_iter()
        .map(|l| {
            let units = doc.entities.values().filter(|x| x.layer == l && x.kind != EntityKind::City).count();
            json!({"id": l.0, "name": doc.layer_name(l), "kind": if doc.layer_kind(l) == LayerKind::Shares { "shares" } else { "exclusive" }, "units": units})
        })
        .collect();
    let fields: Vec<Value> = doc
        .fields
        .values()
        .map(|f| json!({"key": f.key, "name": f.name, "kind": format!("{:?}", f.kind).to_lowercase(), "unit": f.unit, "derived": f.expr, "inherit": f.policy().name(), "base_year": f.base_year}))
        .collect();
    // data years: the records' own years
    let data_years: std::collections::BTreeSet<i32> = doc.values.values().flat_map(|v| v.by_year.keys().copied()).collect();
    Ok(json!({
        "name": doc.meta.name,
        "description": doc.meta.description,
        "planet_radius_km": doc.meta.planet_radius_km,
        "source": s.source,
        "counts": {
            "faces": doc.topo.faces.len() - 1,
            "land_faces_in_year": d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land).count(),
            "edges": doc.topo.edges.len(),
            "vertices": verts,
            "entities": by_kind,
            "rivers": doc.rivers.values().filter(|r| r.route.is_none()).count(),
            "routes": doc.rivers.values().filter(|r| r.route.is_some()).count(),
            "symbols": doc.symbols.len(),
            "areas": doc.areas.len(),
            "annotations": doc.annotations.len(),
            "layouts": doc.layouts.len(),
        },
        "layers": layers,
        "years": {
            "change_years": years.len(),
            "first_change": years.first(),
            "last_change": years.last(),
            "data_years": {"count": data_years.len(), "first": data_years.first(), "last": data_years.last()},
        },
        "fields": fields,
        "theme": {"mode": format!("{:?}", doc.theme.mode).to_lowercase(), "field": doc.theme.field_key, "scale": format!("{:?}", doc.theme.scale)},
        "layouts": doc.layouts.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
    }))
}

pub fn timeline(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let d = s.view(&Context { year: None, layer: ctx.layer });
    let (from, to) = (arg_i32(args, "from")?, arg_i32(args, "to")?);
    let mut years: Vec<Value> = Vec::new();
    for y in d.change_years() {
        if from.is_some_and(|f| y < f) || to.is_some_and(|t| y > t) {
            continue;
        }
        let owner = d.owner_changes.values().filter(|c| c.contains_key(&y)).count();
        let kind = d.kind_changes.values().filter(|c| c.contains_key(&y)).count();
        let founded: Vec<Value> = d.entities.iter().filter(|(_, x)| x.founded == Some(y)).map(|(id, _)| ent(&d, Some(*id))).collect();
        let ended: Vec<Value> = d.entities.iter().filter(|(_, x)| x.ended == Some(y - 1)).map(|(id, _)| ent(&d, Some(*id))).collect();
        let parents = d.entities.values().filter(|x| x.parent_changes.contains_key(&y)).count();
        years.push(json!({"year": y, "faces_changing_owner": owner, "faces_changing_land_water": kind, "founded": founded, "ended_the_year_before": ended, "parent_changes": parents}));
    }
    Ok(json!({"layer": d.layer_name(ctx.layer), "years": years}))
}

pub fn entities(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let d = s.view(ctx);
    let kind = args.get("kind").and_then(Value::as_str);
    if kind.is_some_and(|k| !["country", "region", "city"].contains(&k)) {
        return Err(ApiError::BadRequest("kind 只能是 country、region 或 city".into()));
    }
    let existing = args.get("existing").and_then(Value::as_bool).unwrap_or(true);
    let name = args.get("name").and_then(Value::as_str).unwrap_or("");
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(500) as usize;
    let areas = d.areas_by_entity();
    let mut items = Vec::new();
    let mut count = 0;
    for (id, x) in d.entities.iter() {
        if x.layer != ctx.layer || kind.is_some_and(|k| k != kind_name(x.kind)) || !x.name.contains(name) {
            continue;
        }
        let p = x.presence(ctx.year);
        if existing && p != Presence::Exists {
            continue;
        }
        count += 1;
        if items.len() >= limit {
            continue;
        }
        let mut o = json!({
            "id": id.0,
            "code": x.code,
            "name": x.name,
            "kind": kind_name(x.kind),
            "presence": presence_name(p),
            "lifespan": {"founded": x.founded, "ended": x.ended},
            "parent": ent(&d, x.parent),
        });
        if x.kind == EntityKind::City {
            o["position"] = json!(x.pos.map(|p| [p.lon, p.lat]));
        } else {
            o["area_km2"] = json!(round(areas.get(id).copied().unwrap_or(0.0)));
        }
        if !x.successors.is_empty() {
            o["successors"] = json!(x.successors.iter().map(|e| ent(&d, Some(*e))).collect::<Vec<_>>());
        }
        items.push(o);
    }
    Ok(json!({"count": count, "returned": items.len(), "items": items}))
}

/// The value of every field for `e` in the view's year, with its source.
fn values_of(d: &Document, stats: &Stats, e: EntityId, keys: Option<&[String]>) -> Map<String, Value> {
    let mut out = Map::new();
    for (fid, f) in d.fields.iter() {
        if keys.is_some_and(|k| !k.contains(&f.key)) {
            continue;
        }
        let v = if f.kind == FieldKind::Number || f.expr.is_some() {
            let st = stats.stat(e, &f.key);
            json!({"value": st.value, "source": st.tag(), "note": st.label()})
        } else {
            let (v, src) = d.value_with_source(e, *fid);
            json!({"value": v.map(|v| v.display()), "source": src.tag(), "note": src.label()})
        };
        out.insert(f.key.clone(), v);
    }
    out
}

pub fn entity(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let d = s.view(ctx);
    let e = find_entity(&d, args)?;
    let x = d.entities[&e].clone();
    let stats = Stats::new(&d);
    let members: Vec<Value> = d.entities.iter().filter(|(_, m)| m.parent == Some(e)).map(|(id, _)| ent(&d, Some(*id))).collect();
    // area from each year its territory changes (and from its founding)
    let mut years: Vec<i32> = d.entity_change_years(e).into_iter().collect();
    let truncated = years.len() > 400;
    years.truncate(400);
    let mut series = Vec::new();
    if x.kind != EntityKind::City {
        let mut v = s.doc().clone();
        let _ = v.set_active_layer(ctx.layer);
        for y in &years {
            v.set_year(Some(*y));
            series.push(json!({"from_year": y, "area_km2": round(v.areas_by_entity().get(&e).copied().unwrap_or(0.0))}));
        }
    }
    let area = stats.areas().get(&e).copied();
    // its land's extent [west, south, east, north] (longitudes may run past ±180)
    let bounds = {
        let pieces = d.dissolve(e);
        (!pieces.is_empty()).then(|| {
            let b = pieces.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, p| {
                let q = map_core::geom::bbox(&p.outer);
                [a[0].min(q[0]), a[1].min(q[1]), a[2].max(q[2]), a[3].max(q[3])]
            });
            b.map(|v| (v * 100.0).round() / 100.0)
        })
    };
    Ok(json!({
        "id": e.0,
        "code": x.code,
        "name": x.name,
        "short_name": x.short,
        "kind": kind_name(x.kind),
        "layer": {"id": x.layer.0, "name": d.layer_name(x.layer)},
        "note": x.note,
        "lifespan": {"founded": x.founded, "ended": x.ended, "text": x.lifespan_text()},
        "predecessors": d.predecessors(e).into_iter().map(|p| ent(&d, Some(p))).collect::<Vec<_>>(),
        "successors": x.successors.iter().map(|p| ent(&d, Some(*p))).collect::<Vec<_>>(),
        "in_context_year": {
            "presence": presence_name(x.presence(ctx.year)),
            "area_km2": area.map(round),
            "pieces_of_land": d.faces_of(e).len(),
            "bounds": bounds,
            "parent": ent(&d, x.parent),
            "members": members,
            "capital": ent(&d, x.capital),
            "position": x.pos.map(|p| [p.lon, p.lat]),
            "values": values_of(&d, &stats, e, None),
        },
        "history": {"change_years": years, "truncated": truncated, "area_from_year": series},
    }))
}

pub fn place(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let (lon, lat) = (arg_f64(args, "lon")?, arg_f64(args, "lat")?);
    if !(-90.0..=90.0).contains(&lat) {
        return Err(ApiError::BadRequest("lat 必须在 -90 到 90 之间".into()));
    }
    let p = LonLat::new(lon, lat).normalized();
    let d = s.view(ctx);
    let f = d.topo.face_at(p);
    let kind = d.topo.faces.get(&f).map(|x| x.kind).unwrap_or(FaceKind::Water);
    let mut layers = Vec::new();
    for l in d.layer_ids() {
        let v = s.view(&Context { year: ctx.year, layer: l });
        let mut o = json!({"id": l.0, "name": v.layer_name(l), "owner": ent(&v, v.owner.get(&f).copied())});
        if v.layer_kind(l) == LayerKind::Shares {
            o["shares"] = json!(v
                .shares_shown(f)
                .iter()
                .map(|(e, bp)| json!({"unit": ent(&v, Some(*e)), "percent": *bp as f64 / 100.0}))
                .collect::<Vec<_>>());
        }
        layers.push(o);
    }
    Ok(json!({"lon": p.lon, "lat": p.lat, "face": f.0, "land": kind == FaceKind::Land, "layers": layers}))
}

pub fn changes(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let from = arg_i32(args, "from")?.ok_or_else(|| ApiError::BadRequest("缺少 from".into()))?;
    let to = arg_i32(args, "to")?.ok_or_else(|| ApiError::BadRequest("缺少 to".into()))?;
    let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize;
    let a = s.view(&Context { year: Some(from), layer: ctx.layer });
    let b = s.view(&Context { year: Some(to), layer: ctx.layer });
    let r = a.radius();
    let mut area: HashMap<Option<EntityId>, (f64, f64)> = HashMap::new();
    let mut transfers: HashMap<(Option<EntityId>, Option<EntityId>), f64> = HashMap::new();
    let (mut to_water, mut to_land) = (0.0, 0.0);
    for (f, fa) in a.topo.faces.iter() {
        if *f == BACKGROUND {
            continue;
        }
        let fb = &b.topo.faces[f];
        let km2 = a.topo.face_area(*f, r);
        let (la, lb) = (fa.kind == FaceKind::Land, fb.kind == FaceKind::Land);
        let (oa, ob) = (a.owner.get(f).copied().filter(|_| la), b.owner.get(f).copied().filter(|_| lb));
        if la {
            area.entry(oa).or_default().0 += km2;
        }
        if lb {
            area.entry(ob).or_default().1 += km2;
        }
        match (la, lb) {
            (true, false) => to_water += km2,
            (false, true) => to_land += km2,
            (true, true) if oa != ob => *transfers.entry((oa, ob)).or_default() += km2,
            _ => {}
        }
    }
    let mut per: Vec<(Option<EntityId>, f64, f64)> = area.into_iter().map(|(e, (x, y))| (e, x, y)).filter(|(_, x, y)| (x - y).abs() > 1e-6).collect();
    per.sort_by(|p, q| (q.2 - q.1).abs().total_cmp(&(p.2 - p.1).abs()));
    let mut tr: Vec<((Option<EntityId>, Option<EntityId>), f64)> = transfers.into_iter().collect();
    tr.sort_by(|p, q| q.1.total_cmp(&p.1));
    let total_transfers = tr.len();
    let label = |e: Option<EntityId>| if e.is_none() { json!({"id": null, "name": "（无主）"}) } else { ent(&a, e) };
    Ok(json!({
        "from": from,
        "to": to,
        "layer": a.layer_name(ctx.layer),
        "area_changes": per.iter().map(|(e, x, y)| json!({"entity": label(*e), "area_from_km2": round(*x), "area_to_km2": round(*y), "change_km2": round(y - x)})).collect::<Vec<_>>(),
        "transfers": tr.iter().take(limit).map(|((x, y), km2)| json!({"from": label(*x), "to": label(*y), "area_km2": round(*km2)})).collect::<Vec<_>>(),
        "transfers_total": total_transfers,
        "land_became_water_km2": round(to_water),
        "water_became_land_km2": round(to_land),
    }))
}

pub fn stats(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let d = s.view(ctx);
    let keys: Option<Vec<String>> = match args.get("fields") {
        None | Some(Value::Null) => None,
        Some(Value::Array(a)) => Some(a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()),
        _ => return Err(ApiError::BadRequest("fields 必须是字段键的数组".into())),
    };
    if let Some(k) = &keys {
        if let Some(bad) = k.iter().find(|k| *k != AREA_KEY && d.field_by_key(k).is_none()) {
            return Err(ApiError::NotFound(format!("没有字段“{bad}”")));
        }
    }
    let stats = Stats::new(&d);
    let rows: Vec<Value> = d
        .entities
        .iter()
        .filter(|(_, x)| x.kind != EntityKind::City && x.layer == ctx.layer && x.exists_in(ctx.year))
        .map(|(id, x)| {
            json!({
                "id": id.0,
                "code": x.code,
                "name": x.name,
                "area_km2": round(stats.areas().get(id).copied().unwrap_or(0.0)),
                "values": values_of(&d, &stats, *id, keys.as_deref()),
            })
        })
        .collect();
    let fields: Vec<Value> = d
        .fields
        .values()
        .filter(|f| keys.as_ref().is_none_or(|k| k.contains(&f.key)))
        .map(|f| json!({"key": f.key, "name": f.name, "unit": f.unit, "derived": f.expr}))
        .collect();
    Ok(json!({"fields": fields, "rows": rows}))
}

pub fn features(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let d = s.doc();
    let kind = args.get("kind").and_then(Value::as_str);
    let want = |k: &str| kind.is_none_or(|x| x == k);
    if kind.is_some_and(|k| !["river", "route", "symbol", "area", "annotation"].contains(&k)) {
        return Err(ApiError::BadRequest("kind 只能是 river、route、symbol、area 或 annotation".into()));
    }
    let r = d.radius();
    let length = |pts: &[LonLat]| pts.windows(2).map(|w| map_core::measure::great_circle(w[0], w[1], r)).sum::<f64>();
    // the layer of drawn features it is in, by name (null: in none)
    let layer = |id: map_core::doc::LineId| d.group_of_id(id).and_then(|g| d.group(g)).map(|g| g.name.clone());
    // the named style it follows, by name (null: its own look)
    let style = |id: map_core::doc::LineId| d.style_of.get(&id).and_then(|s| d.style(*s)).map(|s| s.name.clone());
    let mut items = Vec::new();
    for (id, l) in d.rivers.iter() {
        let k = if l.route.is_some() { "route" } else { "river" };
        if want(k) {
            items.push(json!({"id": id.0, "kind": k, "name": l.name, "legend": l.legend, "points": l.pts.len(), "length_km": round(length(&l.pts)), "shown": !l.hidden && !d.group_hides(*id), "layer": layer(*id), "style": style(*id)}));
        }
    }
    if want("symbol") {
        for (id, x) in d.symbols.iter() {
            items.push(json!({"id": id.0, "kind": "symbol", "symbol": x.kind.name(), "name": x.name, "legend": x.legend, "position": [x.at.lon, x.at.lat], "years": {"from": x.from, "to": x.to}, "shown": x.shown_in(ctx.year) && !d.group_hides(*id), "layer": layer(*id), "style": style(*id)}));
        }
    }
    if want("area") {
        for (id, a) in d.areas.iter() {
            items.push(json!({"id": id.0, "kind": "area", "name": a.name, "legend": a.legend, "rings": a.rings().len(), "years": {"from": a.from, "to": a.to}, "shown": a.shown_in(ctx.year) && !d.group_hides(*id), "layer": layer(*id), "style": style(*id)}));
        }
    }
    if want("annotation") {
        for (id, a) in d.annotations.iter() {
            items.push(json!({"id": id.0, "kind": "annotation", "text": a.text, "position": [a.at.lon, a.at.lat], "shown": !a.hidden && !d.group_hides(*id), "layer": layer(*id), "style": style(*id)}));
        }
    }
    Ok(json!({"count": items.len(), "items": items}))
}

pub fn checks(s: &Session, ctx: &Context) -> ApiResult {
    let d = s.view(ctx);
    let topology = match d.topo.validate() {
        Ok(()) => json!("ok"),
        Err(e) => json!(e),
    };
    let conflicts: Vec<Value> = match ctx.year {
        Some(y) => d
            .lifespan_conflicts(y)
            .iter()
            .map(|c| json!({"entity": ent(&d, Some(c.entity)), "presence": presence_name(c.presence), "faces": c.faces.len(), "members": c.members.iter().map(|m| ent(&d, Some(*m))).collect::<Vec<_>>()}))
            .collect(),
        None => vec![],
    };
    let r = d.radius();
    let unowned: Vec<_> = d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land && !d.owner.contains_key(*f)).map(|(f, _)| *f).collect();
    let stats = Stats::new(&d);
    let existing: Vec<EntityId> = d.entities.iter().filter(|(_, x)| x.kind != EntityKind::City && x.layer == ctx.layer && x.exists_in(ctx.year)).map(|(id, _)| *id).collect();
    let missing: Vec<Value> = d
        .fields
        .values()
        .filter_map(|f| {
            let n = existing.iter().filter(|e| if f.kind == FieldKind::Number || f.expr.is_some() { stats.value(**e, &f.key).is_none() } else { d.raw_value(**e, d.field_by_key(&f.key).unwrap()).is_none() }).count();
            (n > 0).then(|| json!({"field": f.key, "entities_without_value": n, "of": existing.len()}))
        })
        .collect();
    Ok(json!({
        "topology": topology,
        "lifespan_conflicts": conflicts,
        "unowned_land": {"pieces": unowned.len(), "area_km2": round(unowned.iter().map(|f| d.topo.face_area(*f, r)).sum())},
        "missing_values": missing,
    }))
}
