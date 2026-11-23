//! Edits on a draft (stage C of the AI-native plan): write operations do
//! not touch the world the session was opened with; they change a draft
//! copy of it, which every later read sees, until the draft is looked at
//! (`draft.changes`, `preview.render` with `before`) and either applied by
//! the application (one undo step) or dropped (`draft.discard`).
//!
//! Every edit says the year it takes effect from (the request's context;
//! none: every year, which a map with history refuses for land that has
//! one) and is checked: a topology that would no longer hold refuses the
//! edit and leaves the draft as it was. Each accepted edit is logged.

use crate::queries::find_entity;
use crate::{ApiError, ApiResult, Context, Session};
use map_core::doc::{Document, EntityId, EntityKind, FieldKind, LineFeature, LineId, Value as FieldValue};
use map_core::geom::LonLat;
use serde_json::{json, Value};

fn s_arg<'a>(args: &'a Value, k: &str) -> Result<&'a str, ApiError> {
    args.get(k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()).ok_or_else(|| ApiError::BadRequest(format!("缺少文字参数 {k}")))
}

/// An entity named by an argument that is an id, a name, or {id|code|name}.
fn entity_arg(d: &Document, args: &Value, k: &str) -> Result<EntityId, ApiError> {
    match args.get(k) {
        Some(Value::Number(n)) => find_entity(d, &json!({"id": n})),
        // a name, or an id written as text (models given one type per field)
        Some(Value::String(s)) => match s.trim().parse::<u64>() {
            Ok(n) if d.entities.contains_key(&EntityId(n)) => Ok(EntityId(n)),
            _ => find_entity(d, &json!({"name": s.trim()})),
        },
        Some(v @ Value::Object(_)) => find_entity(d, v),
        _ => Err(ApiError::BadRequest(format!("缺少参数 {k}（实体 id、名称或 {{id|code|name}}）"))),
    }
}

fn colour(s: &str) -> Result<[u8; 4], ApiError> {
    let h = s.trim().trim_start_matches('#');
    let v = u32::from_str_radix(h, 16).ok().filter(|_| h.len() == 6).ok_or_else(|| ApiError::BadRequest(format!("颜色“{s}”应为 #rrggbb")))?;
    Ok([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255])
}

fn points(v: &Value, k: &str) -> Result<Vec<LonLat>, ApiError> {
    let arr = v.get(k).and_then(Value::as_array).ok_or_else(|| ApiError::BadRequest(format!("缺少点列 {k}：[[经度, 纬度], …]")))?;
    arr.iter()
        .map(|p| match p.as_array().map(|a| (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64))) {
            Some((Some(lon), Some(lat))) if lat.abs() <= 90.0 => Ok(LonLat::new(lon, lat)),
            _ => Err(ApiError::BadRequest(format!("{k} 中的点应为 [经度, 纬度]"))),
        })
        .collect()
}

