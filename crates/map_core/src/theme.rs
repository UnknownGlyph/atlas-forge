//! Thematic styling: political colouring and choropleth classification.
//! Produces per-face colours that both the GPU renderer and exporters use.

use crate::doc::{format_num, Document, EntityId, AREA_KEY};
use crate::topo::{FaceId, FaceKind, BACKGROUND};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemeMode {
    Political,
    Choropleth,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassMethod {
    EqualInterval,
    Quantile,
    Manual,
}

/// Whether class breaks follow the selected year's values, or are worked
/// out once over all years so a colour means the same value in every year
/// (for comparing years, per-year exports and animations).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ClassScale {
    #[default]
    PerYear,
    AllYears,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ThemeSpec {
    pub mode: ThemeMode,
    pub field_key: String,
    pub method: ClassMethod,
    pub classes: u32,
    pub manual_breaks: Vec<f64>,
    pub ramp: Vec<[u8; 4]>,
    pub missing: [u8; 4],
    #[serde(default)]
    pub scale: ClassScale,
}

impl Default for ThemeSpec {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Political,
            field_key: AREA_KEY.into(),
            method: ClassMethod::Quantile,
            classes: 5,
            manual_breaks: vec![],
            ramp: RAMPS[0].1.to_vec(),
            missing: [200, 200, 200, 255],
            scale: ClassScale::PerYear,
        }
    }
}

pub const RAMPS: &[(&str, &[[u8; 4]])] = &[
    ("黄-橙-红", &[[255, 255, 204, 255], [254, 217, 118, 255], [253, 141, 60, 255], [227, 26, 28, 255], [128, 0, 38, 255]]),
    ("蓝", &[[239, 243, 255, 255], [189, 215, 231, 255], [107, 174, 214, 255], [33, 113, 181, 255], [8, 48, 107, 255]]),
    ("绿", &[[237, 248, 233, 255], [186, 228, 179, 255], [116, 196, 118, 255], [35, 139, 69, 255], [0, 90, 50, 255]]),
    ("Viridis", &[[68, 1, 84, 255], [59, 82, 139, 255], [33, 145, 140, 255], [94, 201, 98, 255], [253, 231, 37, 255]]),
    ("红-黄-绿", &[[215, 25, 28, 255], [253, 174, 97, 255], [255, 255, 191, 255], [166, 217, 106, 255], [26, 150, 65, 255]]),
];

