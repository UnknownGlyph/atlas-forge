//! ESRI Shapefile (.shp/.shx/.dbf, with .prj and .cpg) without GDAL.
//!
//! Reading turns polygons and lines into a GeoJSON FeatureCollection, so
//! the GeoJSON importers (islands, or a new world with shared borders) are
//! reused unchanged. Only longitude/latitude coordinates are read: a
//! projected shapefile is refused with a hint, since there is no PROJ here
//! to unproject it. Attribute text is decoded with the .cpg code page, the
//! dBase language driver, or else UTF-8 / GBK by inspection.
//!
//! Writing takes a GeoJSON FeatureCollection of one geometry family (as
//! the exporters make: countries, faces, or rivers and routes) and writes
//! the set as WGS 84 longitude/latitude, attributes in UTF-8.

use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const POLYLINE: i32 = 3;
const POLYGON: i32 = 5;

/// What reading a shapefile found.
#[derive(Debug, Default)]
pub struct ShpRead {
    /// A GeoJSON FeatureCollection.
    pub geojson: String,
    pub features: usize,
    /// Point, multipoint, empty and deleted records left out.
    pub skipped: usize,
    /// How attribute text was decoded.
    pub encoding: String,
}

fn be_i32(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn le_i32(b: &[u8], at: usize) -> Option<i32> {
    Some(i32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}
fn le_f64(b: &[u8], at: usize) -> Option<f64> {
    Some(f64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// Twice the signed area of a ring (positive: counter-clockwise, x east, y north).
fn signed_area2(r: &[[f64; 2]]) -> f64 {
    let n = r.len();
    (0..n).map(|i| {
        let (a, b) = (r[i], r[(i + 1) % n]);
        a[0] * b[1] - b[0] * a[1]
    })
    .sum()
}

fn inside(p: [f64; 2], r: &[[f64; 2]]) -> bool {
    let mut c = false;
    let n = r.len();
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (r[i], r[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0] {
            c = !c;
        }
        j = i;
    }
    c
}

/// Group a polygon record's rings: clockwise rings are outlines, the
/// others holes of the outline they lie in. Files written the other way
/// round (no clockwise ring) are read with the roles swapped.
fn group_rings(rings: Vec<Vec<[f64; 2]>>) -> Vec<Vec<Vec<[f64; 2]>>> {
    let cw: Vec<bool> = rings.iter().map(|r| signed_area2(r) < 0.0).collect();
    let flip = !cw.iter().any(|c| *c);
    let mut polys: Vec<Vec<Vec<[f64; 2]>>> = Vec::new();
    let mut holes = Vec::new();
    for (r, c) in rings.into_iter().zip(cw) {
        if c != flip {
            polys.push(vec![r]);
        } else {
            holes.push(r);
        }
    }
    for h in holes {
        match polys.iter_mut().rev().find(|p| inside(h[0], &p[0])) {
            Some(p) => p.push(h),
            // a hole in nothing: an outline drawn the wrong way
            None => polys.push(vec![h]),
        }
    }
    polys
}

fn closed(r: &[[f64; 2]]) -> Value {
    let mut v: Vec<Value> = r.iter().map(|p| json!([p[0], p[1]])).collect();
    if r.first() != r.last() {
        v.push(json!([r[0][0], r[0][1]]));
    }
    Value::Array(v)
}

/// One record's geometry as GeoJSON; None for shapes not read here.
fn geometry(content: &[u8]) -> Result<Option<Value>, String> {
    let ty = le_i32(content, 0).ok_or("记录太短")?;
    let base = match ty {
        3 | 13 | 23 => POLYLINE,
        5 | 15 | 25 => POLYGON,
        _ => return Ok(None),
    };
    let bad = || "记录内容不完整".to_string();
    let parts = le_i32(content, 36).ok_or_else(bad)?.max(0) as usize;
    let points = le_i32(content, 40).ok_or_else(bad)?.max(0) as usize;
    let mut starts = Vec::with_capacity(parts);
    for i in 0..parts {
        starts.push(le_i32(content, 44 + 4 * i).ok_or_else(bad)?.max(0) as usize);
    }
    let p0 = 44 + 4 * parts;
    let mut pts = Vec::with_capacity(points);
    for i in 0..points {
        pts.push([le_f64(content, p0 + 16 * i).ok_or_else(bad)?, le_f64(content, p0 + 16 * i + 8).ok_or_else(bad)?]);
    }
    let mut lines: Vec<Vec<[f64; 2]>> = Vec::new();
    for (k, s) in starts.iter().enumerate() {
        let e = starts.get(k + 1).copied().unwrap_or(points).min(points);
        if *s < e {
            lines.push(pts[*s..e].to_vec());
        }
    }
    if base == POLYLINE {
        let lines: Vec<Value> = lines.iter().filter(|l| l.len() >= 2).map(|l| Value::Array(l.iter().map(|p| json!([p[0], p[1]])).collect())).collect();
        return Ok(match lines.len() {
            0 => None,
            1 => Some(json!({"type": "LineString", "coordinates": lines[0]})),
            _ => Some(json!({"type": "MultiLineString", "coordinates": lines})),
        });
    }
    let rings: Vec<Vec<[f64; 2]>> = lines
        .into_iter()
        .map(|mut r| {
            if r.len() > 1 && r.first() == r.last() {
                r.pop();
            }
            r
        })
        .filter(|r| r.len() >= 3)
        .collect();
    let polys: Vec<Value> = group_rings(rings).iter().map(|p| Value::Array(p.iter().map(|r| closed(r)).collect())).collect();
    Ok(match polys.len() {
        0 => None,
        1 => Some(json!({"type": "Polygon", "coordinates": polys[0]})),
        _ => Some(json!({"type": "MultiPolygon", "coordinates": polys})),
    })
}

/// The text encoding of the attribute table: the .cpg file, else the
/// dBase language driver byte, else None (decide from the bytes).
fn declared_encoding(cpg: Option<&str>, driver: u8) -> Option<&'static encoding_rs::Encoding> {
    if let Some(c) = cpg {
        let c = c.trim();
        let label = match c.to_ascii_lowercase().as_str() {
            "936" | "cp936" | "gb2312" | "gbk" | "ansi 936" => "gbk",
            "65001" | "utf8" | "utf-8" => "utf-8",
            "950" | "cp950" | "big5" => "big5",
            "1252" | "cp1252" | "ansi 1252" => "windows-1252",
            "932" | "cp932" | "shift_jis" => "shift_jis",
            "949" | "cp949" => "euc-kr",
            other => return encoding_rs::Encoding::for_label(other.as_bytes()),
        };
        return encoding_rs::Encoding::for_label(label.as_bytes());
    }
    match driver {
        0x4D | 0x7A => Some(encoding_rs::GBK),
        0x4F | 0x78 => Some(encoding_rs::BIG5),
        0x57 | 0x03 => Some(encoding_rs::WINDOWS_1252),
        0x4E | 0x13 | 0x7B => Some(encoding_rs::SHIFT_JIS),
        _ => None,
    }
}

/// Attribute rows (None for a deleted row) and the encoding used.
fn read_dbf(b: &[u8], cpg: Option<&str>) -> Result<(Vec<Option<Map<String, Value>>>, String), String> {
    if b.len() < 32 {
        return Err("DBF 文件太短".into());
    }
    let n = u32::from_le_bytes(b[4..8].try_into().unwrap()) as usize;
    let header = u16::from_le_bytes(b[8..10].try_into().unwrap()) as usize;
    let rec_len = u16::from_le_bytes(b[10..12].try_into().unwrap()) as usize;
    let mut fields = Vec::new();
    let mut at = 32;
    while at + 32 <= header.min(b.len()) && b[at] != 0x0D {
        let raw = &b[at..at + 11];
        let name = &raw[..raw.iter().position(|c| *c == 0).unwrap_or(11)];
        fields.push((name.to_vec(), b[at + 11], b[at + 16] as usize));
        at += 32;
    }
    // every text cell, to decide the encoding when nothing declares it
    let enc = declared_encoding(cpg, b[29]).unwrap_or_else(|| {
        let utf8 = (0..n).all(|i| {
            let s = header + i * rec_len;
            b.get(s..s + rec_len).is_none_or(|r| std::str::from_utf8(r).is_ok())
        }) && fields.iter().all(|f| std::str::from_utf8(&f.0).is_ok());
        if utf8 {
            encoding_rs::UTF_8
        } else {
            encoding_rs::GBK
        }
    });
    let text = |bytes: &[u8]| enc.decode_without_bom_handling(bytes).0.trim().to_string();
    let names: Vec<String> = fields.iter().map(|f| text(&f.0)).collect();
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let s = header + i * rec_len;
        let Some(r) = b.get(s..s + rec_len) else { break };
        if r[0] == b'*' {
            rows.push(None);
            continue;
        }
        let mut props = Map::new();
        let mut off = 1;
        for ((_, ty, len), name) in fields.iter().zip(&names) {
            let cell = r.get(off..off + len).unwrap_or(&[]);
            off += len;
            let t = text(cell);
            let v = match ty {
                b'N' | b'F' => t.parse::<f64>().map(|x| json!(x)).unwrap_or(Value::Null),
                b'L' => match t.chars().next() {
                    Some('T' | 't' | 'Y' | 'y') => json!(true),
                    Some('F' | 'f' | 'N' | 'n') => json!(false),
                    _ => Value::Null,
                },
                b'D' if t.len() == 8 => json!(format!("{}-{}-{}", &t[..4], &t[4..6], &t[6..])),
                _ if t.is_empty() => Value::Null,
                _ => json!(t),
            };
            props.insert(name.clone(), v);
        }
        rows.push(Some(props));
    }
    Ok((rows, enc.name().to_string()))
}

fn sibling(shp: &Path, ext: &str) -> Option<PathBuf> {
    // either case of the extension
    [ext.to_string(), ext.to_uppercase()].into_iter().map(|e| shp.with_extension(e)).find(|p| p.exists())
}

/// Read `path` (the .shp; the .dbf, .prj and .cpg beside it are used when
/// present) as a GeoJSON FeatureCollection.
pub fn read(path: &Path) -> Result<ShpRead, String> {
    let shp = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if shp.len() < 100 || be_i32(&shp, 0) != Some(9994) {
        return Err("不是 Shapefile（.shp 文件头无效）".into());
    }
    let unproject = "Atlas Forge 只读取经纬度坐标（WGS 84 等地理坐标系）；请先在 QGIS 等软件中另存为 EPSG:4326";
    if let Some(prj) = sibling(path, "prj").and_then(|p| std::fs::read_to_string(p).ok()) {
        if prj.trim_start().to_ascii_uppercase().starts_with("PROJCS") {
            let name = prj.split('"').nth(1).unwrap_or("投影坐标系");
            return Err(format!("该 Shapefile 使用投影坐标（{name}）。{unproject}"));
        }
    }
    let bbox = [le_f64(&shp, 36).unwrap_or(0.0), le_f64(&shp, 44).unwrap_or(0.0), le_f64(&shp, 52).unwrap_or(0.0), le_f64(&shp, 60).unwrap_or(0.0)];
    if bbox[0] < -360.5 || bbox[2] > 360.5 || bbox[1] < -90.5 || bbox[3] > 90.5 {
        return Err(format!("坐标范围（{:.0}, {:.0}）–（{:.0}, {:.0}）不是经纬度。{unproject}", bbox[0], bbox[1], bbox[2], bbox[3]));
    }
    let (rows, encoding) = match sibling(path, "dbf") {
        Some(p) => {
            let b = std::fs::read(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            let cpg = sibling(path, "cpg").and_then(|p| std::fs::read_to_string(p).ok());
            read_dbf(&b, cpg.as_deref())?
        }
        None => (vec![], "无属性表".into()),
    };
    let mut out = ShpRead { encoding, ..Default::default() };
    let mut feats = Vec::new();
    let mut at = 100;
    let mut i = 0;
    while at + 8 <= shp.len() {
        let len = be_i32(&shp, at + 4).unwrap_or(0).max(0) as usize * 2;
        let content = shp.get(at + 8..at + 8 + len).ok_or_else(|| format!("第 {} 条记录超出文件末尾", i + 1))?;
        at += 8 + len;
        let row = rows.get(i).cloned();
        i += 1;
        if matches!(row, Some(None)) {
            out.skipped += 1;
            continue;
        }
        match geometry(content).map_err(|e| format!("第 {i} 条记录：{e}"))? {
            Some(g) => {
                feats.push(json!({"type": "Feature", "properties": row.flatten().unwrap_or_default(), "geometry": g}));
                out.features += 1;
            }
            None => out.skipped += 1,
        }
    }
    out.geojson = serde_json::to_string(&json!({"type": "FeatureCollection", "features": feats})).unwrap();
    Ok(out)
}

/// A shape ready to write: its parts (rings or lines) and bounding box.
struct Shape {
    parts: Vec<Vec<[f64; 2]>>,
}

fn coords(v: &Value) -> Vec<[f64; 2]> {
    v.as_array().map(|a| a.iter().filter_map(|p| Some([p[0].as_f64()?, p[1].as_f64()?])).collect()).unwrap_or_default()
}

/// A GeoJSON geometry as shapefile parts: outlines clockwise and holes
/// counter-clockwise, rings closed.
fn shape_of(g: &Value) -> Option<(i32, Shape)> {
    let polys: Vec<&Value> = match g["type"].as_str()? {
        "Polygon" => vec![&g["coordinates"]],
        "MultiPolygon" => g["coordinates"].as_array()?.iter().collect(),
        "LineString" => return Some((POLYLINE, Shape { parts: vec![coords(&g["coordinates"])] })),
        "MultiLineString" => return Some((POLYLINE, Shape { parts: g["coordinates"].as_array()?.iter().map(coords).collect() })),
        _ => return None,
    };
    let mut parts = Vec::new();
    for p in polys {
        for (k, r) in p.as_array()?.iter().enumerate() {
            let mut r = coords(r);
            if r.len() > 1 && r.first() == r.last() {
                r.pop();
            }
            if r.len() < 3 {
                continue;
            }
            let ccw = signed_area2(&r) > 0.0;
            if (k == 0) == ccw {
                r.reverse();
            }
            r.push(r[0]);
            parts.push(r);
        }
    }
    Some((POLYGON, Shape { parts }))
}

/// dBase column for one property.
struct Column {
    key: String,
    name: [u8; 11],
    ty: u8,
    len: usize,
    dec: usize,
}

/// A dBase field name: at most 10 bytes (cut on a character boundary),
/// unique among `taken`.
fn dbf_name(key: &str, taken: &mut Vec<Vec<u8>>) -> [u8; 11] {
    let cut = |s: &str, max: usize| -> Vec<u8> {
        let mut end = 0;
        for (i, c) in s.char_indices() {
            if i + c.len_utf8() > max {
                break;
            }
            end = i + c.len_utf8();
        }
        s.as_bytes()[..end].to_vec()
    };
    let mut n = cut(if key.is_empty() { "field" } else { key }, 10);
    let mut k = 1;
    while taken.contains(&n) {
        let tail = format!("_{k}");
        n = cut(key, 10 - tail.len());
        n.extend_from_slice(tail.as_bytes());
        k += 1;
    }
    taken.push(n.clone());
    let mut out = [0u8; 11];
    out[..n.len()].copy_from_slice(&n);
    out
}

fn num_text(x: f64, len: usize, dec: usize) -> String {
    let s = format!("{x:.dec$}");
    if s.len() <= len {
        return s;
    }
    let s = format!("{x:.e$}", e = dec.min(6));
    if s.len() <= len {
        s
    } else {
        format!("{x:e}")
    }
}

/// Text cut to `max` bytes on a character boundary.
fn clip(s: &str, max: usize) -> &str {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i + c.len_utf8() > max {
            break;
        }
        end = i + c.len_utf8();
    }
    &s[..end]
}

fn columns(rows: &[&Map<String, Value>]) -> Vec<Column> {
    let mut keys: Vec<String> = Vec::new();
    for r in rows {
        for k in r.keys() {
            if !keys.contains(k) {
                keys.push(k.clone());
            }
        }
    }
    let mut taken = Vec::new();
    keys.into_iter()
        .map(|key| {
            let vals: Vec<&Value> = rows.iter().filter_map(|r| r.get(&key)).filter(|v| !v.is_null()).collect();
            let (ty, len, dec) = if !vals.is_empty() && vals.iter().all(|v| v.is_number()) {
                (b'N', 20, 6)
            } else if !vals.is_empty() && vals.iter().all(|v| v.is_boolean()) {
                (b'L', 1, 0)
            } else {
                let w = vals.iter().map(|v| v.as_str().map_or_else(|| v.to_string().len(), str::len)).max().unwrap_or(1);
                (b'C', w.clamp(1, 254), 0)
            };
            Column { name: dbf_name(&key, &mut taken), key, ty, len, dec }
        })
        .collect()
}

fn dbf(rows: &[&Map<String, Value>]) -> Vec<u8> {
    let cols = columns(rows);
    let rec_len = 1 + cols.iter().map(|c| c.len).sum::<usize>();
    let header = 32 + 32 * cols.len() + 1;
    let mut b = vec![0u8; 32];
    b[0] = 0x03;
    // a fixed date keeps output identical between runs
    b[1..4].copy_from_slice(&[100, 1, 1]);
    b[4..8].copy_from_slice(&(rows.len() as u32).to_le_bytes());
    b[8..10].copy_from_slice(&(header as u16).to_le_bytes());
    b[10..12].copy_from_slice(&(rec_len as u16).to_le_bytes());
    for c in &cols {
        let mut f = [0u8; 32];
        f[..11].copy_from_slice(&c.name);
        f[11] = c.ty;
        f[16] = c.len as u8;
        f[17] = c.dec as u8;
        b.extend_from_slice(&f);
    }
    b.push(0x0D);
    for r in rows {
        b.push(b' ');
        for c in &cols {
            let v = r.get(&c.key).unwrap_or(&Value::Null);
            let cell = match (c.ty, v) {
                (_, Value::Null) => String::new(),
                (b'N', v) => format!("{:>w$}", num_text(v.as_f64().unwrap_or(0.0), c.len, c.dec), w = c.len),
                (b'L', Value::Bool(t)) => if *t { "T" } else { "F" }.into(),
                (_, Value::String(s)) => clip(s, c.len).to_string(),
                (_, v) => clip(&v.to_string(), c.len).to_string(),
            };
            let mut bytes = cell.into_bytes();
            bytes.truncate(c.len);
            bytes.resize(c.len, b' ');
            b.extend_from_slice(&bytes);
        }
    }
    b.push(0x1A);
    b
}

const WGS84: &str = r#"GEOGCS["GCS_WGS_1984",DATUM["D_WGS_1984",SPHEROID["WGS_1984",6378137.0,298.257223563]],PRIMEM["Greenwich",0.0],UNIT["Degree",0.0174532925199433]]"#;

fn header(ty: i32, words: usize, bbox: [f64; 4]) -> Vec<u8> {
    let mut h = vec![0u8; 100];
    h[0..4].copy_from_slice(&9994i32.to_be_bytes());
    h[24..28].copy_from_slice(&(words as i32).to_be_bytes());
    h[28..32].copy_from_slice(&1000i32.to_le_bytes());
    h[32..36].copy_from_slice(&ty.to_le_bytes());
    for (k, v) in bbox.iter().enumerate() {
        h[36 + 8 * k..44 + 8 * k].copy_from_slice(&v.to_le_bytes());
    }
    h
}

/// Write a GeoJSON FeatureCollection of polygons or of lines as a
/// shapefile set at `path` (.shp, .shx, .dbf, .prj, .cpg). Features of the
/// other family or without geometry are left out. Returns the files.
pub fn write_geojson(text: &str, path: &Path) -> Result<Vec<PathBuf>, String> {
    let root: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let feats = root["features"].as_array().cloned().unwrap_or_default();
    let shaped: Vec<(i32, Shape, &Map<String, Value>)> = feats
        .iter()
        .filter_map(|f| {
            let (ty, s) = shape_of(&f["geometry"])?;
            Some((ty, s, f["properties"].as_object()?))
        })
        .filter(|(_, s, _)| s.parts.iter().any(|p| p.len() >= 2))
        .collect();
    // one geometry family per file: the first feature's
    let ty = shaped.first().map_or(POLYGON, |s| s.0);
    let shaped: Vec<&(i32, Shape, &Map<String, Value>)> = shaped.iter().filter(|s| s.0 == ty).collect();
    let mut bbox = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    let mut recs: Vec<Vec<u8>> = Vec::new();
    for (_, s, _) in &shaped {
        let pts: Vec<[f64; 2]> = s.parts.iter().flatten().copied().collect();
        let mut bb = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
        for p in &pts {
            bb = [bb[0].min(p[0]), bb[1].min(p[1]), bb[2].max(p[0]), bb[3].max(p[1])];
        }
        bbox = [bbox[0].min(bb[0]), bbox[1].min(bb[1]), bbox[2].max(bb[2]), bbox[3].max(bb[3])];
        let mut c = Vec::with_capacity(44 + 4 * s.parts.len() + 16 * pts.len());
        c.extend_from_slice(&ty.to_le_bytes());
        for v in bb {
            c.extend_from_slice(&v.to_le_bytes());
        }
        c.extend_from_slice(&(s.parts.len() as i32).to_le_bytes());
        c.extend_from_slice(&(pts.len() as i32).to_le_bytes());
        let mut start = 0i32;
        for p in &s.parts {
            c.extend_from_slice(&start.to_le_bytes());
            start += p.len() as i32;
        }
        for p in &pts {
            c.extend_from_slice(&p[0].to_le_bytes());
            c.extend_from_slice(&p[1].to_le_bytes());
        }
        recs.push(c);
    }
    if recs.is_empty() {
        bbox = [0.0; 4];
    }
    let total: usize = recs.iter().map(|r| 8 + r.len()).sum();
    let mut shp = header(ty, (100 + total) / 2, bbox);
    let mut shx = header(ty, (100 + 8 * recs.len()) / 2, bbox);
    for (i, r) in recs.iter().enumerate() {
        shx.extend_from_slice(&((shp.len() / 2) as i32).to_be_bytes());
        shx.extend_from_slice(&((r.len() / 2) as i32).to_be_bytes());
        shp.extend_from_slice(&(i as i32 + 1).to_be_bytes());
        shp.extend_from_slice(&((r.len() / 2) as i32).to_be_bytes());
        shp.extend_from_slice(r);
    }
    let rows: Vec<&Map<String, Value>> = shaped.iter().map(|s| s.2).collect();
    let files = [
        (path.with_extension("shp"), shp),
        (path.with_extension("shx"), shx),
        (path.with_extension("dbf"), dbf(&rows)),
        (path.with_extension("prj"), WGS84.as_bytes().to_vec()),
        (path.with_extension("cpg"), b"UTF-8".to_vec()),
    ];
    let mut out = Vec::new();
    for (p, b) in files {
        std::fs::write(&p, b).map_err(|e| format!("{}: {e}", p.display()))?;
        out.push(p);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("shp_test_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn polygons_with_holes_and_attributes_round_trip() {
        let gj = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {"name": "北原王国", "code": "C001", "area_km2": 12345.678, "coastal": true, "note": null},
             "geometry": {"type": "Polygon", "coordinates": [
                 [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0], [0.0, 0.0]],
                 [[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0], [2.0, 2.0]]]}},
            {"type": "Feature", "properties": {"name": "群岛", "code": "C002", "area_km2": 5, "coastal": false, "note": "两块"},
             "geometry": {"type": "MultiPolygon", "coordinates": [
                 [[[20.0, 0.0], [25.0, 0.0], [25.0, 5.0], [20.0, 0.0]]],
                 [[[30.0, 0.0], [35.0, 0.0], [35.0, 5.0], [30.0, 0.0]]]]}}
        ]});
        let dir = tmp("poly");
        let files = write_geojson(&gj.to_string(), &dir.join("国家.shp")).unwrap();
        assert_eq!(files.len(), 5);
        assert_eq!(std::fs::read_to_string(dir.join("国家.cpg")).unwrap(), "UTF-8");
        // .shx offsets point at the records
        let (shp, shx) = (std::fs::read(dir.join("国家.shp")).unwrap(), std::fs::read(dir.join("国家.shx")).unwrap());
        assert_eq!(be_i32(&shp, 24).unwrap() as usize * 2, shp.len());
        assert_eq!(shx.len(), 100 + 16);
        assert_eq!(be_i32(&shx, 100), Some(50));
        assert_eq!(be_i32(&shp, 100), Some(1));
        let r = read(&dir.join("国家.shp")).unwrap();
        assert_eq!((r.features, r.skipped, r.encoding.as_str()), (2, 0, "UTF-8"));
        let v: Value = serde_json::from_str(&r.geojson).unwrap();
        let f = &v["features"];
        assert_eq!(f[0]["properties"]["name"], "北原王国");
        assert_eq!(f[0]["properties"]["code"], "C001");
        assert!((f[0]["properties"]["area_km2"].as_f64().unwrap() - 12345.678).abs() < 1e-9);
        assert_eq!(f[0]["properties"]["coastal"], true);
        assert!(f[0]["properties"]["note"].is_null());
        assert_eq!(f[1]["properties"]["note"], "两块");
        // the hole stays a hole of the square; two islands stay two
        assert_eq!(f[0]["geometry"]["type"], "Polygon");
        assert_eq!(f[0]["geometry"]["coordinates"].as_array().unwrap().len(), 2);
        assert_eq!(f[1]["geometry"]["type"], "MultiPolygon");
        assert_eq!(f[1]["geometry"]["coordinates"].as_array().unwrap().len(), 2);
        // and the result imports as a world with the same areas
        let (doc, rep) = crate::geojson::import_topology(&r.geojson).unwrap();
        assert_eq!(rep.countries, 2);
        assert!(doc.countries().iter().any(|c| doc.entities[c].name == "北原王国"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lines_and_long_field_names() {
        let gj = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {"population_source": "a", "population_sourcf": "b"},
             "geometry": {"type": "MultiLineString", "coordinates": [[[170.0, 0.0], [180.0, 1.0]], [[-180.0, 1.0], [-170.0, 2.0]]]}},
            {"type": "Feature", "properties": {"population_source": "c"}, "geometry": {"type": "Polygon", "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]}}
        ]});
        let dir = tmp("line");
        write_geojson(&gj.to_string(), &dir.join("r.shp")).unwrap();
        let r = read(&dir.join("r.shp")).unwrap();
        // one family per file: the polygon is left out
        assert_eq!(r.features, 1);
        let v: Value = serde_json::from_str(&r.geojson).unwrap();
        let p = v["features"][0]["properties"].as_object().unwrap();
        assert_eq!(p.keys().cloned().collect::<Vec<_>>(), ["populati_1", "population"]);
        assert_eq!(v["features"][0]["geometry"]["type"], "MultiLineString");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn projected_files_and_gbk_attributes() {
        let gj = json!({"type": "FeatureCollection", "features": [
            {"type": "Feature", "properties": {"name": "x"}, "geometry": {"type": "Polygon", "coordinates": [[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 0.0]]]}}]});
        let dir = tmp("prj");
        let p = dir.join("a.shp");
        write_geojson(&gj.to_string(), &p).unwrap();
        // a GBK table without .cpg: recognised from the bytes
        std::fs::remove_file(dir.join("a.cpg")).unwrap();
        let mut dbf = std::fs::read(dir.join("a.dbf")).unwrap();
        let (gbk, _, _) = encoding_rs::GBK.encode("河谷");
        let header = u16::from_le_bytes(dbf[8..10].try_into().unwrap()) as usize;
        // one row of one 1-byte cell: widen the column to 4 bytes, GBK text in it
        dbf[32 + 16] = 4;
        dbf[10..12].copy_from_slice(&5u16.to_le_bytes());
        dbf.truncate(header + 1);
        dbf.extend_from_slice(&gbk);
        dbf.push(0x1A);
        std::fs::write(dir.join("a.dbf"), &dbf).unwrap();
        let r = read(&p).unwrap();
        assert_eq!(r.encoding, "GBK");
        assert!(r.geojson.contains("河谷"));
        // projected coordinates are refused with a hint
        std::fs::write(dir.join("a.prj"), r#"PROJCS["WGS_1984_Web_Mercator_Auxiliary_Sphere",GEOGCS["GCS_WGS_1984"]]"#).unwrap();
        let e = read(&p).unwrap_err();
        assert!(e.contains("Web_Mercator") && e.contains("EPSG:4326"), "{e}");
        assert!(read(&dir.join("a.dbf")).is_err(), "not a .shp");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