/// The area of a transfer: a ring (`polygon`), a box (`bounds`), or the side
/// of a meridian or parallel (`east_of`, `west_of`, `north_of`, `south_of`)
/// over the giver's land.
fn area(d: &Document, from: EntityId, args: &Value) -> Result<Vec<LonLat>, ApiError> {
    if args.get("polygon").is_some() {
        return points(args, "polygon");
    }
    if let Some(b) = args.get("bounds").and_then(Value::as_array) {
        let b: Vec<f64> = b.iter().filter_map(Value::as_f64).collect();
        if b.len() != 4 || b[2] <= b[0] || b[3] <= b[1] {
            return Err(ApiError::BadRequest("bounds 应为 [西, 南, 东, 北]".into()));
        }
        return Ok(vec![LonLat::new(b[0], b[1]), LonLat::new(b[2], b[1]), LonLat::new(b[2], b[3]), LonLat::new(b[0], b[3])]);
    }
    // the giver's land's extent, to reach past it
    let pieces = d.dissolve(from);
    let bb = pieces.iter().fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, p| {
        let b = map_core::geom::bbox(&p.outer);
        [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
    });
    if pieces.is_empty() {
        return Err(ApiError::BadRequest("给出的单位在这一年没有领土".into()));
    }
    let (w, s, e, n) = (bb[0] - 1.0, (bb[1] - 1.0).max(-89.9), bb[2] + 1.0, (bb[3] + 1.0).min(89.9));
    let rect = |w: f64, s: f64, e: f64, n: f64| vec![LonLat::new(w, s), LonLat::new(e, s), LonLat::new(e, n), LonLat::new(w, n)];
    let f = |k: &str| args.get(k).and_then(Value::as_f64);
    match (f("east_of"), f("west_of"), f("north_of"), f("south_of")) {
        (Some(x), _, _, _) => Ok(rect(x, s, e.max(x + 0.1), n)),
        (_, Some(x), _, _) => Ok(rect(w.min(x - 0.1), s, x, n)),
        (_, _, Some(y), _) => Ok(rect(w, y, e, n.max(y + 0.1))),
        (_, _, _, Some(y)) => Ok(rect(w, s.min(y - 0.1), e, y)),
        _ => Err(ApiError::BadRequest("需要 polygon、bounds 或 east_of / west_of / north_of / south_of 之一".into())),
    }
}

fn area_of(d: &Document, e: EntityId) -> f64 {
    d.areas_by_entity().get(&e).copied().unwrap_or(0.0)
}

fn year_arg(args: &Value, key: &str) -> Result<Option<i32>, ApiError> {
    let Some(value) = args.get(key).filter(|value| !value.is_null()) else { return Ok(None) };
    value.as_i64().and_then(|year| i32::try_from(year).ok()).map(Some)
        .ok_or_else(|| ApiError::BadRequest(format!("{key} 须为年份整数或 null")))
}

fn bool_arg(args: &Value, key: &str) -> Result<Option<bool>, ApiError> {
    args.get(key).map(|value| {
        value.as_bool().ok_or_else(|| ApiError::BadRequest(format!("{key} 须为布尔值")))
    }).transpose()
}

fn set_visibility(doc: &mut Document, args: &Value) -> ApiResult {
    let labels = bool_arg(args, "labels")?;
    let cities = bool_arg(args, "cities")?;
    let visible = bool_arg(args, "visible")?;
    let group = if let Some(group) = args.get("group") {
        let mut matches = doc.groups.iter().filter(|entry| {
            group.as_u64() == Some(entry.id.0) || group.as_str() == Some(entry.name.as_str())
        });
        let id = matches.next().ok_or_else(|| ApiError::NotFound("没有匹配的要素图层".into()))?.id;
        if matches.next().is_some() {
            return Err(ApiError::BadRequest("图层名称重复，请使用图层 ID".into()));
        }
        let visible = visible.ok_or_else(|| ApiError::BadRequest("指定 group 时需要 visible".into()))?;
        doc.set_group_hidden(id, !visible).map_err(ApiError::Failed)?;
        Some(json!({"id": id.0, "visible": visible}))
    } else {
        if visible.is_some() || (labels.is_none() && cities.is_none()) {
            return Err(ApiError::BadRequest("提交 labels、cities，或提交 group 和 visible".into()));
        }
        None
    };
    if let Some(show) = labels { doc.style.show_labels = show; }
    if let Some(show) = cities { doc.style.show_cities = show; }
    Ok(json!({"labels": doc.style.show_labels, "cities": doc.style.show_cities, "group": group}))
}

fn field_value(kind: FieldKind, value: &Value) -> Result<Option<FieldValue>, ApiError> {
    if value.is_null() { return Ok(None); }
    let typed = match kind {
        FieldKind::Number => value.as_f64().filter(|x| x.is_finite()).map(FieldValue::Num),
        FieldKind::Text | FieldKind::Category => value.as_str().map(|s| FieldValue::Text(s.to_owned())),
        FieldKind::Bool => value.as_bool().map(FieldValue::Bool),
    };
    typed.map(Some).ok_or_else(|| ApiError::BadRequest(format!("value 与字段类型 {kind:?} 不符")))
}

