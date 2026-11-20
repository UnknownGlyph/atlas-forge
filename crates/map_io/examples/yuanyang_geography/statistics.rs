use super::districts::{share, METROS};
use crate::{Ally, Region, SOVEREIGN_DATA};
use map_core::{
    doc::{EntityKind, FieldId, FieldKind, Value},
    Document, EntityId,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy)]
struct Totals {
    population: usize,
    gdp_million: usize,
}

impl Totals {
    fn from_brief(population_wan: u32, gdp_yi: u32) -> Self {
        Self {
            population: population_wan as usize * 10000,
            gdp_million: gdp_yi as usize * 100,
        }
    }
}

pub struct Fields {
    pub area: FieldId,
    population: FieldId,
    gdp: FieldId,
    source: FieldId,
    pub rank: FieldId,
    pub note: FieldId,
}

impl Fields {
    pub fn new(doc: &mut Document) -> Result<Self, String> {
        Ok(Self {
            area: doc.add_field(
                "official_area",
                "设定面积（万 km²）",
                FieldKind::Number,
                None,
            )?,
            population: doc.add_field("population", "人口（万）", FieldKind::Number, None)?,
            gdp: doc.add_field("necessary_gdp", "GDP（亿远洋币）", FieldKind::Number, None)?,
            source: doc.add_field("stat_source", "数据来源与口径", FieldKind::Text, None)?,
            rank: doc.add_field(
                "administrative_rank",
                "行政或城市等级",
                FieldKind::Text,
                None,
            )?,
            note: doc.add_field("geographic_note", "地理与经济说明", FieldKind::Text, None)?,
        })
    }

    pub fn text(&self, doc: &mut Document, id: EntityId, field: FieldId, text: &str) {
        doc.set_value(id, field, Some(2897), Some(Value::Text(text.into())));
    }

    pub fn number(&self, doc: &mut Document, id: EntityId, field: FieldId, value: f64) {
        doc.set_value(id, field, Some(2897), Some(Value::Num(value)));
    }

    fn totals(&self, doc: &mut Document, id: EntityId, total: Totals, source: &str) {
        self.number(doc, id, self.population, total.population as f64 / 10000.0);
        self.number(doc, id, self.gdp, total.gdp_million as f64 / 100.0);
        self.text(doc, id, self.source, source);
    }

    pub fn seed(
        &self,
        doc: &mut Document,
        sovereign: &BTreeMap<&str, EntityId>,
        provinces: &BTreeMap<&str, EntityId>,
        home: &[Region],
        allies: &[Ally],
    ) {
        for (code, area, pop, gdp) in SOVEREIGN_DATA {
            let id = sovereign[code];
            self.number(doc, id, self.area, area);
            self.totals(
                doc,
                id,
                Totals::from_brief(pop, gdp),
                if code == "YY" {
                    "地理志 · 全国必要 GDP"
                } else {
                    "地理志 · 全国全口径 GDP"
                },
            );
            self.text(doc, id, self.rank, "国家");
            if code == "YY" {
                continue;
            }
            let provinces_of_country: Vec<_> =
                allies.iter().filter(|r| r.country == code).collect();
            let weights: Vec<_> = provinces_of_country
                .iter()
                .map(|r| {
                    f64::from(r.pop)
                        * (1.0 + 0.18 * r.note.matches(['港', '工', '贸', '金']).count() as f64)
                })
                .collect();
            let budgets = share(gdp as usize * 100, &weights);
            for (r, gdp_million) in provinces_of_country.into_iter().zip(budgets) {
                let id = provinces[r.code.as_str()];
                self.number(doc, id, self.area, r.area);
                self.totals(
                    doc,
                    id,
                    Totals {
                        population: r.pop as usize * 10000,
                        gdp_million,
                    },
                    "地理志面积和人口 · 省级 GDP 按人口及产业设定分摊",
                );
                self.text(doc, id, self.rank, "一级行政区");
                self.text(doc, id, self.note, &r.note.replace("**", ""));
            }
        }
        for r in home {
            let id = provinces[r.code.as_str()];
            self.number(doc, id, self.area, r.area);
            self.totals(
                doc,
                id,
                Totals::from_brief(r.pop, r.gdp),
                "地理志 · 一级行政区设定值",
            );
            let rank = if METROS.contains(&r.code.as_str()) {
                "直辖市"
            } else {
                "一级行政区"
            };
            self.text(doc, id, self.rank, rank);
        }
    }

