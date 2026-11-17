//! Statistics in a selected year, with consistent borders (ADR-7, settled
//! 2026-10-29): a value computed from area uses the borders of the year
//! its input belongs to. Population recorded in 1900 is divided by the 1900
//! area even when 1960 is selected; interpolated values and undated base
//! values use the selected year's borders; inputs from different years
//! fall back to the selected year and are marked as mixed.

use crate::doc::{Document, EntityId, Source, AREA_KEY};
use crate::expr;
use std::cell::RefCell;
use std::collections::HashMap;

/// Whose borders an area-based value was computed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Borders {
    /// The selected year's (or no area involved).
    Selected,
    /// Another year's, the year the inputs belong to.
    Year(i32),
    /// Inputs from different years: the selected year's, marked.
    Mixed,
}

impl Borders {
    pub fn label(self) -> Option<String> {
        match self {
            Borders::Selected => None,
            Borders::Year(y) => Some(format!("口径：{y} 年边界")),
            Borders::Mixed => Some("混合口径".into()),
        }
    }
}

/// A numeric value with where it comes from and whose borders it uses.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stat {
    pub value: Option<f64>,
    pub source: Source,
    pub borders: Borders,
}

impl Stat {
    /// Export tag, e.g. "interpolated:1900-2020" or "derived:borders-1900".
    pub fn tag(&self) -> String {
        match self.borders {
            Borders::Selected => self.source.tag(),
            Borders::Year(y) => format!("{}:borders-{y}", self.source.tag()),
            Borders::Mixed => format!("{}:mixed", self.source.tag()),
        }
    }

    /// For display next to a value, e.g. "计算值，口径：1900 年边界".
    pub fn label(&self) -> String {
        match self.borders.label() {
            Some(b) => format!("{}，{b}", self.source.label()),
            None => self.source.label(),
        }
    }
}

/// Shared context for many lookups in one document state: areas in the
/// selected year, and areas in other years computed once on demand.
pub struct Stats<'a> {
    doc: &'a Document,
    areas: HashMap<EntityId, f64>,
    other: RefCell<HashMap<i32, HashMap<EntityId, f64>>>,
}

impl<'a> Stats<'a> {
    pub fn new(doc: &'a Document) -> Self {
        Self::with_areas(doc, doc.areas_by_entity())
    }

    pub fn with_areas(doc: &'a Document, areas: HashMap<EntityId, f64>) -> Self {
        Self { doc, areas, other: RefCell::new(HashMap::new()) }
    }

    /// Reuse other years' areas computed earlier for the same document
    /// state (the caller decides when that state changed).
    pub fn with_cache(doc: &'a Document, areas: HashMap<EntityId, f64>, other: HashMap<i32, HashMap<EntityId, f64>>) -> Self {
        Self { doc, areas, other: RefCell::new(other) }
    }

    /// Other years' areas computed so far, to pass to [`Stats::with_cache`].
    pub fn into_cache(self) -> HashMap<i32, HashMap<EntityId, f64>> {
        self.other.into_inner()
    }

    /// Areas in the selected year.
    pub fn areas(&self) -> &HashMap<EntityId, f64> {
        &self.areas
    }

    fn area_in(&self, e: EntityId, year: Option<i32>) -> Option<f64> {
        match year {
            Some(y) if Some(y) != self.doc.year => {
                let mut cache = self.other.borrow_mut();
                let m = cache.entry(y).or_insert_with(|| {
                    let mut d = self.doc.clone();
                    d.set_year(Some(y));
                    d.areas_by_entity()
                });
                m.get(&e).copied()
            }
            _ => self.areas.get(&e).copied(),
        }
    }

    pub fn value(&self, e: EntityId, key: &str) -> Option<f64> {
        self.stat(e, key).value
    }

    pub fn stat(&self, e: EntityId, key: &str) -> Stat {
        self.stat_depth(e, key, 0)
    }

    /// The year an input ties the borders to: a record's or carried-over
    /// record's year; None for the selected year (interpolated, plain base).
    fn input_year(&self, s: &Stat) -> Option<Option<i32>> {
        match (s.source, s.borders) {
            (_, Borders::Year(y)) => Some(Some(y)),
            (_, Borders::Mixed) => None,
            (Source::Recorded(y) | Source::Inherited(y), _) => Some(Some(y)),
            _ => Some(None),
        }
    }

    fn stat_depth(&self, e: EntityId, key: &str, depth: u32) -> Stat {
        let unknown = Stat { value: None, source: Source::Unknown, borders: Borders::Selected };
        if key == AREA_KEY {
            return Stat { value: self.areas.get(&e).copied(), source: Source::Derived, borders: Borders::Selected };
        }
        if depth > 8 {
            return unknown;
        }
        let Some(fid) = self.doc.field_by_key(key) else { return unknown };
        let def = &self.doc.fields[&fid];
        let Some(src) = &def.expr else {
            let (v, source) = self.doc.value_with_source(e, fid);
            // a base value with an effective year counts as that year's record
            let borders = match (source, def.base_year) {
                (Source::Base, Some(by)) if self.doc.year.is_some() && self.doc.year != Some(by) => Borders::Year(by),
                _ => Borders::Selected,
            };
            return Stat { value: v.and_then(|v| v.as_num()), source, borders };
        };
        let Ok(ast) = expr::parse(src) else { return unknown };
        // first pass: which years do the inputs belong to?
        let inputs: RefCell<Vec<Stat>> = RefCell::new(Vec::new());
        let first = expr::eval(&ast, &|k| {
            if k == AREA_KEY {
                return self.areas.get(&e).copied();
            }
            let s = self.stat_depth(e, k, depth + 1);
            inputs.borrow_mut().push(s);
            s.value
        });
        // border year of each input that has a value
        let (mut tied, mut selected, mut mixed, mut nested) = (Vec::new(), false, false, false);
        for s in inputs.borrow().iter().filter(|s| s.value.is_some()) {
            nested |= s.borders != Borders::Selected;
            match self.input_year(s) {
                None => mixed = true,
                Some(Some(y)) if Some(y) != self.doc.year => tied.push(y),
                _ => selected = true,
            }
        }
        tied.sort();
        tied.dedup();
        let uses_area = src.contains(AREA_KEY) || nested;
        let borders = if !uses_area {
            Borders::Selected
        } else if mixed || tied.len() > 1 || (tied.len() == 1 && selected) {
            Borders::Mixed
        } else if let [y] = tied[..] {
            Borders::Year(y)
        } else {
            Borders::Selected
        };
        let value = match borders {
            // second pass with that year's area
            Borders::Year(y) => expr::eval(&ast, &|k| if k == AREA_KEY { self.area_in(e, Some(y)) } else { self.stat_depth(e, k, depth + 1).value }),
            _ => first,
        };
        Stat { value, source: if value.is_some() { Source::Derived } else { Source::Unknown }, borders }
    }
}
