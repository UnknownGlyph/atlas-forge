//! Map previews that match the data: the image is drawn from the same
//! world version, year and layer as the answer, and the answer says where
//! in the image each country, city and symbol is, by id.

use crate::queries::find_entity;
use crate::{ApiError, ApiResult, Context, Session};
use map_core::doc::EntityKind;
use map_core::geom;
use map_core::LonLat;
use map_io::batch::Format;
use map_io::scene::{ExportOptions, PixelMap};
use serde_json::{json, Value};
use std::collections::BTreeSet;

fn hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Projected bounds of lon/lat points (continuous longitudes), with a margin.
fn projected_bounds(o: &ExportOptions, pts: &[LonLat]) -> Option<[f64; 4]> {
    if pts.is_empty() {
        return None;
    }
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in pts {
        let q = o.projection.forward_deg(geom::wrap_lon(p.lon - o.lon0), p.lat);
        b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[0]), b[3].max(q[1])];
    }
    let (mx, my) = (((b[2] - b[0]) * 0.08).max(1e-3), ((b[3] - b[1]) * 0.08).max(1e-3));
    Some([b[0] - mx, b[1] - my, b[2] + mx, b[3] + my])
}

pub fn render(s: &Session, args: &Value, ctx: &Context) -> ApiResult {
    let path = args.get("path").and_then(Value::as_str).ok_or_else(|| ApiError::BadRequest("缺少输出文件 path".into()))?;
    let path = std::path::PathBuf::from(path);
    let format = match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("png") => Format::Png,
        Some("svg") => Format::Svg,
        Some("pdf") => Format::Pdf,
        _ => return Err(ApiError::BadRequest("path 须以 .png、.svg 或 .pdf 结尾".into())),
    };
    // the draft (what reads see), or the world before it
    let d = if args.get("before").and_then(Value::as_bool) == Some(true) { s.view_original(ctx) } else { s.view(ctx) };
    let mut o = match args.get("layout").and_then(Value::as_str) {
        Some(name) => ExportOptions::from_layout(d.layout(name).ok_or_else(|| ApiError::NotFound(format!("没有名为“{name}”的版式")))?),
        None => ExportOptions { width: 1200, height: 700, ..Default::default() },
    };
    let size = |k: &str| args.get(k).and_then(Value::as_u64).map(|v| v.clamp(64, 8000) as u32);
    if let Some(w) = size("width") {
        o.width = w;
    }
    if let Some(h) = size("height") {
        o.height = h;
    }
    if let Some(t) = args.get("title").and_then(Value::as_str) {
        o.title = t.replace("{year}", &ctx.year.map(|y| y.to_string()).unwrap_or_default());
    }
    o.legend = args.get("legend").and_then(Value::as_bool).unwrap_or(o.legend);
    o.labels = args.get("labels").and_then(Value::as_bool).unwrap_or(o.labels);
    // focus on a country, or a lon/lat box
    if let Some(f) = args.get("focus").filter(|f| !f.is_null()) {
        let pts: Vec<LonLat> = if let Some(b) = f.get("bounds").and_then(Value::as_array) {
            let v: Vec<f64> = b.iter().filter_map(Value::as_f64).collect();
            let [w, so, e, n] = v[..] else { return Err(ApiError::BadRequest("focus.bounds 须为 [西, 南, 东, 北]".into())) };
            let e = if e < w { e + 360.0 } else { e };
            (0..=10).flat_map(|i| (0..=10).map(move |j| LonLat::new(w + (e - w) * i as f64 / 10.0, so + (n - so) * j as f64 / 10.0))).collect()
        } else {
            // {entity: id}, or {id | code | name}
            let named = match f.get("entity") {
                Some(id) => json!({"id": id}),
                None => f.clone(),
            };
            let e = find_entity(&d, &named)?;
            d.dissolve(e).iter().flat_map(|p| p.outer.iter().map(|q| LonLat::new(q[0], q[1]))).collect()
        };
        o.bounds = Some(projected_bounds(&o, &pts).ok_or_else(|| ApiError::BadRequest("聚焦的实体在该年没有领土".into()))?);
    }
    map_io::batch::write_one(&d, format, &o, s.font.as_deref(), &path).map_err(ApiError::Failed)?;

    // where things are in the image
    let pm = PixelMap::new(&o);
    let (w, h) = (o.width as f64, o.height as f64);
    let inside = |b: &[f64; 4]| b[2] >= 0.0 && b[0] <= w && b[3] >= 0.0 && b[1] <= h;
    let r2 = |v: f64| (v * 10.0).round() / 10.0;
    let mut objects = Vec::new();
    let owners: BTreeSet<_> = d.areas_by_entity().into_keys().collect();
    for e in owners {
        let Some(x) = d.entities.get(&e) else { continue };
        // the box of the pieces in view (a far-away island does not
        // stretch it across the image)
        let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for poly in d.dissolve(e) {
            let mut pb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
            for q in &poly.outer {
                let p = pm.px(LonLat::new(q[0], q[1]));
                pb = [pb[0].min(p[0]), pb[1].min(p[1]), pb[2].max(p[0]), pb[3].max(p[1])];
            }
            // a piece crossing the image's seam spans it; take it as is
            if pb[0].is_finite() && inside(&pb) {
                b = [b[0].min(pb[0]), b[1].min(pb[1]), b[2].max(pb[2]), b[3].max(pb[3])];
            }
        }
        if !b[0].is_finite() {
            continue;
        }
        // the name's place, when it is in the image
        let anchor = d.label_anchor(e).map(|(p, _)| pm.px(p)).filter(|p| inside(&[p[0], p[1], p[0], p[1]])).map(|p| [r2(p[0]), r2(p[1])]);
        objects.push(json!({
            "kind": match x.kind { EntityKind::Region => "region", _ => "country" },
            "id": e.0,
            "name": x.name,
            "color": hex(x.color),
            "anchor_px": anchor,
            "bbox_px": [r2(b[0].max(0.0)), r2(b[1].max(0.0)), r2(b[2].min(w)), r2(b[3].min(h))],
        }));
    }
    for (id, x) in d.entities.iter().filter(|(_, x)| x.kind == EntityKind::City && x.exists_in(ctx.year)) {
        let Some(pos) = x.pos else { continue };
        let p = pm.px(pos);
        if inside(&[p[0], p[1], p[0], p[1]]) {
            objects.push(json!({"kind": "city", "id": id.0, "name": x.name, "anchor_px": [r2(p[0]), r2(p[1])]}));
        }
    }
    for (id, x) in d.symbols.iter().filter(|(id, x)| x.shown_in(ctx.year) && !d.group_hides(**id)) {
        let p = pm.px(x.at);
        if inside(&[p[0], p[1], p[0], p[1]]) {
            objects.push(json!({"kind": "symbol", "id": id.0, "symbol": x.kind.name(), "name": x.name, "anchor_px": [r2(p[0]), r2(p[1])]}));
        }
    }
    let theme = map_core::theme::evaluate(&d);
    Ok(json!({
        "path": path.display().to_string(),
        "format": format.ext(),
        "width": o.width,
        "height": o.height,
        "projection": o.projection.name(),
        "legend": {"title": theme.title, "rows": theme.legend.iter().map(|i| json!({"label": i.label, "color": hex(i.color)})).collect::<Vec<_>>()},
        "objects": objects,
        "pixel_note": "像素坐标以图像左上角为原点，x 向右、y 向下；anchor_px 为名称位置（不在图内时为 null），bbox_px 为该对象在图内各块领土的范围（已裁到图像内）。",
    }))
}