impl Session {
    /// The draft (made from the world on the first edit).
    fn draft_mut(&mut self) -> &mut Document {
        self.draft.get_or_insert_with(|| self.doc.clone())
    }

    /// Answer a request that may edit: `edit.*` and `draft.*` change or
    /// report on the draft; anything else is answered as by [`Session::handle`].
    pub fn handle_mut(&mut self, req: &Value) -> Value {
        let op = req.get("op").and_then(Value::as_str).unwrap_or("");
        if !(op.starts_with("edit.") || op.starts_with("draft.")) {
            return self.handle(req);
        }
        let id = req.get("id").cloned().unwrap_or(Value::Null);
        let args = req.get("args").cloned().unwrap_or(json!({}));
        let out = if args.is_object() {
            self.context(req.get("context")).and_then(|ctx| self.edit(op, &args, &ctx))
        } else {
            Err(ApiError::BadRequest("args 必须是对象".into()))
        };
        match out {
            Ok(result) => json!({"id": id, "ok": true, "version": self.version, "result": result}),
            Err(e) => json!({"id": id, "ok": false, "version": self.version, "error": {"kind": e.kind(), "message": e.message()}}),
        }
    }

    fn edit(&mut self, op: &str, args: &Value, ctx: &Context) -> ApiResult {
        match op {
            "draft.changes" => return Ok(self.changes(ctx)),
            "draft.discard" => {
                let had = self.draft.take().is_some();
                self.log.clear();
                return Ok(json!({"discarded": had}));
            }
            "draft.save" => {
                let path = std::path::PathBuf::from(s_arg(args, "path")?);
                if path.extension().and_then(|e| e.to_str()) != Some("atlas") {
                    return Err(ApiError::BadRequest("path 须以 .atlas 结尾".into()));
                }
                // never over the world it came from
                let same = std::fs::canonicalize(&path).ok().zip(std::fs::canonicalize(&self.source).ok()).is_some_and(|(a, b)| a == b);
                if same || path.as_os_str() == self.source.as_str() {
                    return Err(ApiError::BadRequest("不能覆盖原来的世界；请另选文件名".into()));
                }
                let d = self.draft.as_ref().ok_or_else(|| ApiError::BadRequest("还没有草稿".into()))?;
                map_io::project::save(&path, d).map_err(ApiError::Failed)?;
                return Ok(json!({"saved": path.display().to_string(), "edits": self.log}));
            }
            _ => {}
        }
        // worked on a copy set to the request's year and layer, kept only when it holds
        let (year0, layer0) = (self.doc.year, self.doc.active_layer);
        // (the draft is made only by an edit that holds)
        let mut d = self.draft.clone().unwrap_or_else(|| self.doc.clone());
        d.set_layer_and_year(ctx.layer, ctx.year).map_err(ApiError::BadRequest)?;
        let when = ctx.year.map_or("所有年份".to_string(), |y| format!("{y} 年起"));
        let (result, note) = match op {
            "edit.transfer" => {
                let (from, to) = (entity_arg(&d, args, "from")?, entity_arg(&d, args, "to")?);
                let ring = area(&d, from, args)?;
                let before = area_of(&d, from);
                let moved = d.transfer_land(from, to, &ring).map_err(ApiError::Failed)?;
                let names = (d.entities[&from].name.clone(), d.entities[&to].name.clone());
                (json!({"moved_km2": moved.round(), "from_before_km2": before.round(), "from_after_km2": area_of(&d, from).round()}), format!("{when}，{} 的 {:.0} km² 划给 {}", names.0, moved, names.1))
            }
            "edit.rename" => {
                let e = entity_arg(&d, args, "entity")?;
                let name = s_arg(args, "name")?.trim().to_string();
                let old = std::mem::replace(&mut d.entities.get_mut(&e).unwrap().name, name.clone());
                (json!({"entity": e.0, "name": name}), format!("{old} 改名为 {name}"))
            }
            "edit.set_color" => {
                let e = entity_arg(&d, args, "entity")?;
                let c = colour(s_arg(args, "color")?)?;
                d.entities.get_mut(&e).unwrap().color = c;
                (json!({"entity": e.0}), format!("{} 换了颜色", d.entities[&e].name))
            }
            "edit.create" => {
                let name = s_arg(args, "name")?.trim().to_string();
                let kind = match args.get("kind").and_then(Value::as_str).unwrap_or("country") {
                    "country" => EntityKind::Country,
                    "region" => EntityKind::Region,
                    "city" => EntityKind::City,
                    k => return Err(ApiError::BadRequest(format!("kind 应为 country、region 或 city，不是“{k}”"))),
                };
                let e = if kind == EntityKind::City {
                    let (lon, lat) = (args.get("lon").and_then(Value::as_f64), args.get("lat").and_then(Value::as_f64));
                    let (Some(lon), Some(lat)) = (lon, lat) else { return Err(ApiError::BadRequest("城市需要 lon 与 lat".into())) };
                    d.add_city(LonLat::new(lon, lat), &name)
                } else {
                    d.add_entity(kind, &name)
                };
                if let Some(c) = args.get("color").and_then(Value::as_str) {
                    d.entities.get_mut(&e).unwrap().color = colour(c)?;
                }
                if args.get("parent").is_some_and(|p| !p.is_null()) {
                    let p = entity_arg(&d, args, "parent")?;
                    d.record_parent(e, Some(p)).map_err(ApiError::Failed)?;
                }
                (json!({"entity": e.0, "name": name}), format!("新建 {name}"))
            }
            "edit.set_lifespan" => {
                let e = entity_arg(&d, args, "entity")?;
                let (founded, ended) = (year_arg(args, "founded")?, year_arg(args, "ended")?);
                d.set_lifespan(e, founded, ended).map_err(ApiError::Failed)?;
                (json!({"entity": e.0}), format!("{} 的存续期改为 {founded:?}–{ended:?}", d.entities[&e].name))
            }
            "edit.set_parent" => {
                let e = entity_arg(&d, args, "entity")?;
                let p = if args.get("parent").is_some_and(|p| !p.is_null()) { Some(entity_arg(&d, args, "parent")?) } else { None };
                d.record_parent(e, p).map_err(ApiError::Failed)?;
                let pn = p.map_or("（顶层）".to_string(), |p| d.entities[&p].name.clone());
                (json!({"entity": e.0}), format!("{when}，{} 归于 {pn}", d.entities[&e].name))
            }
            "edit.partition" => {
                let e = entity_arg(&d, args, "entity")?;
                let spec = map_core::partition::PartitionSpec {
                    units: args.get("units").and_then(Value::as_u64).unwrap_or(3).clamp(2, 40) as usize,
                    provinces: args.get("provinces").and_then(Value::as_bool).unwrap_or(false),
                    seed: args.get("seed").and_then(Value::as_u64).unwrap_or(1),
                    ..Default::default()
                };
                let plan = d.partition_plan(e, &spec).map_err(ApiError::Failed)?;
                let units = d.apply_partition(&plan, &spec).map_err(ApiError::Failed)?;
                let names: Vec<String> = units.iter().map(|u| d.entities[u].name.clone()).collect();
                (json!({"units": units.iter().map(|u| json!({"id": u.0, "name": d.entities[u].name})).collect::<Vec<_>>()}), format!("{when}，自动划分为 {}", names.join("、")))
            }
            "edit.add_event" => {
                let name = s_arg(args, "name")?;
                // (a tool call's `year` arrives as the request's context)
                let year = year_arg(args, "year")?.or(ctx.year).ok_or_else(|| ApiError::BadRequest("缺少 year".into()))?;
                let end = year_arg(args, "end")?;
                let id = d.add_event(name, year, end);
                if let Some(note) = args.get("note").and_then(Value::as_str) {
                    if let Some(mut ev) = d.event(id).cloned() {
                        ev.note = note.to_string();
                        d.update_event(ev).map_err(ApiError::Failed)?;
                    }
                }
                (json!({"event": id}), format!("事件“{name}”（{year}）"))
            }
            "edit.add_route" => {
                let pts = points(args, "points")?;
                if pts.len() < 2 {
                    return Err(ApiError::BadRequest("线路至少需要两个点".into()));
                }
                let name = args.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let legend = args.get("legend").and_then(Value::as_str).unwrap_or("").to_string();
                let id = LineId(d.topo.alloc());
                d.rivers.insert(id, LineFeature { name: name.clone(), pts, route: Some(Default::default()), legend, show_name: !name.is_empty(), hidden: false });
                (json!({"route": id.0}), format!("线路“{name}”"))
            }
            "edit.draw_land" => {
                let ring = points(args, "polygon")?;
                // new land in the sea (and unclaimed land) for a country, or unclaimed
                let (intent, who) = match args.get("country").filter(|c| !c.is_null()) {
                    Some(_) => {
                        let e = entity_arg(&d, args, "country")?;
                        (map_core::overlay::DrawIntent::Expand(e), d.entities[&e].name.clone())
                    }
                    None => (map_core::overlay::DrawIntent::AddLand, "无主".to_string()),
                };
                let (next, rep) = map_core::overlay::apply(&d, &ring, intent).map_err(ApiError::Failed)?;
                d = next;
                let km2 = rep.added_km2 + rep.taken.iter().map(|t| t.1).sum::<f64>();
                (json!({"added_km2": rep.added_km2.round(), "claimed_km2": km2.round()}), format!("{when}，画出陆地 {:.0} km²（{who}）", km2))
            }
            "edit.draw_lake" => {
                let ring = points(args, "polygon")?;
                let (next, rep) = map_core::overlay::apply(&d, &ring, map_core::overlay::DrawIntent::CarveWater).map_err(ApiError::Failed)?;
                d = next;
                (json!({"water_km2": rep.carved_km2.round()}), format!("{when}，挖出水域 {:.0} km²", rep.carved_km2))
            }
            "edit.add_river" => {
                let pts = points(args, "points")?;
                if pts.len() < 2 {
                    return Err(ApiError::BadRequest("河流至少需要两个点".into()));
                }
                let name = args.get("name").and_then(Value::as_str).unwrap_or("").to_string();
                let id = LineId(d.topo.alloc());
                d.rivers.insert(id, LineFeature { name: name.clone(), pts, route: None, legend: String::new(), show_name: !name.is_empty(), hidden: false });
                (json!({"river": id.0}), format!("河流“{name}”"))
            }
            "edit.set_visibility" => (set_visibility(&mut d, args)?, "修改地图显示".into()),
            "edit.add_field" => {
                let key = s_arg(args, "key")?;
                let name = s_arg(args, "name")?;
                let kind = match s_arg(args, "kind")? {
                    "number" => FieldKind::Number,
                    "text" => FieldKind::Text,
                    "bool" => FieldKind::Bool,
                    "category" => FieldKind::Category,
                    _ => return Err(ApiError::BadRequest("kind 可选 number、text、bool 或 category".into())),
                };
                let id = d.add_field(key, name, kind, None).map_err(ApiError::BadRequest)?;
                (json!({"field": id.0, "key": key}), format!("新增字段“{name}”"))
            }
            "edit.set_value" => {
                let entity = entity_arg(&d, args, "entity")?;
                let key = s_arg(args, "key")?;
                let id = d.field_by_key(key).ok_or_else(|| ApiError::NotFound(format!("字段“{key}”不存在")))?;
                let field = &d.fields[&id];
                if field.expr.is_some() {
                    return Err(ApiError::BadRequest("计算字段的值来自公式".into()));
                }
                let value = args.get("value").ok_or_else(|| ApiError::BadRequest("缺少 value；用 null 删除记录".into()))?;
                let value = field_value(field.kind, value)?;
                d.set_value(entity, id, ctx.year, value);
                (json!({"entity": entity.0, "key": key, "year": ctx.year}), format!("更新字段“{key}” · {when}"))
            }
            "edit.import_table" => {
                let csv = s_arg(args, "csv")?;
                let create_missing = bool_arg(args, "create_missing")?.unwrap_or(false);
                let report = map_io::table::import(&mut d, csv, create_missing).map_err(ApiError::BadRequest)?;
                let note = format!("导入 CSV：{} 行，写入 {} 项", report.rows, report.values_set);
                (json!({"rows": report.rows, "values_set": report.values_set, "fields_created": report.fields_created,
                    "entities_created": report.entities_created, "unmatched": report.unmatched}), note)
            }
            "edit.add_label" => {
                let text = s_arg(args, "text")?;
                let (lon, lat) = (args.get("lon").and_then(Value::as_f64), args.get("lat").and_then(Value::as_f64));
                let (Some(lon), Some(lat)) = (lon, lat) else { return Err(ApiError::BadRequest("需要 lon 与 lat".into())) };
                let id = LineId(d.topo.alloc());
                d.annotations.insert(id, map_core::doc::Annotation::new(text, LonLat::new(lon, lat)));
                (json!({"label": id.0}), format!("地图文字“{text}”"))
            }
            _ => return Err(ApiError::Unsupported(format!("没有编辑操作“{op}”"))),
        };
        d.topo.validate().map_err(|e| ApiError::Failed(format!("编辑后拓扑无效，未采用：{e}")))?;
        d.set_layer_and_year(layer0, year0).map_err(ApiError::Failed)?;
        *self.draft_mut() = d;
        self.log.push(note.clone());
        Ok(json!({"done": note, "result": result, "edits_in_draft": self.log.len()}))
    }

