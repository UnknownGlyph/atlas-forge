//! Tabular data import for visualization rules.
use map_core::{visualization::Row, world_book::Node, LonLat};
use std::path::Path;

pub fn headers(path: &Path) -> Result<Vec<String>, String> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    Ok(reader
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(str::to_owned)
        .collect())
}

#[derive(Clone, Copy, Debug)]
pub struct Columns {
    pub label: Option<usize>,
    pub x: Option<usize>,
    pub y: usize,
    pub coordinates: Option<(usize, usize)>,
    pub source: Option<usize>,
}

pub fn read(path: &Path, columns: Columns) -> Result<Vec<Row>, String> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    if columns.y >= reader.headers().map_err(|e| e.to_string())?.len() {
        return Err("数值列不存在。".into());
    }
    let mut out = Vec::new();
    for (i, r) in reader.records().enumerate() {
        if i >= 250_000 {
            return Err("表格超过 250,000 行。".into());
        }
        let row = r.map_err(|e| e.to_string())?;
        let parse = |index| {
            row.get(index)
                .and_then(|s: &str| s.trim().parse::<f64>().ok())
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("CSV 第 {} 行，第 {} 列不是有效数字。", i + 2, index + 1))
        };
        let value = parse(columns.y)?;
        let x = columns.x.map_or(Ok(i as f64 + 1.0), parse)?;
        let point = columns
            .coordinates
            .map(|(lon, lat)| {
                let (lon, lat) = (parse(lon)?, parse(lat)?);
                if (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat) {
                    Ok(LonLat::new(lon, lat))
                } else {
                    Err(format!("CSV 第 {} 行经纬度越界。", i + 2))
                }
            })
            .transpose()?;
        let node = columns
            .source
            .map(|column| {
                let uri = row.get(column).unwrap_or("").trim();
                if uri.is_empty() {
                    Ok(None)
                } else {
                    Node::parse(uri)
                        .map(Some)
                        .ok_or_else(|| format!("CSV 第 {} 行的工程对象标识无效。", i + 2))
                }
            })
            .transpose()?
            .flatten();
        let label = columns
            .label
            .and_then(|j| row.get(j))
            .map_or_else(|| (i + 1).to_string(), str::to_owned);
        out.push(Row {
            label,
            x,
            value,
            point,
            node,
        });
    }
    if out.is_empty() {
        return Err("表格中没有数据行。".into());
    }
    Ok(out)
}

pub fn write(path: &Path, data: &map_core::visualization::Dataset) -> Result<(), String> {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(["label", "x", "value", "longitude", "latitude", "atlas_uri"])
        .map_err(|e| e.to_string())?;
    for row in &data.rows {
        let (lon, lat) = row.point.map_or((String::new(), String::new()), |p| {
            (p.lon.to_string(), p.lat.to_string())
        });
        writer
            .write_record([
                row.label.clone(),
                row.x.to_string(),
                row.value.to_string(),
                lon,
                lat,
                row.node.map_or_else(String::new, Node::uri),
            ])
            .map_err(|e| e.to_string())?;
    }
    let bytes = writer.into_inner().map_err(|e| e.to_string())?;
    crate::recovery::write_atomic(path, &bytes).map_err(|e| e.to_string())
}