    pub fn local(
        &self,
        doc: &mut Document,
        home: &[Region],
        provinces: &BTreeMap<&str, EntityId>,
        areas: &BTreeMap<EntityId, f64>,
    ) {
        let mut children = BTreeMap::<EntityId, Vec<EntityId>>::new();
        let mut seats = Vec::new();
        for (id, entity) in &doc.entities {
            if entity.kind == EntityKind::Country && entity.layer == doc.active_layer {
                if let Some(parent) = entity.parent {
                    children.entry(parent).or_default().push(*id);
                }
            }
            if entity.kind == EntityKind::City {
                if let Some(parent) = entity.parent {
                    seats.push((*id, parent));
                }
            }
        }
        for ids in children.values_mut() {
            ids.sort();
        }
        // A primary seat is weighted above its rural hinterland. The
        // province's published totals remain fixed.
        for r in home {
            let province = provinces[r.code.as_str()];
            for &prefecture in children.get(&province).into_iter().flatten() {
                self.number(
                    doc,
                    prefecture,
                    self.area,
                    doc_area(areas, &children, prefecture) / 1e4,
                );
                for &county in children.get(&prefecture).into_iter().flatten() {
                    self.number(
                        doc,
                        county,
                        self.area,
                        doc_area(areas, &children, county) / 1e4,
                    );
                }
            }
            let Some(prefectures) = children.get(&province) else {
                continue;
            };
            let total = Totals::from_brief(r.pop, r.gdp);
            let (pops, gdps) = allocate(total, prefectures, |id| {
                let capital = doc.entities[&province].capital == doc.entities[&id].capital;
                doc_area(areas, &children, id).max(1.0).sqrt() * if capital { 1.85 } else { 1.0 }
            });
            for (&prefecture, (population, gdp_million)) in
                prefectures.iter().zip(pops.into_iter().zip(gdps))
            {
                let total = Totals {
                    population,
                    gdp_million,
                };
                self.totals(doc, prefecture, total, "示例估算 · 含下辖县级市");
                let county_ids = &children[&prefecture];
                let mut pieces = vec![prefecture];
                pieces.extend(county_ids);
                let (pops, gdps) = allocate(total, &pieces, |id| {
                    areas.get(&id).copied().unwrap_or(1.0).sqrt()
                        * if id == prefecture { 1.7 } else { 1.0 }
                });
                for (&unit, (population, gdp_million)) in
                    pieces.iter().zip(pops.into_iter().zip(gdps))
                {
                    let total = Totals {
                        population,
                        gdp_million,
                    };
                    if unit != prefecture {
                        self.totals(doc, unit, total, "示例估算 · 县级辖区合计");
                    }
                    if let Some(city) = doc.entities[&unit].capital {
                        self.totals(doc, city, total, "示例估算 · 城区及其直辖腹地");
                    }
                }
            }
        }
        // Direct municipalities and allied seats do not contain subdivided city
        // polygons. Use a separate core estimate so it is not the province total.
        for (city, parent) in seats {
            if doc.value_with_source(city, self.population).0.is_some() {
                continue;
            }
            let number = |field| match doc.value_with_source(parent, field).0 {
                Some(Value::Num(n)) => n,
                _ => 0.0,
            };
            let pop = number(self.population);
            let gdp = number(self.gdp);
            self.totals(
                doc,
                city,
                Totals {
                    population: (pop * 10000.0 * 0.57).round() as usize,
                    gdp_million: (gdp * 100.0 * 0.69).round() as usize,
                },
                "示例估算 · 首府城区",
            );
        }
    }

    pub fn export(&self, doc: &Document, path: &Path) -> Result<(), String> {
        let mut csv = csv::Writer::from_path(path).map_err(|e| e.to_string())?;
        csv.write_record([
            "编码",
            "名称",
            "类型",
            "上级编码",
            "等级",
            "面积_万平方千米",
            "人口_万",
            "GDP_亿远洋币",
            "来源",
        ])
        .map_err(|e| e.to_string())?;
        let mut rows: Vec<_> = doc
            .entities
            .iter()
            .filter(|(_, e)| e.layer == doc.active_layer)
            .collect();
        rows.sort_by_key(|(_, e)| e.code.as_str());
        for (&id, entity) in rows {
            let field = |key| match doc.value_with_source(id, key).0 {
                Some(Value::Num(n)) => format!("{n:.4}"),
                Some(Value::Text(s)) => s,
                _ => String::new(),
            };
            let parent = entity
                .parent
                .and_then(|p| doc.entities.get(&p))
                .map_or("", |p| p.code.as_str());
            let kind = if entity.kind == EntityKind::City {
                "城市点"
            } else {
                "辖区"
            };
            csv.write_record([
                &entity.code,
                &entity.name,
                kind,
                parent,
                &field(self.rank),
                &field(self.area),
                &field(self.population),
                &field(self.gdp),
                &field(self.source),
            ])
            .map_err(|e| e.to_string())?;
        }
        csv.flush().map_err(|e| e.to_string())
    }
}

fn doc_area(
    areas: &BTreeMap<EntityId, f64>,
    children: &BTreeMap<EntityId, Vec<EntityId>>,
    id: EntityId,
) -> f64 {
    areas.get(&id).copied().unwrap_or(0.0)
        + children
            .get(&id)
            .into_iter()
            .flatten()
            .map(|&c| doc_area(areas, children, c))
            .sum::<f64>()
}

fn allocate(
    total: Totals,
    ids: &[EntityId],
    weight: impl Fn(EntityId) -> f64,
) -> (Vec<usize>, Vec<usize>) {
    let weights: Vec<_> = ids.iter().map(|&id| weight(id)).collect();
    let economy: Vec<_> = weights.iter().map(|&w| w.powf(1.15)).collect();
    (
        share(total.population, &weights),
        share(total.gdp_million, &economy),
    )
}