    /// What the draft changes, in the request's year and layer: each unit's
    /// area before and after, units added and gone, renames, and the log.
    fn changes(&self, ctx: &Context) -> Value {
        let Some(draft) = &self.draft else { return json!({"draft": false, "edits": []}) };
        let (a, b) = (
            self.view_original(ctx).areas_by_entity(),
            self.view(ctx).areas_by_entity(),
        );
        let mut rows = Vec::new();
        for (e, x) in draft.entities.iter() {
            let (before, after) = (a.get(e).copied().unwrap_or(0.0), b.get(e).copied().unwrap_or(0.0));
            let old = self.doc.entities.get(e);
            if (before - after).abs() > 1e-6 * before.max(1.0) || old.is_none_or(|o| o.name != x.name) {
                rows.push(json!({"id": e.0, "name": x.name, "was": old.map(|o| o.name.clone()), "new": old.is_none(), "area_before_km2": before.round(), "area_after_km2": after.round()}));
            }
        }
        let gone: Vec<Value> = self.doc.entities.iter().filter(|(e, _)| !draft.entities.contains_key(*e)).map(|(e, x)| json!({"id": e.0, "name": x.name})).collect();
        json!({"draft": true, "year": ctx.year, "edits": self.log, "units": rows, "removed": gone, "events_added": draft.events.len() as i64 - self.doc.events.len() as i64})
    }

    /// The draft, when there is one.
    pub fn draft(&self) -> Option<&Document> {
        self.draft.as_ref()
    }

    /// What the draft's edits were, in order.
    pub fn edit_log(&self) -> &[String] {
        &self.log
    }
}
