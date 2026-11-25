//! CSV attribute exchange, joined on the entity's stable `code`.
//!
//! Import format: a header row with a key column (`code`, or `name` as a
//! fallback), an optional `year` column (values go into the time series),
//! and one column per field. Unknown columns create new fields; the type is
//! Number if every non-empty cell parses as a number, otherwise Text.

use map_core::doc::{format_num, Document, EntityId, EntityKind, FieldDef, FieldId, FieldKind, Value, AREA_KEY};
use map_core::stats::Stats;

/// A field and its provenance in the selected year, shared by CSV and GeoJSON.
pub(crate) fn field_cell(doc: &Document, stats: &Stats<'_>, entity: EntityId, id: FieldId, field: &FieldDef) -> (Option<Value>, Option<String>) {
    if field.expr.is_some() {
        let stat = stats.stat(entity, &field.key);
        (stat.value.map(Value::Num), doc.year.map(|_| stat.tag()))
    } else {
        let (value, source) = doc.value_with_source(entity, id);
        (value, doc.year.map(|_| source.tag()))
    }
}

pub fn export(doc: &Document) -> String {
    let stats = Stats::new(doc);
    let mut w = csv::Writer::from_writer(Vec::new());
    let mut header = vec!["code".to_string(), "name".into(), "kind".into(), AREA_KEY.into()];
    // with a year selected each field gets a `<key>_source` column
    let sources = doc.year.is_some();
    for f in doc.fields.values() {
        header.push(f.key.clone());
        if sources {
            header.push(format!("{}_source", f.key));
        }
    }
    let _ = w.write_record(&header);
    for (id, e) in doc.entities.iter() {
        let kind = match e.kind {
            EntityKind::Country => "country",
            EntityKind::Region => "region",
            EntityKind::City => "city",
        };
        let mut row = vec![e.code.clone(), e.name.clone(), kind.into(), stats.areas().get(id).map(|a| format_num(*a)).unwrap_or_default()];
        for (fid, f) in &doc.fields {
            let (value, source) = field_cell(doc, &stats, *id, *fid, f);
            row.push(value.map(|v| v.display()).unwrap_or_default());
            row.extend(source);
        }
        let _ = w.write_record(&row);
    }
    String::from_utf8(w.into_inner().unwrap_or_default()).unwrap_or_default()
}

#[derive(Debug, Default)]
pub struct CsvReport {
    pub rows: usize,
    pub values_set: usize,
    pub fields_created: Vec<String>,
    pub entities_created: usize,
    pub unmatched: Vec<String>,
}