pub fn sample_ramp(ramp: &[[u8; 4]], t: f64) -> [u8; 4] {
    if ramp.is_empty() {
        return [128, 128, 128, 255];
    }
    if ramp.len() == 1 {
        return ramp[0];
    }
    let t = t.clamp(0.0, 1.0) * (ramp.len() - 1) as f64;
    let i = (t.floor() as usize).min(ramp.len() - 2);
    let f = t - i as f64;
    let (a, b) = (ramp[i], ramp[i + 1]);
    let mut out = [0u8; 4];
    for k in 0..4 {
        out[k] = (a[k] as f64 + (b[k] as f64 - a[k] as f64) * f).round() as u8;
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct LegendItem {
    pub label: String,
    pub color: [u8; 4],
}

#[derive(Clone, Debug, Default)]
pub struct ThemeResult {
    pub title: String,
    pub face_colors: HashMap<FaceId, [u8; 4]>,
    pub entity_colors: HashMap<EntityId, [u8; 4]>,
    pub entity_values: HashMap<EntityId, f64>,
    pub legend: Vec<LegendItem>,
}

/// Class break values (upper bounds excluding the last = max).
pub fn compute_breaks(values: &[f64], method: ClassMethod, classes: usize, manual: &[f64]) -> Vec<f64> {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return vec![];
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (lo, hi) = (v[0], v[v.len() - 1]);
    let k = classes.max(1);
    let mut b: Vec<f64> = match method {
        ClassMethod::EqualInterval => (1..k).map(|i| lo + (hi - lo) * i as f64 / k as f64).collect(),
        ClassMethod::Quantile => (1..k).map(|i| v[((v.len() * i) / k).min(v.len() - 1)]).collect(),
        ClassMethod::Manual => manual.to_vec(),
    };
    b.sort_by(|a, b| a.partial_cmp(b).unwrap());
    b.dedup();
    b
}

pub fn class_of(x: f64, breaks: &[f64]) -> usize {
    breaks.iter().take_while(|b| x >= **b).count()
}

/// The entities coloured in the document's year: those owning land (on a
/// share layer every unit holding any share).
fn owned(doc: &Document) -> Vec<EntityId> {
    let mut v: Vec<EntityId> = doc.owner.values().copied().collect();
    if doc.active_is_shares() {
        v.extend(doc.areas_by_entity().keys().copied());
    }
    v.sort();
    v.dedup();
    v
}

/// Class breaks shared by all years ([`ClassScale::AllYears`]).
#[derive(Clone, Debug, PartialEq)]
pub struct FixedScale {
    pub breaks: Vec<f64>,
    /// Smallest and largest value over all the years.
    pub lo: f64,
    pub hi: f64,
    /// The years the values were taken from.
    pub years: Vec<i32>,
}

/// The years a shared scale is worked out from: the year before the first
/// change and every change year (borders, and so areas, change only then),
/// and every year the field, or a field its expression uses, has a record
/// (in between, values are carried over or interpolated, so stay within).
pub fn scale_years(doc: &Document) -> Vec<i32> {
    let mut ys = doc.change_years();
    if let Some(f) = ys.first().copied() {
        ys.insert(f - 1);
    }
    let mut keys = vec![doc.theme.field_key.clone()];
    let mut seen = std::collections::HashSet::new();
    while let Some(k) = keys.pop() {
        if !seen.insert(k.clone()) {
            continue;
        }
        let Some(fid) = doc.field_by_key(&k) else { continue };
        let def = &doc.fields[&fid];
        ys.extend(def.base_year);
        ys.extend(doc.values.iter().filter(|((_, f), _)| *f == fid).flat_map(|(_, s)| s.by_year.keys().copied()));
        if let Some(src) = &def.expr {
            if let Ok(a) = crate::expr::parse(src) {
                crate::expr::vars(&a, &mut keys);
            }
        }
    }
    ys.into_iter().collect()
}

/// The shared class breaks for a choropleth set to [`ClassScale::AllYears`]:
/// every coloured entity's value in every [`scale_years`] year, classified
/// together. None when the theme does not ask for it or there are no years
/// (a map without years: the per-year classes are the only ones).
pub fn fixed_scale(doc: &Document) -> Option<FixedScale> {
    let spec = &doc.theme;
    if spec.mode != ThemeMode::Choropleth || spec.scale != ClassScale::AllYears {
        return None;
    }
    let years = scale_years(doc);
    // faces keep their shape through the years: their areas are worked out once
    let r = doc.radius();
    let face_area: std::collections::HashMap<crate::topo::FaceId, f64> = doc.topo.faces.keys().map(|f| (*f, doc.topo.face_area(*f, r))).collect();
    let face_area = &face_area;
    // each year's values are independent: years are shared out over the cores
    let values_in = |ys: &[i32]| -> Vec<f64> {
        let mut d = doc.clone();
        let mut out = Vec::new();
        for y in ys {
            d.set_year(Some(*y));
            let areas = d.areas_by_entity_with(|f| face_area.get(&f).copied().unwrap_or(0.0));
            let stats = crate::stats::Stats::with_areas(&d, areas);
            out.extend(owned(&d).into_iter().filter_map(|e| stats.value(e, &spec.field_key)).filter(|v| v.is_finite()));
        }
        out
    };
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()).min(years.len().div_ceil(8)).max(1);
    let all: Vec<f64> = if workers == 1 {
        values_in(&years)
    } else {
        let chunk = years.len().div_ceil(workers);
        std::thread::scope(|s| {
            let hs: Vec<_> = years.chunks(chunk).map(|c| s.spawn(move || values_in(c))).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
        })
    };
    if all.is_empty() {
        return None;
    }
    let lo = all.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = all.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    Some(FixedScale { breaks: compute_breaks(&all, spec.method, spec.classes as usize, &spec.manual_breaks), lo, hi, years })
}

/// For the legend title, e.g. "各年统一分级 1899–1950 年".
fn scale_note(f: &FixedScale) -> String {
    match (f.years.first(), f.years.last()) {
        (Some(a), Some(b)) if a != b => format!("各年统一分级 {a}–{b} 年"),
        (Some(a), _) => format!("统一分级 {a} 年"),
        _ => "统一分级".into(),
    }
}

/// Colours and legend for the document's year; a theme set to
/// [`ClassScale::AllYears`] works out the shared classes first (callers
/// drawing many years should use [`evaluate_with`] and [`fixed_scale`]).
pub fn evaluate(doc: &Document) -> ThemeResult {
    evaluate_with(doc, fixed_scale(doc).as_ref())
}

/// As [`evaluate`], with the shared classes given (`None`: this year's own).
pub fn evaluate_with(doc: &Document, fixed: Option<&FixedScale>) -> ThemeResult {
    let spec = &doc.theme;
    let st = &doc.style;
    let mut res = ThemeResult::default();
    let owned = owned(doc);
    match spec.mode {
        ThemeMode::Political => {
            res.title = "政治地图".into();
            for e in &owned {
                let top = doc.top_ancestor(*e);
                if let Some(ent) = doc.entities.get(&top) {
                    res.entity_colors.insert(*e, st.toned(ent.color));
                }
            }
            for id in doc.countries() {
                let e = &doc.entities[&id];
                if owned.contains(&id) {
                    res.legend.push(LegendItem { label: e.name.clone(), color: st.toned(e.color) });
                }
            }
        }
        ThemeMode::Choropleth => {
            let stats = crate::stats::Stats::new(doc);
            let mut stat_of = HashMap::new();
            for e in &owned {
                let st = stats.stat(*e, &spec.field_key);
                if let Some(v) = st.value {
                    res.entity_values.insert(*e, v);
                    stat_of.insert(*e, st);
                }
            }
            let vals: Vec<f64> = res.entity_values.values().copied().collect();
            let breaks = match fixed {
                Some(f) => f.breaks.clone(),
                None => compute_breaks(&vals, spec.method, spec.classes as usize, &spec.manual_breaks),
            };
            let n = breaks.len() + 1;
            let color_of = |c: usize| sample_ramp(&spec.ramp, if n > 1 { c as f64 / (n - 1) as f64 } else { 0.5 });
            for e in &owned {
                let c = match res.entity_values.get(e) {
                    Some(v) => color_of(class_of(*v, &breaks)),
                    None => spec.missing,
                };
                res.entity_colors.insert(*e, c);
            }
            let name = if spec.field_key == AREA_KEY {
                "面积 (km²)".to_string()
            } else {
                doc.field_by_key(&spec.field_key).map(|f| doc.fields[&f].name.clone()).unwrap_or(spec.field_key.clone())
            };
            res.title = name;
            // say so when values are not records of the selected year
            if let Some(y) = doc.year {
                let (mut inherited, mut interpolated, mut base, mut other_borders, mut mixed) = (0, 0, 0, 0, 0);
                for st in stat_of.values() {
                    match st.source {
                        crate::doc::Source::Inherited(_) => inherited += 1,
                        crate::doc::Source::Interpolated(..) => interpolated += 1,
                        crate::doc::Source::Base => base += 1,
                        _ => {}
                    }
                    match st.borders {
                        crate::stats::Borders::Year(_) => other_borders += 1,
                        crate::stats::Borders::Mixed => mixed += 1,
                        crate::stats::Borders::Selected => {}
                    }
                }
                let mut notes = vec![format!("{y} 年")];
                for (n, what) in [(inherited, "沿用早年记录"), (interpolated, "插值"), (base, "基础值"), (other_borders, "按数据所属年份的边界"), (mixed, "混合口径")] {
                    if n > 0 {
                        notes.push(format!("{n} 个{what}"));
                    }
                }
                if let Some(f) = fixed {
                    notes.push(scale_note(f));
                }
                res.title = format!("{}（{}）", res.title, notes.join("，"));
            } else if let Some(f) = fixed {
                res.title = format!("{}（{}）", res.title, scale_note(f));
            }
            if !vals.is_empty() || fixed.is_some() {
                // a shared scale's legend covers every year's values
                let (mut lo, mut hi) = fixed.map_or((f64::INFINITY, f64::NEG_INFINITY), |f| (f.lo, f.hi));
                lo = vals.iter().copied().fold(lo, f64::min);
                hi = vals.iter().copied().fold(hi, f64::max);
                let mut edges = vec![lo];
                edges.extend(breaks.iter().copied());
                edges.push(hi);
                for c in 0..n {
                    res.legend.push(LegendItem {
                        label: format!("{} – {}", format_num(edges[c]), format_num(edges[c + 1])),
                        color: color_of(c),
                    });
                }
            }
            if res.entity_values.len() < owned.len() {
                res.legend.push(LegendItem { label: "无数据".into(), color: spec.missing });
            }
        }
    }
    let shares_layer = doc.active_is_shares();
    for (fid, f) in doc.topo.faces.iter() {
        let c = match f.kind {
            FaceKind::Water if *fid == BACKGROUND => st.ocean,
            FaceKind::Water => st.lake,
            // a share layer: the units' colours mixed by share, the rest land colour
            FaceKind::Land if shares_layer => {
                let sh = doc.shares_shown(*fid);
                if sh.is_empty() {
                    st.land
                } else {
                    let mut acc = [0.0f64; 4];
                    let mut used = 0u32;
                    for (e, bp) in &sh {
                        let c = res.entity_colors.get(e).copied().or_else(|| doc.entities.get(e).map(|x| x.color)).unwrap_or(st.land);
                        for k in 0..4 {
                            acc[k] += c[k] as f64 * *bp as f64;
                        }
                        used += *bp as u32;
                    }
                    let rest = (crate::doc::FULL_SHARE as u32).saturating_sub(used) as f64;
                    let mut c = [0u8; 4];
                    for k in 0..4 {
                        c[k] = ((acc[k] + st.land[k] as f64 * rest) / crate::doc::FULL_SHARE as f64).round() as u8;
                    }
                    c
                }
            }
            FaceKind::Land => doc.owner.get(fid).and_then(|e| res.entity_colors.get(e)).copied().unwrap_or(st.land),
        };
        res.face_colors.insert(*fid, c);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn breaks_and_classes() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let b = compute_breaks(&v, ClassMethod::EqualInterval, 3, &[]);
        assert_eq!(b, vec![4.0, 7.0]);
        assert_eq!(class_of(1.0, &b), 0);
        assert_eq!(class_of(5.0, &b), 1);
        assert_eq!(class_of(10.0, &b), 2);
        let q = compute_breaks(&v, ClassMethod::Quantile, 2, &[]);
        assert_eq!(q, vec![6.0]);
        assert_eq!(sample_ramp(&[[0, 0, 0, 255], [200, 100, 0, 255]], 0.5), [100, 50, 0, 255]);
    }
}