pub fn import(doc: &mut Document, text: &str, create_missing: bool) -> Result<CsvReport, String> {
    let text = text.trim_start_matches('\u{feff}');
    let mut rdr = csv::ReaderBuilder::new().flexible(true).from_reader(text.as_bytes());
    let header: Vec<String> = rdr.headers().map_err(|e| format!("CSV 表头错误: {e}"))?.iter().map(|h| h.trim().to_string()).collect();
    let find = |names: &[&str]| header.iter().position(|h| names.iter().any(|n| h.eq_ignore_ascii_case(n)));
    let code_col = find(&["code", "id", "编号", "代码"]);
    let name_col = find(&["name", "名称", "国家"]);
    let key_col = code_col.or(name_col).ok_or("CSV 需要 code 或 name 列用于关联")?;
    let year_col = find(&["year", "年份"]);
    let records: Vec<csv::StringRecord> = rdr.records().collect::<Result<_, _>>().map_err(|e| format!("CSV 读取失败: {e}"))?;
    // `<key>_source` columns written by export describe the values, they are not data
    let is_source = |h: &str| h.strip_suffix("_source").is_some_and(|k| header.iter().any(|x| x == k));
    let skip = |i: usize| Some(i) == code_col || Some(i) == name_col || Some(i) == year_col || header[i].eq_ignore_ascii_case("kind") || header[i] == AREA_KEY || is_source(&header[i]);

    let mut rep = CsvReport { rows: records.len(), ..Default::default() };
    // resolve / create fields
    let mut cols = Vec::new();
    for (i, h) in header.iter().enumerate() {
        if skip(i) || h.is_empty() {
            continue;
        }
        let fid = match doc.field_by_key(h) {
            Some(f) => f,
            None => {
                let numeric = records.iter().filter_map(|r| r.get(i)).filter(|c| !c.trim().is_empty()).all(|c| c.trim().replace(',', "").parse::<f64>().is_ok());
                let kind = if numeric { FieldKind::Number } else { FieldKind::Text };
                let key: String = h.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '_' }).collect();
                let f = match doc.field_by_key(&key) {
                    Some(f) => f,
                    None => {
                        rep.fields_created.push(key.clone());
                        doc.add_field(&key, h, kind, None)?
                    }
                };
                f
            }
        };
        cols.push((i, fid));
    }
    for r in &records {
        let key = r.get(key_col).unwrap_or("").trim();
        if key.is_empty() {
            continue;
        }
        let ent = if Some(key_col) == code_col {
            doc.entity_by_code(key)
        } else {
            doc.entities.iter().find(|(_, e)| e.name == key).map(|(id, _)| *id)
        };
        let ent = match ent {
            Some(e) => e,
            None if create_missing => {
                let name = name_col.and_then(|c| r.get(c)).unwrap_or(key);
                let e = doc.add_entity(EntityKind::Country, name);
                if Some(key_col) == code_col {
                    doc.entities.get_mut(&e).unwrap().code = key.to_string();
                }
                rep.entities_created += 1;
                e
            }
            None => {
                if rep.unmatched.len() < 50 {
                    rep.unmatched.push(key.to_string());
                }
                continue;
            }
        };
        let year = year_col.and_then(|c| r.get(c)).and_then(|y| y.trim().parse::<i32>().ok());
        for (i, fid) in &cols {
            let kind = doc.fields[fid].kind;
            if let Some(v) = r.get(*i).and_then(|c| Value::parse(kind, c)) {
                doc.set_value(ent, *fid, year, Some(v));
                rep.values_set += 1;
            }
        }
    }
    Ok(rep)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn csv_roundtrip_with_years() {
        let mut d = Document::default();
        let a = d.add_entity(EntityKind::Country, "甲");
        let code = d.entities[&a].code.clone();
        let src = format!("code,year,population,motto\n{code},1900,100,统一\n{code},1950,250,\nZZZ,1900,5,x\n");
        let rep = import(&mut d, &src, false).unwrap();
        assert_eq!(rep.values_set, 3);
        assert_eq!(rep.unmatched, vec!["ZZZ".to_string()]);
        assert_eq!(rep.fields_created.len(), 2);
        let areas = d.areas_by_entity();
        // numbers are strict by default
        d.year = Some(1950);
        assert_eq!(d.num_value(a, "population", &areas), Some(250.0));
        d.year = Some(1920);
        assert_eq!(d.num_value(a, "population", &areas), None);
        // text falls back to its base value, which this import did not set
        assert_eq!(d.raw_value(a, d.field_by_key("motto").unwrap()), None);
        let out = export(&d);
        let head = out.lines().next().unwrap();
        assert!(head.contains("population") && head.contains("population_source"));
        assert!(out.contains("甲") && out.contains("unknown"));
        // re-importing an export does not turn source columns into fields
        let n = d.fields.len();
        d.year = Some(1950);
        let again = export(&d);
        assert!(again.contains("recorded:1950"));
        import(&mut d, &again, false).unwrap();
        assert_eq!(d.fields.len(), n);
        d.year = None;
        assert!(!export(&d).contains("_source"), "plain maps export plain tables");
        // create missing
        let rep = import(&mut d, "code,name,gdp\nNEW,新国,3.5\n", true).unwrap();
        assert_eq!(rep.entities_created, 1);
        assert!(d.entity_by_code("NEW").is_some());
    }
}
