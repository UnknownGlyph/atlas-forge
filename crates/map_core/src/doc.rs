//! The map document: topology + political entities + attribute data + style.

use crate::expr;
use crate::geom::{self, LonLat, P2};
use crate::theme::ThemeSpec;
use crate::topo::{ArcRef, FaceId, FaceKind, TResult, Topology, BACKGROUND};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EntityId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct FieldId(pub u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LineId(pub u64);
/// A partition layer (political ownership, actual control, culture, ...).
/// `LayerId(0)` is the political layer every document has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct LayerId(pub u64);
pub const POLITICAL: LayerId = LayerId(0);

/// Ownership storage of one layer: owner from the start, plus change points.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Ownership {
    pub base: OrdMap<FaceId, EntityId>,
    pub changes: OrdMap<FaceId, OwnerChanges>,
}

impl Ownership {
    fn history(&self, f: FaceId) -> (Option<EntityId>, OwnerChanges) {
        (self.base.get(&f).copied(), self.changes.get(&f).cloned().unwrap_or_default())
    }

    fn set(&mut self, f: FaceId, base: Option<EntityId>, ch: OwnerChanges) {
        match base {
            Some(e) => self.base.insert(f, e),
            None => self.base.remove(&f),
        };
        if ch.is_empty() {
            self.changes.remove(&f);
        } else {
            self.changes.insert(f, ch);
        }
    }

    fn same(&self, a: FaceId, b: FaceId) -> bool {
        self.base.get(&a) == self.base.get(&b) && self.changes.get(&a) == self.changes.get(&b)
    }

    fn map_entities(&mut self, m: &dyn Fn(EntityId) -> Option<EntityId>) {
        let faces: HashSet<FaceId> = self.base.keys().chain(self.changes.keys()).copied().collect();
        for f in faces {
            let base = self.base.get(&f).and_then(|e| m(*e));
            let mut ch: OwnerChanges = self.changes.get(&f).map(|c| c.iter().map(|(y, o)| (*y, o.and_then(m))).collect()).unwrap_or_default();
            normalize(base, &mut ch);
            self.set(f, base, ch);
        }
    }

    /// Storage after faces were replaced: faces still alive keep theirs,
    /// new faces start from their source face's history.
    fn inherit(old: &Ownership, live: &dyn Fn(&FaceId) -> bool, src_of: &HashMap<FaceId, Option<FaceId>>) -> Ownership {
        let mut o = Ownership {
            base: old.base.iter().filter(|(f, _)| live(f)).map(|(f, e)| (*f, *e)).collect(),
            changes: old.changes.iter().filter(|(f, _)| live(f)).map(|(f, c)| (*f, c.clone())).collect(),
        };
        for (f, src) in src_of {
            if let Some(s) = src {
                if let Some(e) = old.base.get(s) {
                    o.base.insert(*f, *e);
                }
                if let Some(c) = old.changes.get(s) {
                    o.changes.insert(*f, c.clone());
                }
            }
        }
        o
    }
}

/// Units' shares of one face in basis points (10000 = 100%); the rest is
/// unknown / none. Exact integers, so equal shares compare equal.
pub type Shares = BTreeMap<EntityId, u16>;
pub const FULL_SHARE: u16 = 10000;

/// The unit with the largest share (ties: lower id), if any share is > 0.
pub fn dominant(s: &Shares) -> Option<EntityId> {
    s.iter().filter(|(_, v)| **v > 0).max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(e, _)| *e)
}

/// `cur` with `e` set to `bp`; the other units are scaled down
/// proportionally when the total would exceed 100%.
pub fn rebalance(cur: &Shares, e: EntityId, bp: u16) -> Shares {
    let bp = bp.min(FULL_SHARE);
    let others: Vec<(EntityId, u16)> = cur.iter().filter(|(k, v)| **k != e && **v > 0).map(|(k, v)| (*k, *v)).collect();
    let total: u32 = others.iter().map(|x| x.1 as u32).sum();
    let room = (FULL_SHARE - bp) as u32;
    let mut out: Shares = if total <= room {
        others.into_iter().collect()
    } else {
        others.iter().map(|(k, v)| (*k, (*v as u32 * room / total) as u16)).filter(|x| x.1 > 0).collect()
    };
    if bp > 0 {
        out.insert(e, bp);
    }
    out
}

/// Share storage of one share layer: shares from the start, plus change
/// points (year → the face's whole share table from then on).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShareStore {
    pub base: OrdMap<FaceId, Shares>,
    pub changes: OrdMap<FaceId, BTreeMap<i32, Shares>>,
}

impl ShareStore {
    pub fn at(&self, f: FaceId, year: Option<i32>) -> Shares {
        let base = self.base.get(&f).cloned().unwrap_or_default();
        let Some(ch) = self.changes.get(&f) else { return base };
        let last = match year {
            Some(y) => ch.range(..=y).next_back(),
            None => ch.iter().next_back(),
        };
        last.map_or(base, |(_, s)| s.clone())
    }

    fn history(&self, f: FaceId) -> (Shares, BTreeMap<i32, Shares>) {
        (self.base.get(&f).cloned().unwrap_or_default(), self.changes.get(&f).cloned().unwrap_or_default())
    }

    fn set(&mut self, f: FaceId, base: Shares, ch: BTreeMap<i32, Shares>) {
        if base.is_empty() {
            self.base.remove(&f);
        } else {
            self.base.insert(f, base);
        }
        if ch.is_empty() {
            self.changes.remove(&f);
        } else {
            self.changes.insert(f, ch);
        }
    }

    fn same(&self, a: FaceId, b: FaceId) -> bool {
        self.history(a) == self.history(b)
    }

    fn map_entities(&mut self, m: &dyn Fn(EntityId) -> Option<EntityId>) {
        let remap = |s: &Shares| {
            let mut out = Shares::new();
            for (e, v) in s {
                if let Some(n) = m(*e) {
                    let x = out.entry(n).or_insert(0);
                    *x = (*x + *v).min(FULL_SHARE);
                }
            }
            out
        };
        let faces: HashSet<FaceId> = self.base.keys().chain(self.changes.keys()).copied().collect();
        for f in faces {
            let (b, ch) = self.history(f);
            let b = remap(&b);
            let mut ch: BTreeMap<i32, Shares> = ch.iter().map(|(y, s)| (*y, remap(s))).collect();
            compress(&b, &mut ch);
            self.set(f, b, ch);
        }
    }

    fn inherit(old: &ShareStore, live: &dyn Fn(&FaceId) -> bool, src_of: &HashMap<FaceId, Option<FaceId>>) -> ShareStore {
        let mut o = ShareStore {
            base: old.base.iter().filter(|(f, _)| live(f)).map(|(f, s)| (*f, s.clone())).collect(),
            changes: old.changes.iter().filter(|(f, _)| live(f)).map(|(f, c)| (*f, c.clone())).collect(),
        };
        for (f, src) in src_of {
            if let Some(s) = src {
                let (b, ch) = old.history(*s);
                o.set(*f, b, ch);
            }
        }
        o
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LayerKind {
    /// Each face belongs to at most one unit (political, actual control).
    #[default]
    Exclusive,
    /// Each face is shared between units by percentage (culture, religion).
    Shares,
}

/// A partition layer. Exclusive layers keep their ownership here only while
/// inactive (the active one's lives in `Document::owner_base` /
/// `owner_changes`); share layers always keep theirs in `shares`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    pub name: String,
    #[serde(default)]
    pub parked: Ownership,
    #[serde(default)]
    pub kind: LayerKind,
    #[serde(default)]
    pub shares: ShareStore,
}

pub const FORMAT_VERSION: u32 = 8;
/// Built-in, geometry-derived field key.
pub const AREA_KEY: &str = "area_km2";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntityKind {
    Country,
    Region,
    City,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    pub kind: EntityKind,
    /// Stable, user-visible external id used to join CSV data (e.g. "FRA").
    pub code: String,
    pub name: String,
    #[serde(default)]
    pub short: String,
    pub color: [u8; 4],
    /// Parent (country of a region, ...) before any recorded change; stored
    /// under the old name, so earlier files load unchanged.
    #[serde(default, rename = "parent")]
    pub parent_base: Option<EntityId>,
    /// Parent changes: from that year on, the parent (None = top level).
    #[serde(default)]
    pub parent_changes: BTreeMap<i32, Option<EntityId>>,
    /// Parent in the selected year (latest when none), derived by
    /// [`Document::refresh_time_view`]; written only by the methods there.
    #[serde(skip)]
    pub parent: Option<EntityId>,
    #[serde(default)]
    pub capital: Option<EntityId>,
    /// The partition layer this entity belongs to.
    #[serde(default)]
    pub layer: LayerId,
    /// Location for point entities (cities).
    #[serde(default)]
    pub pos: Option<LonLat>,
    #[serde(default)]
    pub note: String,
    /// A label placed by hand (None: placed automatically).
    #[serde(default)]
    pub label: Option<LabelPlace>,
    /// Never show this entity's label.
    #[serde(default)]
    pub label_hidden: bool,
    /// First and last year it exists (inclusive); `None` is open. See
    /// [`crate::lifespan`].
    #[serde(default)]
    pub founded: Option<i32>,
    #[serde(default)]
    pub ended: Option<i32>,
    /// What came after it (for a country: the states that followed).
    #[serde(default)]
    pub successors: Vec<EntityId>,
}

/// A hand-placed label: where its centre is, its angle (degrees,
/// counter-clockwise) and a size factor on the automatic size. The same in
/// every year, like the rest of the layout.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LabelPlace {
    pub at: LonLat,
    #[serde(default)]
    pub angle: f32,
    #[serde(default = "one")]
    pub scale: f32,
}

fn one() -> f32 {
    1.0
}

impl LabelPlace {
    pub fn at(at: LonLat) -> Self {
        Self { at, angle: 0.0, scale: 1.0 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldKind {
    Number,
    Text,
    Bool,
    Category,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    /// Identifier used in expressions and CSV headers.
    pub key: String,
    pub name: String,
    pub kind: FieldKind,
    #[serde(default)]
    pub unit: String,
    /// Derived field expression, e.g. `population / area_km2`.
    #[serde(default)]
    pub expr: Option<String>,
    /// How a selected year without its own record is filled in; None = the
    /// default for the field's kind (see [`Inherit::default_for`]).
    #[serde(default)]
    pub inherit: Option<Inherit>,
    /// The year the undated base value actually holds for (e.g. "latest =
    /// 2020"); it then counts as a record of that year. None = the base
    /// value belongs to no year.
    #[serde(default)]
    pub base_year: Option<i32>,
}

impl FieldDef {
    pub fn policy(&self) -> Inherit {
        self.inherit.unwrap_or(Inherit::default_for(self.kind))
    }
}

/// How a field's value is found for a selected year that has no record of
/// its own. With no year selected the base value is used as it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Inherit {
    /// Only a record of exactly that year; otherwise unknown.
    Strict,
    /// The latest record at or before the year; otherwise unknown.
    Nearest,
    /// That year's record, otherwise the base value.
    BaseFallback,
    /// Numbers: linear between the records around the year; unknown
    /// outside them.
    Interpolate,
}

impl Inherit {
    pub const ALL: [Inherit; 4] = [Inherit::Strict, Inherit::Nearest, Inherit::BaseFallback, Inherit::Interpolate];

    /// Statistics (numbers) are strict; static attributes (text, category,
    /// yes/no) fall back to their base value.
    pub fn default_for(kind: FieldKind) -> Inherit {
        match kind {
            FieldKind::Number => Inherit::Strict,
            _ => Inherit::BaseFallback,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Inherit::Strict => "严格年份",
            Inherit::Nearest => "最近历史值",
            Inherit::BaseFallback => "基础值回退",
            Inherit::Interpolate => "线性插值",
        }
    }
}

/// Where a looked-up value comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// A record of exactly this year.
    Recorded(i32),
    /// The base value.
    Base,
    /// Carried over from an earlier year's record.
    Inherited(i32),
    /// Linear between the records of two years.
    Interpolated(i32, i32),
    /// Computed from other fields.
    Derived,
    /// No value.
    Unknown,
}

impl Source {
    /// For display, e.g. "1900 年记录".
    pub fn label(self) -> String {
        match self {
            Source::Recorded(y) => format!("{y} 年记录"),
            Source::Base => "基础值".into(),
            Source::Inherited(y) => format!("沿用 {y} 年"),
            Source::Interpolated(a, b) => format!("{a}–{b} 年插值"),
            Source::Derived => "计算值".into(),
            Source::Unknown => "未知".into(),
        }
    }

    /// For exports, e.g. "recorded:1900", "interpolated:1900-2020".
    pub fn tag(self) -> String {
        match self {
            Source::Recorded(y) => format!("recorded:{y}"),
            Source::Base => "base".into(),
            Source::Inherited(y) => format!("inherited:{y}"),
            Source::Interpolated(a, b) => format!("interpolated:{a}-{b}"),
            Source::Derived => "derived".into(),
            Source::Unknown => "unknown".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Value {
    Num(f64),
    Text(String),
    Bool(bool),
}

impl Value {
    pub fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Bool(b) => Some(*b as u8 as f64),
            Value::Text(t) => t.trim().parse().ok(),
        }
    }
    pub fn display(&self) -> String {
        match self {
            Value::Num(v) => format_num(*v),
            Value::Text(t) => t.clone(),
            Value::Bool(b) => if *b { "是" } else { "否" }.into(),
        }
    }
    pub fn parse(kind: FieldKind, s: &str) -> Option<Value> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        match kind {
            FieldKind::Number => s.replace(',', "").parse().ok().map(Value::Num),
            FieldKind::Bool => match s.to_lowercase().as_str() {
                "1" | "true" | "yes" | "是" | "y" => Some(Value::Bool(true)),
                "0" | "false" | "no" | "否" | "n" => Some(Value::Bool(false)),
                _ => None,
            },
            FieldKind::Text | FieldKind::Category => Some(Value::Text(s.to_string())),
        }
    }
}

pub fn format_num(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e15 {
        format!("{}", v as i64)
    } else if v.abs() >= 100.0 {
        format!("{v:.1}")
    } else {
        format!("{v:.4}").trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// A value with an undated base and per-year records. How a year is looked
/// up depends on the field ([`Series::resolve`]).

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub base: Option<Value>,
    #[serde(default)]
    pub by_year: BTreeMap<i32, Value>,
}

impl Series {
    /// The value for `year` under `policy`. A `base_year` makes the base a
    /// record of that year (a real record of the same year wins). With no
    /// year: the base value, or else the latest record.
    pub fn resolve(&self, year: Option<i32>, policy: Inherit, base_year: Option<i32>) -> (Option<Value>, Source) {
        let mut recs: BTreeMap<i32, (&Value, bool)> = self.by_year.iter().map(|(y, v)| (*y, (v, false))).collect();
        if let (Some(by), Some(b)) = (base_year, &self.base) {
            recs.entry(by).or_insert((b, true));
        }
        let Some(y) = year else {
            return match (&self.base, recs.iter().next_back()) {
                (Some(b), _) => (Some(b.clone()), Source::Base),
                (None, Some((ry, (v, _)))) => (Some((*v).clone()), Source::Inherited(*ry)),
                (None, None) => (None, Source::Unknown),
            };
        };
        if let Some((v, is_base)) = recs.get(&y) {
            return (Some((*v).clone()), if *is_base { Source::Base } else { Source::Recorded(y) });
        }
        let before = recs.range(..y).next_back();
        match policy {
            Inherit::Strict => (None, Source::Unknown),
            Inherit::Nearest => before.map_or((None, Source::Unknown), |(ry, (v, _))| (Some((*v).clone()), Source::Inherited(*ry))),
            Inherit::BaseFallback => self.base.as_ref().map_or((None, Source::Unknown), |b| (Some(b.clone()), Source::Base)),
            Inherit::Interpolate => {
                let after = recs.range(y + 1..).next();
                match (before, after) {
                    (Some((y0, (v0, _))), Some((y1, (v1, _)))) => match (v0.as_num(), v1.as_num()) {
                        (Some(a), Some(b)) => {
                            let t = (y - y0) as f64 / (y1 - y0) as f64;
                            (Some(Value::Num(a + (b - a) * t)), Source::Interpolated(*y0, *y1))
                        }
                        // not numbers: carry the earlier record
                        _ => (Some((*v0).clone()), Source::Inherited(*y0)),
                    },
                    _ => (None, Source::Unknown),
                }
            }
        }
    }
}

/// A piece of free text placed on the map: its centre, angle (degrees,
/// counter-clockwise), font size in pixels at 1× export scale, colour,
/// and letter spacing in ems (wide spacing suits seas and ranges).
/// `bend` curves the line of text into an arc spanning that many degrees:
/// positive arches up, negative sags, 0 is straight.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    pub text: String,
    pub at: LonLat,
    #[serde(default)]
    pub angle: f32,
    pub size: f32,
    pub color: [u8; 4],
    #[serde(default)]
    pub spacing: f32,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub bend: f32,
    /// A curve the text follows (a ridge, a coast); with two or more
    /// points the glyphs sit along it and `angle`/`bend` are not used.
    /// `at` is then the curve's middle, where the text is grabbed.
    #[serde(default)]
    pub path: Vec<LonLat>,
    /// Hidden: not drawn, exported or put in the legend, but kept (and
    /// listed in the 要素 tab, where it is shown again).
    #[serde(default)]
    pub hidden: bool,
    /// The font (a family name; none: the default font), and its bold face.
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub bold: bool,
}

impl Annotation {
    /// `size` is in pixels when the whole world is 1600 px wide; the text
    /// grows and shrinks with the map like the place it names.
    pub const REF_PX_PER_DEG: f64 = 1600.0 / 360.0;

    /// New text in the default sea-name look.
    pub fn new(text: &str, at: LonLat) -> Self {
        Self { text: text.into(), at, angle: 0.0, size: 16.0, color: [40, 70, 110, 255], spacing: 0.3, italic: true, bend: 0.0, path: Vec::new(), hidden: false, font: None, bold: false }
    }

    /// The text as drawn, with its letter spacing (spaces between characters).
    pub fn spaced(&self) -> String {
        let n = (self.spacing * 2.0).round().clamp(0.0, 6.0) as usize;
        if n == 0 {
            return self.text.clone();
        }
        let gap = " ".repeat(n);
        self.text.chars().map(|c| c.to_string()).collect::<Vec<_>>().join(&gap)
    }

    /// Whether the text follows an arc rather than a straight line.
    pub fn curved(&self) -> bool {
        self.bend.abs() >= 0.5
    }

    /// Whether the text follows a drawn curve.
    pub fn on_path(&self) -> bool {
        self.path.len() >= 2
    }

    /// Lay the text along `path` (fewer than two points: back to a straight
    /// or bent line where it was); it is then grabbed at the curve's middle.
    pub fn set_path(&mut self, path: Vec<LonLat>) {
        if path.len() >= 2 {
            self.at = path_middle(&path);
        }
        self.path = if path.len() >= 2 { path } else { Vec::new() };
    }

    /// Move the text so its grab point is at `to`; a curve it follows turns
    /// with it on the sphere, keeping its true shape.
    pub fn move_to(&mut self, to: LonLat) {
        if self.on_path() {
            self.path = crate::copy::moved(&self.path, self.at, to);
        }
        self.at = to;
    }
}

/// The point halfway along a path, measured along great circles.
pub(crate) fn path_middle(path: &[LonLat]) -> LonLat {
    let lens: Vec<f64> = path.windows(2).map(|w| crate::measure::great_circle(w[0], w[1], 1.0)).collect();
    let mut left = lens.iter().sum::<f64>() * 0.5;
    for (w, l) in path.windows(2).zip(&lens) {
        if left <= *l && *l > 0.0 {
            let pts = crate::measure::great_circle_path(w[0], w[1], 0.05);
            let i = ((left / l) * (pts.len() - 1) as f64).round() as usize;
            return pts[i.min(pts.len() - 1)];
        }
        left -= l;
    }
    path[path.len() / 2]
}

/// Where each glyph goes when a line of text follows an arc. `advances`
/// are the glyph widths in order; `bend` is the angle the arc spans in
/// degrees (positive arches up). Returns each glyph's centre relative to
/// the text's centre (x right, y up, before the text's own angle) and its
/// turn in degrees, counter-clockwise. The arc keeps the text's length and
/// is centred on its middle height, so the text stays where it was put.
pub fn arc_layout(advances: &[f32], bend: f32) -> Vec<([f32; 2], f32)> {
    let total: f32 = advances.iter().sum();
    let mut s = 0.0;
    let mids: Vec<f32> = advances
        .iter()
        .map(|a| {
            let m = s + a * 0.5 - total * 0.5;
            s += a;
            m
        })
        .collect();
    if bend.abs() < 0.5 || total <= 0.0 {
        return mids.into_iter().map(|m| ([m, 0.0], 0.0)).collect();
    }
    let th = bend.to_radians();
    let r = total / th;
    // how far the ends drop below the middle (negative: rise above it)
    let sag = r * (1.0 - (th * 0.5).cos());
    mids.into_iter()
        .map(|m| {
            let t = m / r;
            ([r * t.sin(), r * (t.cos() - 1.0) + sag * 0.5], -t.to_degrees())
        })
        .collect()
}

/// A free line on the map: a river, or with a style a route (road, trade
/// route, campaign...). Both are layout, the same in every year.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineFeature {
    pub name: String,
    pub pts: Vec<LonLat>,
    #[serde(default)]
    pub route: Option<RouteStyle>,
    /// What a route stands for in the export's legend ("进军路线"...);
    /// empty keeps it out. Routes with the same text and style share a row.
    #[serde(default)]
    pub legend: String,
    /// Write the name along the line (`path_text::along_path`).
    #[serde(default)]
    pub show_name: bool,
    /// Hidden: not drawn, exported or put in the legend, but kept (and
    /// listed in the 要素 tab, where it is shown again).
    #[serde(default)]
    pub hidden: bool,
}

impl LineFeature {
    /// The colour its name is written in: a route's own colour, or the
    /// map's river colour darkened so the text reads.
    pub fn name_color(&self, river: [u8; 4]) -> [u8; 4] {
        match self.route {
            Some(st) => st.color,
            None => [(river[0] as f32 * 0.6) as u8, (river[1] as f32 * 0.6) as u8, (river[2] as f32 * 0.6) as u8, 255],
        }
    }

    /// What it is called in messages and panels.
    pub fn kind_name(&self) -> &'static str {
        if self.route.is_some() {
            "线路"
        } else {
            "河流"
        }
    }
}

/// How a route is drawn: colour, width in screen pixels (scaled with the
/// export like borders and rivers), dash pattern and an arrowhead at its end.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct RouteStyle {
    pub color: [u8; 4],
    pub width: f32,
    pub dash: Dash,
    pub arrow: bool,
    /// Drawn as a broad, widening band (an advance, a migration) instead of
    /// a line; `width` is then the band's width.
    #[serde(default)]
    pub broad: bool,
    /// A broad arrow drawn as its outline only (a retreat, a planned move);
    /// the outline then takes the dash pattern.
    #[serde(default)]
    pub hollow: bool,
    /// Marks along a (not broad) line: a front line's teeth, a railway's ties.
    #[serde(default)]
    pub deco: crate::deco::Deco,
    /// Teeth on the right of the drawing direction instead of the left.
    #[serde(default)]
    pub deco_flip: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dash {
    Solid,
    Dashed,
    Dotted,
}

impl Dash {
    pub const ALL: [Dash; 3] = [Dash::Solid, Dash::Dashed, Dash::Dotted];
    pub fn name(self) -> &'static str {
        match self {
            Dash::Solid => "实线",
            Dash::Dashed => "虚线",
            Dash::Dotted => "点线",
        }
    }
}

impl Default for RouteStyle {
    /// A campaign-route look: dark red, dashed, with an arrow.
    fn default() -> Self {
        Self { color: [170, 40, 40, 255], width: 2.5, dash: Dash::Dashed, arrow: true, broad: false, hollow: false, deco: crate::deco::Deco::None, deco_flip: false }
    }
}

impl RouteStyle {
    /// The edge of a broad arrow at export scale `k`: its width and dash
    /// (a hollow arrow's edge is heavier and takes the dash pattern).
    pub fn broad_edge(&self, k: f32) -> (f32, Option<[f32; 2]>) {
        if self.hollow {
            (2.0 * k, self.pattern(2.0 * k))
        } else {
            (1.2 * k, None)
        }
    }

    /// The fill of a broad arrow: the colour, a little see-through.
    pub fn broad_fill(&self) -> [u8; 4] {
        [self.color[0], self.color[1], self.color[2], (self.color[3] as f32 * 0.7) as u8]
    }

    /// Dash and gap lengths in pixels for a line `width` wide (round caps;
    /// a near-zero dash draws dots); `None` for a solid line.
    pub fn pattern(&self, width: f32) -> Option<[f32; 2]> {
        match self.dash {
            Dash::Solid => None,
            Dash::Dashed => Some([(width * 4.0).max(6.0), (width * 2.5).max(4.0) + width]),
            Dash::Dotted => Some([0.01, (width * 2.5).max(4.0)]),
        }
    }
}

/// An arrowhead at the end of a screen polyline `width` wide: the line cut
/// back so its caps stay under the head, and the head (tip, two barbs).
/// `None` when the line is too short to carry one.
pub fn arrow_end(pts: &[[f32; 2]], width: f32) -> Option<(Vec<[f32; 2]>, [[f32; 2]; 3])> {
    let (len, half) = (width * 3.0 + 7.0, width * 1.8 + 4.0);
    let tip = *pts.last()?;
    let dist = |a: [f32; 2], b: [f32; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
    // walk back from the tip to the point one head length away
    let mut left = len;
    for i in (0..pts.len().saturating_sub(1)).rev() {
        let d = dist(pts[i], pts[i + 1]);
        if d >= left && d > 0.0 {
            let t = left / d;
            let base = [pts[i + 1][0] + (pts[i][0] - pts[i + 1][0]) * t, pts[i + 1][1] + (pts[i][1] - pts[i + 1][1]) * t];
            let dir = [(tip[0] - base[0]) / len, (tip[1] - base[1]) / len];
            let n = [-dir[1], dir[0]];
            let head = [tip, [base[0] + n[0] * half, base[1] + n[1] * half], [base[0] - n[0] * half, base[1] - n[1] * half]];
            // the line ends a little inside the head
            let mut line = pts[..=i].to_vec();
            line.push([base[0] + dir[0] * len * 0.4, base[1] + dir[1] * len * 0.4]);
            return Some((line, head));
        }
        left -= d;
    }
    None
}

/// What moving a city did to the country it belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CityMove {
    /// Still in the same country.
    Same,
    /// It now belongs to this country (or to none).
    Parent(Option<EntityId>),
    /// It stands in `here`'s land now, but its parent history was kept.
    Kept { here: Option<EntityId> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    pub planet_radius_km: f64,
    #[serde(default)]
    pub description: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Style {
    pub ocean: [u8; 4],
    pub land: [u8; 4],
    pub lake: [u8; 4],
    pub coast: [u8; 4],
    pub coast_width: f32,
    pub border: [u8; 4],
    pub border_width: f32,
    pub internal: [u8; 4],
    pub internal_width: f32,
    /// Hide mesh seams between faces with the same owner in exported maps.
    #[serde(default)]
    pub hide_face_seams_in_export: bool,
    pub river: [u8; 4],
    pub river_width: f32,
    pub label: [u8; 4],
    /// The font country names are written in (a family name; none: the
    /// default font), and whether in its bold face.
    #[serde(default)]
    pub label_font: Option<String>,
    #[serde(default)]
    pub label_bold: bool,
    /// Use different faces and sizes for countries, provinces and lower tiers.
    #[serde(default)]
    pub admin_typography: bool,
    pub show_labels: bool,
    pub show_cities: bool,
    /// Another partition layer drawn as boundary lines over the active
    /// one (e.g. actual-control lines over political colours).
    #[serde(default)]
    pub overlay_layer: Option<LayerId>,
    #[serde(default = "default_overlay_color")]
    pub overlay_color: [u8; 4],
    #[serde(default = "default_overlay_width")]
    pub overlay_width: f32,
    /// Country colours mixed with a tone (0: as they are) — how a map
    /// theme makes them pastel, sepia or dark. Political maps only.
    #[serde(default = "default_tone")]
    pub tone: [u8; 4],
    #[serde(default)]
    pub tone_mix: f32,
    /// Bands of this colour in the sea along the coasts, as wide as this
    /// in pixels (0: none).
    #[serde(default = "default_glow")]
    pub coast_glow: [u8; 4],
    #[serde(default)]
    pub coast_glow_width: f32,
    /// How strongly the terrain's hill shading shows (0: not at all).
    #[serde(default = "default_hillshade")]
    pub hillshade: f32,
}

fn default_hillshade() -> f32 {
    0.6
}

fn default_tone() -> [u8; 4] {
    [255, 255, 255, 255]
}

fn default_glow() -> [u8; 4] {
    [120, 160, 190, 255]
}

impl Style {
    /// A colour mixed with the tone.
    pub fn toned(&self, c: [u8; 4]) -> [u8; 4] {
        let k = self.tone_mix.clamp(0.0, 1.0);
        if k <= 0.0 {
            return c;
        }
        let m = |a: u8, b: u8| (a as f32 * (1.0 - k) + b as f32 * k).round() as u8;
        [m(c[0], self.tone[0]), m(c[1], self.tone[1]), m(c[2], self.tone[2]), c[3]]
    }

    /// The colours of the three coast bands, outermost first: the glow
    /// colour mixed into the sea more and more towards the coast.
    pub fn glow_bands(&self) -> [[u8; 4]; 3] {
        let m = |k: f32| {
            let f = |a: u8, b: u8| (a as f32 * (1.0 - k) + b as f32 * k).round() as u8;
            [f(self.ocean[0], self.coast_glow[0]), f(self.ocean[1], self.coast_glow[1]), f(self.ocean[2], self.coast_glow[2]), 255]
        };
        [m(0.35), m(0.65), m(1.0)]
    }
}

/// A halo that sets text of colour `c` apart: white behind dark text,
/// near black behind light text.
pub fn halo_for(c: [u8; 4]) -> [u8; 4] {
    let lum = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    if lum > 150.0 {
        [20, 22, 28, 255]
    } else {
        [255, 255, 255, 255]
    }
}

fn default_overlay_color() -> [u8; 4] {
    [200, 30, 40, 235]
}

fn default_overlay_width() -> f32 {
    2.2
}

impl Default for Style {
    fn default() -> Self {
        Self {
            ocean: [168, 200, 225, 255],
            land: [236, 230, 214, 255],
            lake: [168, 200, 225, 255],
            coast: [70, 100, 130, 255],
            coast_width: 1.2,
            border: [60, 60, 60, 255],
            border_width: 1.4,
            internal: [120, 120, 120, 160],
            internal_width: 0.6,
            hide_face_seams_in_export: false,
            river: [90, 140, 200, 255],
            river_width: 1.2,
            label: [30, 30, 30, 255],
            label_font: None,
            label_bold: false,
            admin_typography: false,
            show_labels: true,
            show_cities: true,
            overlay_layer: None,
            overlay_color: default_overlay_color(),
            overlay_width: default_overlay_width(),
            tone: default_tone(),
            tone_mix: 0.0,
            coast_glow: default_glow(),
            coast_glow_width: 0.0,
            hillshade: default_hillshade(),
        }
    }
}

/// Starting orientation for a geographic project and its globe.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct HomeView {
    pub projection: crate::projection::Projection,
    pub central_meridian: f64,
    pub globe_latitude: f64,
    /// Full longitude and latitude span in degrees for a regional opening view.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flat_extent: Option<[f64; 2]>,
}

impl HomeView {
    pub fn valid(self) -> bool {
        (-180.0..=180.0).contains(&self.central_meridian)
            && (-89.0..=89.0).contains(&self.globe_latitude)
            && self.flat_extent.is_none_or(|[lon, lat]| {
                (0.01..=360.0).contains(&lon) && (0.01..=180.0).contains(&lat)
            })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home_view: Option<HomeView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<crate::guides::Guide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paper: Option<crate::paper::Paper>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diplomacy: Option<std::sync::Arc<crate::diplomacy::Diplomacy>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub book: Option<std::sync::Arc<crate::world_book::WorldBook>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub climate: Option<std::sync::Arc<crate::climate::Project>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<std::sync::Arc<crate::environment::Project>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visualization: Option<std::sync::Arc<crate::visualization::Project>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub star_chart: Option<std::sync::Arc<crate::star_chart::StarChart>>,
    pub format_version: u32,
    pub meta: Meta,
    pub topo: Topology,
    pub entities: OrdMap<EntityId, Entity>,
    /// Ownership of the *active* layer before any recorded change ("from
    /// the start"). Stored under the old name, so earlier files (which only
    /// have the political layer) load unchanged.
    #[serde(rename = "owner")]
    pub owner_base: OrdMap<FaceId, EntityId>,
    /// Ownership changes per face: from that year on, the owner (None =
    /// unclaimed), until the next change. See ADR-7.
    #[serde(default)]
    pub owner_changes: OrdMap<FaceId, OwnerChanges>,
    /// Ownership in the selected year (the latest state when no year is
    /// selected), derived from the two fields above by
    /// [`Document::refresh_owner`]. Everything that draws or measures the
    /// map reads this; only the ownership methods below write it.
    #[serde(skip)]
    pub owner: OrdMap<FaceId, EntityId>,
    /// Land/water of faces whose kind changes over time (coastlines that
    /// move): kind from the start, and change points. For these faces the
    /// topology's `kind` is the value in the selected year (latest when
    /// none), set by [`Document::refresh_time_view`]. Faces without an
    /// entry have a fixed kind.
    #[serde(default)]
    pub kind_base: OrdMap<FaceId, FaceKind>,
    #[serde(default)]
    pub kind_changes: OrdMap<FaceId, BTreeMap<i32, FaceKind>>,
    /// Partition layers by id (the political layer may be absent: it then
    /// has its default name). See [`Layer`].
    #[serde(default)]
    pub layers: OrdMap<LayerId, Layer>,
    /// The layer being viewed and edited; `owner_*` hold its ownership.
    #[serde(default)]
    pub active_layer: LayerId,
    pub fields: OrdMap<FieldId, FieldDef>,
    pub values: OrdMap<(EntityId, FieldId), Series>,
    #[serde(default)]
    pub rivers: OrdMap<LineId, LineFeature>,
    /// Free text on the map (seas, ranges, regions...): layout, the same in every year.
    #[serde(default)]
    pub annotations: OrdMap<LineId, Annotation>,
    /// Point symbols (peaks, battles, ports...): layout, each with an
    /// optional year range.
    #[serde(default)]
    pub symbols: OrdMap<LineId, crate::symbol::MapSymbol>,
    /// Hatched areas (disputed, occupied, claimed...): layout over the
    /// map, never territory; each with an optional year range.
    #[serde(default)]
    pub areas: OrdMap<LineId, crate::area::AreaMark>,
    pub style: Style,
    pub theme: ThemeSpec,
    /// Current year for time-series lookups (None = base values).
    #[serde(default)]
    pub year: Option<i32>,
    /// Reference images drawn under the map, bottom first.
    #[serde(default)]
    pub underlays: Vec<crate::underlay::Underlay>,
    /// Saved export layouts, in the order they were made.
    #[serde(default)]
    pub layouts: Vec<crate::layout::Layout>,
    /// Drawn features that are locked: shown, but not picked or moved on
    /// the map (see [`crate::arrange`]).
    #[serde(default)]
    pub locked: std::collections::BTreeSet<LineId>,
    /// Layers of drawn features, from the bottom of the stack up, and the
    /// one each feature is in (see [`crate::stack`]).
    #[serde(default)]
    pub groups: Vec<crate::stack::FeatureGroup>,
    #[serde(default)]
    pub group_of: std::collections::BTreeMap<LineId, crate::stack::GroupId>,
    /// Rivers, routes and hatched areas drawn with curves: the curve, by
    /// feature id (its points are the flattened curve; see [`crate::curve`]).
    #[serde(default)]
    pub curves: std::collections::BTreeMap<LineId, crate::curve::Curve>,
    /// Named styles of drawn features, the style each linked feature
    /// follows, and the palette (see [`crate::styles`]).
    #[serde(default)]
    pub styles: Vec<crate::styles::NamedStyle>,
    #[serde(default)]
    pub style_of: std::collections::BTreeMap<LineId, crate::styles::StyleId>,
    #[serde(default)]
    pub swatches: Vec<crate::styles::Swatch>,
    /// Local views: a region and a stretch of years to look at on their
    /// own, e.g. a war (see [`crate::focus`]).
    #[serde(default)]
    pub focuses: Vec<crate::focus::Focus>,
    /// How years are shown (an era of the world's own; plain years when off).
    #[serde(default)]
    pub calendar: crate::calendar::Calendar,
    /// Named events of the history (see [`crate::events`]).
    #[serde(default)]
    pub events: Vec<crate::events::Event>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_event_id: Option<u32>,
    /// Name styles given to units (a culture's, say): new names for the
    /// places a unit covers follow its style ([`Document::name_style_for`]).
    #[serde(default)]
    pub name_styles: OrdMap<EntityId, crate::names::NameStyle>,
    /// Elevation over the globe, shown as hill shading (see [`crate::terrain`]).
    #[serde(default, with = "crate::terrain::opt_arc", skip_serializing_if = "Option::is_none")]
    pub terrain: Option<std::sync::Arc<crate::terrain::Terrain>>,
}

/// Change points of one face's ownership: year → owner from that year on.
pub type OwnerChanges = BTreeMap<i32, Option<EntityId>>;

/// Owner at `year` given a base owner and change points; `None` year means
/// the latest state.
fn resolve(base: Option<EntityId>, changes: Option<&OwnerChanges>, year: Option<i32>) -> Option<EntityId> {
    let Some(ch) = changes else { return base };
    let last = match year {
        Some(y) => ch.range(..=y).next_back(),
        None => ch.iter().next_back(),
    };
    last.map_or(base, |(_, o)| *o)
}

/// Drop change points that repeat the state already in force. Only used
/// where identities change (merging or deleting entities); recorded change
/// points are otherwise kept, see [`insert_event`].
fn compress<T: PartialEq + Clone>(base: &T, ch: &mut BTreeMap<i32, T>) {
    let mut cur = base.clone();
    ch.retain(|_, o| {
        let keep = *o != cur;
        cur = o.clone();
        keep
    });
}

fn normalize(base: Option<EntityId>, ch: &mut OwnerChanges) {
    compress(&base, ch);
}

/// Record `v` from year `y` on. The point is dropped again only when it
/// repeats the state just before `y` (a no-op). Other change points are
/// events the author set and are kept even when they now repeat the state
/// before them: recording "B from 1920" must not erase an existing "B from
/// 1950", which matters once something is later inserted in between.
pub(crate) fn insert_event<T: PartialEq + Clone>(base: &T, ch: &mut BTreeMap<i32, T>, y: i32, v: T) {
    ch.insert(y, v.clone());
    let before = ch.range(..y).next_back().map_or_else(|| base.clone(), |(_, s)| s.clone());
    if before == v {
        ch.remove(&y);
    }
}

/// A planar (unwrapped lon/lat) polygon with holes.
#[derive(Clone, Debug, PartialEq)]
pub struct PlanarPolygon {
    pub outer: Vec<P2>,
    pub holes: Vec<Vec<P2>>,
}

pub(crate) const PALETTE: [[u8; 4]; 12] = [
    [231, 160, 140, 255],
    [150, 190, 140, 255],
    [240, 205, 120, 255],
    [160, 170, 215, 255],
    [215, 160, 200, 255],
    [140, 200, 200, 255],
    [225, 185, 150, 255],
    [190, 210, 120, 255],
    [200, 150, 150, 255],
    [170, 200, 230, 255],
    [230, 220, 150, 255],
    [180, 160, 210, 255],
];

impl Default for Document {
    fn default() -> Self {
        Self::new("未命名世界", 6371.0088)
    }
}

impl Document {
    pub fn new_paper(name: &str, size: crate::paper::Paper) -> Self {
        let mut doc = Self::new(name, 6371.0);
        doc.paper = Some(size);
        doc.style.ocean = [255; 4];
        doc.style.coast_glow_width = 0.0;
        doc
    }

    pub fn new(name: &str, radius_km: f64) -> Self {
        Self {
            format_version: FORMAT_VERSION,
            home_view: None,
            guides: Vec::new(),
            book: None,
            paper: None,
            diplomacy: None,
            climate: None,
            environment: None,
            visualization: None,
            star_chart: None,
            meta: Meta { name: name.into(), planet_radius_km: radius_km, description: String::new() },
            topo: Topology::default(),
            entities: OrdMap::new(),
            owner_base: OrdMap::new(),
            owner_changes: OrdMap::new(),
            owner: OrdMap::new(),
            kind_base: OrdMap::new(),
            kind_changes: OrdMap::new(),
            layers: OrdMap::new(),
            active_layer: POLITICAL,
            fields: OrdMap::new(),
            values: OrdMap::new(),
            rivers: OrdMap::new(),
            annotations: OrdMap::new(),
            symbols: OrdMap::new(),
            areas: OrdMap::new(),
            style: Style { admin_typography: true, ..Style::default() },
            theme: ThemeSpec::default(),
            year: None,
            underlays: Vec::new(),
            layouts: Vec::new(),
            locked: Default::default(),
            groups: Vec::new(),
            group_of: Default::default(),
            curves: Default::default(),
            styles: Vec::new(),
            style_of: Default::default(),
            swatches: Vec::new(),
            focuses: Vec::new(),
            calendar: Default::default(),
            events: Vec::new(),
            next_event_id: None,
            name_styles: OrdMap::new(),
            terrain: None,
        }
    }

    pub fn radius(&self) -> f64 {
        self.meta.planet_radius_km
    }

    // ------------------------------------------------------------ entities

    pub fn add_entity(&mut self, kind: EntityKind, name: &str) -> EntityId {
        let id = EntityId(self.topo.alloc());
        let n = self.entities.values().filter(|e| e.kind == kind).count();
        let prefix = match kind {
            EntityKind::Country => "C",
            EntityKind::Region => "R",
            EntityKind::City => "T",
        };
        let mut code = format!("{prefix}{:03}", n + 1);
        while self.entity_by_code(&code).is_some() {
            code.push('_');
        }
        self.entities.insert(
            id,
            Entity {
                kind,
                code,
                name: name.into(),
                short: String::new(),
                color: PALETTE[n % PALETTE.len()],
                parent_base: None,
                parent_changes: BTreeMap::new(),
                parent: None,
                capital: None,
                layer: self.active_layer,
                pos: None,
                label: None,
                label_hidden: false,
                note: String::new(),
                founded: None,
                ended: None,
                successors: Vec::new(),
            },
        );
        id
    }

    pub fn entity_by_code(&self, code: &str) -> Option<EntityId> {
        self.entities.iter().find(|(_, e)| e.code == code).map(|(id, _)| *id)
    }

    /// Top-level units ("countries") of the active layer.
    pub fn countries(&self) -> Vec<EntityId> {
        self.entities.iter().filter(|(_, e)| e.kind == EntityKind::Country && e.layer == self.active_layer).map(|(id, _)| *id).collect()
    }

    // ------------------------------------------------------ partition layers

    pub fn layer_name(&self, l: LayerId) -> String {
        match self.layers.get(&l) {
            Some(x) => x.name.clone(),
            None if l == POLITICAL => "政治归属".into(),
            None => format!("层 {}", l.0),
        }
    }

    /// All layers, political first.
    pub fn layer_ids(&self) -> Vec<LayerId> {
        let mut v: Vec<LayerId> = self.layers.keys().copied().collect();
        if !v.contains(&POLITICAL) {
            v.insert(0, POLITICAL);
        }
        v
    }

    /// Add an (empty) exclusive layer, e.g. "实际控制" or "文化区".
    pub fn add_layer(&mut self, name: &str) -> LayerId {
        self.add_layer_of(name, LayerKind::Exclusive)
    }

    /// Add an (empty) layer of the given kind.
    pub fn add_layer_of(&mut self, name: &str, kind: LayerKind) -> LayerId {
        let id = LayerId(self.layer_ids().iter().map(|l| l.0).max().unwrap_or(0) + 1);
        self.layers.insert(id, Layer { name: name.into(), kind, ..Default::default() });
        id
    }

    pub fn layer_kind(&self, l: LayerId) -> LayerKind {
        self.layers.get(&l).map_or(LayerKind::Exclusive, |x| x.kind)
    }

    /// The active layer is a share layer.
    pub fn active_is_shares(&self) -> bool {
        self.layer_kind(self.active_layer) == LayerKind::Shares
    }

    /// Shares of a face in layer `l` at the selected year (empty for
    /// exclusive layers).
    pub fn shares_in(&self, l: LayerId, f: FaceId) -> Shares {
        match self.layers.get(&l) {
            Some(x) if x.kind == LayerKind::Shares => x.shares.at(f, self.year),
            _ => Shares::new(),
        }
    }

    /// Shares of a face in the active layer at the selected year.
    pub fn shares(&self, f: FaceId) -> Shares {
        self.shares_in(self.active_layer, f)
    }

    /// The shares of a face that count in the selected year's view: none
    /// while the face is water, and none for units that do not exist then
    /// (their part is unassigned). The records ([`Document::shares_in`])
    /// are what editing reads and writes; they are not changed by this.
    pub fn shares_shown_in(&self, l: LayerId, f: FaceId) -> Shares {
        if !self.topo.faces.get(&f).is_some_and(|x| x.kind == FaceKind::Land) {
            return Shares::new();
        }
        let mut s = self.shares_in(l, f);
        s.retain(|e, _| self.exists_in(*e, self.year));
        s
    }

    /// [`Document::shares_shown_in`] for the active layer.
    pub fn shares_shown(&self, f: FaceId) -> Shares {
        self.shares_shown_in(self.active_layer, f)
    }

    /// Share history of a face in the active share layer.
    pub fn share_history(&self, f: FaceId) -> (Shares, Vec<(i32, Shares)>) {
        let st = self.layers.get(&self.active_layer).map(|x| x.shares.history(f)).unwrap_or_default();
        (st.0, st.1.into_iter().collect())
    }

    /// Set unit `e`'s share of face `f` in the active share layer (others
    /// scaled down to fit), or clear the face's shares when `e` is None.
    /// Same year rules as [`Document::record_owner`].
    pub fn set_share(&mut self, f: FaceId, e: Option<EntityId>, bp: u16) -> TResult<()> {
        if !self.active_is_shares() {
            return Err("当前层不是比例层".into());
        }
        if !self.topo.faces.get(&f).is_some_and(|x| x.kind == FaceKind::Land) {
            return Err("只能设置陆地的比例".into());
        }
        if let Some(e) = e {
            if self.entities.get(&e).map(|x| x.layer) != Some(self.active_layer) {
                return Err("单位不属于当前层".into());
            }
        }
        let l = self.active_layer;
        let mut layer = self.layers[&l].clone();
        let cur = layer.shares.at(f, self.year);
        let new = match e {
            Some(e) => rebalance(&cur, e, bp),
            None => Shares::new(),
        };
        if new == cur {
            return Ok(());
        }
        let (base, mut ch) = layer.shares.history(f);
        match self.year {
            Some(y) => {
                insert_event(&base, &mut ch, y, new);
                layer.shares.set(f, base, ch);
            }
            None if ch.is_empty() => layer.shares.set(f, new, ch),
            None => return Err("这个面的比例随年份变化，请先选择年份再修改".into()),
        }
        self.layers.insert(l, layer);
        self.refresh_face_owner(f);
        Ok(())
    }

    /// Apply `f` to the share storage of every share layer.
    fn each_share_layer(&mut self, f: impl Fn(&mut ShareStore)) {
        let ids: Vec<LayerId> = self.layers.iter().filter(|(_, x)| x.kind == LayerKind::Shares).map(|(l, _)| *l).collect();
        for l in ids {
            let mut x = self.layers[&l].clone();
            f(&mut x.shares);
            self.layers.insert(l, x);
        }
    }

    pub fn rename_layer(&mut self, l: LayerId, name: &str) -> TResult<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err("分区层名称不能为空".into());
        }
        self.layers.get_mut(&l).ok_or("分区层不存在")?.name = name.into();
        Ok(())
    }

    /// Delete a layer with its entities (not the political layer).
    pub fn delete_layer(&mut self, l: LayerId) -> TResult<()> {
        if l == POLITICAL {
            return Err("政治归属层不能删除".into());
        }
        if !self.layers.contains_key(&l) {
            return Err("层不存在".into());
        }
        if self.active_layer == l {
            self.set_active_layer(POLITICAL)?;
        }
        let ents: Vec<EntityId> = self.entities.iter().filter(|(_, e)| e.layer == l).map(|(id, _)| *id).collect();
        for e in ents {
            self.delete_entity(e);
        }
        self.layers.remove(&l);
        Ok(())
    }

    /// View and edit another layer: its ownership becomes `owner_*` (and
    /// `owner`), the current one is parked.
    pub fn set_active_layer(&mut self, l: LayerId) -> TResult<()> {
        if l == self.active_layer {
            return Ok(());
        }
        if l != POLITICAL && !self.layers.contains_key(&l) {
            return Err("层不存在".into());
        }
        let cur = Ownership { base: std::mem::take(&mut self.owner_base), changes: std::mem::take(&mut self.owner_changes) };
        let mut park = self.layers.get(&self.active_layer).cloned().unwrap_or_else(|| Layer { name: self.layer_name(self.active_layer), ..Default::default() });
        park.parked = cur;
        self.layers.insert(self.active_layer, park);
        let mut next = self.layers.get(&l).cloned().unwrap_or_default();
        let own = std::mem::take(&mut next.parked);
        if l == POLITICAL && next.name.is_empty() {
            next.name = self.layer_name(POLITICAL);
        }
        self.layers.insert(l, next);
        self.owner_base = own.base;
        self.owner_changes = own.changes;
        self.active_layer = l;
        self.refresh_owner();
        Ok(())
    }

    /// Owners of every face in layer `l` at the selected year.
    pub fn layer_owner(&self, l: LayerId) -> OrdMap<FaceId, EntityId> {
        if l == self.active_layer {
            return self.owner.clone();
        }
        let Some(x) = self.layers.get(&l) else { return OrdMap::new() };
        if x.kind == LayerKind::Shares {
            let st = &x.shares;
            let faces: HashSet<FaceId> = st.base.keys().chain(st.changes.keys()).copied().collect();
            return faces.into_iter().filter_map(|f| dominant(&st.at(f, self.year)).map(|e| (f, e))).collect();
        }
        let o = &x.parked;
        let mut out = o.base.clone();
        for (f, ch) in o.changes.iter() {
            match resolve(o.base.get(f).copied(), Some(ch), self.year) {
                Some(e) => out.insert(*f, e),
                None => out.remove(f),
            };
        }
        out
    }

    /// Edges where layer `l` has a boundary between land faces at the
    /// selected year: the two sides belong to different top-level units
    /// (or one side to none).
    pub fn layer_border_edges(&self, l: LayerId) -> Vec<crate::topo::EdgeId> {
        let own = self.layer_owner(l);
        let land = |f: &FaceId| self.topo.faces.get(f).is_some_and(|x| x.kind == FaceKind::Land);
        let top = |f: &FaceId| own.get(f).map(|e| self.top_ancestor(*e));
        self.topo.edges.iter().filter(|(_, e)| land(&e.left) && land(&e.right) && top(&e.left) != top(&e.right)).map(|(id, _)| *id).collect()
    }

    /// The overlay layer to draw, if it is set, exists and is not the
    /// active one.
    pub fn overlay_layer(&self) -> Option<LayerId> {
        self.style.overlay_layer.filter(|l| *l != self.active_layer && self.layer_ids().contains(l))
    }

    /// Apply `f` to the storage of every inactive layer.
    fn each_parked(&mut self, f: impl Fn(&mut Ownership)) {
        let ids: Vec<LayerId> = self.layers.keys().copied().filter(|l| *l != self.active_layer).collect();
        for l in ids {
            let mut x = self.layers[&l].clone();
            f(&mut x.parked);
            self.layers.insert(l, x);
        }
    }

    pub fn add_city(&mut self, pos: LonLat, name: &str) -> EntityId {
        let id = self.add_entity(EntityKind::City, name);
        let face = self.topo.face_at(pos);
        self.entities.get_mut(&id).unwrap().pos = Some(pos.normalized());
        let p = self.owner.get(&face).copied();
        // a new entity has no history, so this cannot fail
        let _ = self.record_parent(id, p);
        id
    }

    /// The city whose point is nearest to `p` within `tol` degrees.
    pub fn city_at(&self, p: LonLat, tol: f64) -> Option<EntityId> {
        self.city_at_where(p, tol, |_| true)
    }

    pub fn city_at_where(&self, p: LonLat, tol: f64, visible: impl Fn(EntityId) -> bool) -> Option<EntityId> {
        self.entities
            .iter()
            .filter_map(|(id, e)| match (e.kind, e.pos) {
                (EntityKind::City, Some(q)) if e.exists_in(self.year) && visible(*id) => Some((*id, geom::geo_dist_deg(p, q))),
                _ => None,
            })
            .filter(|(_, d)| *d <= tol)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(id, _)| id)
    }

    /// Move a city (its position is the same in every year). When it lands
    /// in another country and its parent has no recorded history and no
    /// year is selected, the parent follows; otherwise the parent history
    /// is left as it is. Returns the new parent when it changed, and the
    /// owner of the new place when the parent was left alone although it
    /// differs.
    pub fn move_city(&mut self, id: EntityId, pos: LonLat) -> TResult<CityMove> {
        let e = self.entities.get(&id).ok_or("城市不存在")?;
        if e.kind != EntityKind::City {
            return Err("不是城市".into());
        }
        let simple = e.parent_changes.is_empty() && self.year.is_none();
        let before = e.parent;
        self.entities.get_mut(&id).unwrap().pos = Some(pos.normalized());
        let face = self.topo.face_at(pos);
        let here = self.owner.get(&face).copied();
        if here == before {
            return Ok(CityMove::Same);
        }
        if simple {
            self.record_parent(id, here)?;
            return Ok(CityMove::Parent(here));
        }
        Ok(CityMove::Kept { here })
    }

    /// Assign a land face (the background too, when it is land) to an
    /// entity (None = unclaimed): from the selected year on, or for all
    /// years when none is selected (see [`Document::record_owner`]).
    pub fn assign(&mut self, face: FaceId, who: Option<EntityId>) -> TResult<()> {
        let f = self.topo.faces.get(&face).ok_or("面不存在")?;
        if f.kind != FaceKind::Land {
            return Err("只能分配陆地面".into());
        }
        self.record_owner(face, who)
    }

    // ----------------------------------------------------- ownership in time

    /// Owner of a face at a year (None = latest state).
    pub fn owner_at(&self, f: FaceId, year: Option<i32>) -> Option<EntityId> {
        if self.active_is_shares() {
            return self.layers.get(&self.active_layer).and_then(|x| dominant(&x.shares.at(f, year)));
        }
        resolve(self.owner_base.get(&f).copied(), self.owner_changes.get(&f), year)
    }

    /// Recompute everything derived from the selected year: ownership
    /// ([`Document::owner`]) and parents ([`Entity::parent`]).
    pub fn refresh_time_view(&mut self) {
        self.refresh_kinds();
        self.refresh_owner();
        let ids: Vec<EntityId> = self.entities.iter().filter(|(_, e)| !e.parent_changes.is_empty() || e.parent != e.parent_base || e.parent_base.is_some()).map(|(id, _)| *id).collect();
        for id in ids {
            self.refresh_parent(id);
        }
    }

    fn refresh_parent(&mut self, e: EntityId) {
        let Some(x) = self.entities.get(&e) else { return };
        // no parent that does not exist in the year
        let p = resolve(x.parent_base, Some(&x.parent_changes), self.year).filter(|p| self.entities.get(p).is_some_and(|q| q.exists_in(self.year)));
        if x.parent != p {
            self.entities.get_mut(&e).unwrap().parent = p;
        }
    }

    pub fn parent_at(&self, e: EntityId, year: Option<i32>) -> Option<EntityId> {
        self.entities.get(&e).and_then(|x| resolve(x.parent_base, Some(&x.parent_changes), year))
    }

    fn set_parent_history(&mut self, e: EntityId, base: Option<EntityId>, ch: OwnerChanges) {
        let x = self.entities.get_mut(&e).unwrap();
        x.parent_base = base;
        x.parent_changes = ch;
        self.refresh_parent(e);
    }

    /// Record that `e` is under `p` (None = top level), with the same year
    /// rules as [`Document::record_owner`]. Refused if it would make an
    /// entity its own ancestor in any year from then on.
    pub fn record_parent(&mut self, e: EntityId, p: Option<EntityId>) -> TResult<()> {
        let x = self.entities.get(&e).ok_or("实体不存在")?;
        if p.is_some_and(|p| !self.entities.contains_key(&p)) {
            return Err("上级不存在".into());
        }
        if p.is_some_and(|p| self.entities[&p].layer != x.layer) {
            return Err("上级必须属于同一层".into());
        }
        if resolve(x.parent_base, Some(&x.parent_changes), self.year) == p {
            return Ok(());
        }
        let (base, mut ch) = (x.parent_base, x.parent_changes.clone());
        let before = self.entities.clone();
        match self.year {
            Some(y) => {
                insert_event(&base, &mut ch, y, p);
                self.set_parent_history(e, base, ch);
            }
            None if ch.is_empty() => self.set_parent_history(e, p, ch),
            None => return Err("上级关系随年份变化，请先选择年份再修改".into()),
        }
        // The hierarchy was acyclic before, so a new cycle must run through
        // `e`: walk up from `e` in every year its new parent is in force,
        // i.e. at the edit year and at every change year (of any entity)
        // before `e`'s next own change.
        let from = self.year.unwrap_or(i32::MIN);
        let until = self.year.and_then(|y| self.entities[&e].parent_changes.range(y + 1..).next().map(|(ny, _)| *ny)).unwrap_or(i32::MAX);
        let mut years: Vec<Option<i32>> = vec![self.year];
        if self.year.is_none() {
            years.push(Some(i32::MIN));
        }
        if until == i32::MAX {
            years.push(None);
        }
        years.extend(self.entities.values().flat_map(|x| x.parent_changes.keys()).filter(|y| **y >= from && **y < until).map(|y| Some(*y)));
        years.sort();
        years.dedup();
        for y in years {
            if self.reaches_itself(e, y) {
                self.entities = before;
                return Err("这会使一个实体成为自己的上级".into());
            }
        }
        Ok(())
    }

    /// Whether following parents up from `e` in `year` comes back to `e`.
    fn reaches_itself(&self, e: EntityId, year: Option<i32>) -> bool {
        let mut cur = e;
        for _ in 0..=self.entities.len() {
            match self.parent_at(cur, year) {
                Some(p) if p == e => return true,
                Some(p) => cur = p,
                None => return false,
            }
        }
        true
    }

    /// Parent history of an entity: base and change points.
    pub fn parent_history(&self, e: EntityId) -> (Option<EntityId>, Vec<(i32, Option<EntityId>)>) {
        self.entities.get(&e).map(|x| (x.parent_base, x.parent_changes.iter().map(|(y, p)| (*y, *p)).collect())).unwrap_or_default()
    }

    /// Recompute [`Document::owner`] for the selected year (for a share
    /// layer: each face's largest share). Faces that are water in that year
    /// have no owner (their owners in other years are kept).
    pub fn refresh_owner(&mut self) {
        let mut o = if self.active_is_shares() {
            let st = &self.layers[&self.active_layer].shares;
            let faces: HashSet<FaceId> = st.base.keys().chain(st.changes.keys()).copied().collect();
            faces.into_iter().filter_map(|f| dominant(&self.shares_shown(f)).map(|e| (f, e))).collect()
        } else {
            let mut o = self.owner_base.clone();
            for (f, ch) in self.owner_changes.iter() {
                match resolve(self.owner_base.get(f).copied(), Some(ch), self.year) {
                    Some(e) => o.insert(*f, e),
                    None => o.remove(f),
                };
            }
            o
        };
        // water has no owner, whether the face is water in every year or only
        // in this one (as in `refresh_face_owner`)
        let wet: Vec<FaceId> = o.keys().filter(|f| self.topo.faces.get(*f).is_none_or(|x| x.kind == FaceKind::Water)).copied().collect();
        for f in wet {
            o.remove(&f);
        }
        // an entity owns nothing in a year it does not exist (records kept)
        let gone: HashSet<EntityId> = self.entities.iter().filter(|(_, e)| !e.exists_in(self.year)).map(|(id, _)| *id).collect();
        if !gone.is_empty() {
            let drop: Vec<FaceId> = o.iter().filter(|(_, e)| gone.contains(e)).map(|(f, _)| *f).collect();
            for f in drop {
                o.remove(&f);
            }
        }
        self.owner = o;
    }

    // ------------------------------------------------- land/water in time

    /// Land or water of a face in a year (None = latest).
    pub fn face_kind_at(&self, f: FaceId, year: Option<i32>) -> Option<FaceKind> {
        let face = self.topo.faces.get(&f)?;
        let Some(ch) = self.kind_changes.get(&f) else { return Some(face.kind) };
        let base = self.kind_base.get(&f).copied().unwrap_or(face.kind);
        let last = match year {
            Some(y) => ch.range(..=y).next_back(),
            None => ch.iter().next_back(),
        };
        Some(last.map_or(base, |(_, k)| *k))
    }

    /// Set every face with a kind history to its kind in the selected year.
    fn refresh_kinds(&mut self) {
        let ids: Vec<FaceId> = self.kind_changes.keys().copied().collect();
        for f in ids {
            self.apply_kind(f);
        }
    }

    fn apply_kind(&mut self, f: FaceId) {
        let Some(k) = self.face_kind_at(f, self.year) else { return };
        if self.topo.faces[&f].kind != k {
            let mut face = self.topo.faces[&f].clone();
            face.kind = k;
            self.topo.faces.insert(f, face);
            // new revision: fills and coast lines are rebuilt
            self.topo.touch(f);
        }
    }

    fn set_kind_history(&mut self, f: FaceId, base: FaceKind, ch: BTreeMap<i32, FaceKind>) {
        if ch.is_empty() {
            self.kind_base.remove(&f);
            self.kind_changes.remove(&f);
            // a fixed kind is the face's own
            if self.topo.faces[&f].kind != base {
                let mut face = self.topo.faces[&f].clone();
                face.kind = base;
                self.topo.faces.insert(f, face);
                self.topo.touch(f);
            }
        } else {
            self.kind_base.insert(f, base);
            self.kind_changes.insert(f, ch);
            self.apply_kind(f);
        }
        self.refresh_face_owner(f);
    }

    /// Land/water history of a face: kind from the start and change points.
    pub fn kind_history(&self, f: FaceId) -> (FaceKind, Vec<(i32, FaceKind)>) {
        let k = self.topo.faces.get(&f).map_or(FaceKind::Water, |x| x.kind);
        match self.kind_changes.get(&f) {
            Some(ch) => (self.kind_base.get(&f).copied().unwrap_or(k), ch.iter().map(|(y, k)| (*y, *k)).collect()),
            None => (k, vec![]),
        }
    }

    /// Make a face land or water from the selected year on (until its next
    /// change), or for all years when none is selected (refused when it
    /// has a history). The background keeps its fixed kind.
    pub fn record_kind(&mut self, f: FaceId, kind: FaceKind) -> TResult<()> {
        if f == BACKGROUND {
            return Err("背景的海陆在“样式/世界”中设置，不随年份变化".into());
        }
        if !self.topo.faces.contains_key(&f) {
            return Err("面不存在".into());
        }
        if self.face_kind_at(f, self.year) == Some(kind) {
            return Ok(());
        }
        let (base, ch) = self.kind_history(f);
        let mut ch: BTreeMap<i32, FaceKind> = ch.into_iter().collect();
        match self.year {
            Some(y) => {
                insert_event(&base, &mut ch, y, kind);
                self.set_kind_history(f, base, ch);
            }
            None if ch.is_empty() => self.set_kind_history(f, kind, ch),
            None => return Err("这个面的海陆随年份变化，请先选择年份再修改".into()),
        }
        Ok(())
    }

    pub fn set_year(&mut self, year: Option<i32>) {
        self.year = year;
        self.refresh_time_view();
    }

    /// One face of [`Document::refresh_owner`]: the same rules, so the view
    /// after an edit equals a full refresh.
    fn refresh_face_owner(&mut self, f: FaceId) {
        let water = self.topo.faces.get(&f).is_some_and(|x| x.kind == FaceKind::Water);
        let shown = if self.active_is_shares() { dominant(&self.shares_shown(f)) } else { self.owner_at(f, self.year).filter(|_| !water).filter(|e| self.exists_in(*e, self.year)) };
        match shown {
            Some(e) => self.owner.insert(f, e),
            None => self.owner.remove(&f),
        };
    }

    pub(crate) fn set_history(&mut self, f: FaceId, base: Option<EntityId>, ch: OwnerChanges) {
        match base {
            Some(e) => self.owner_base.insert(f, e),
            None => self.owner_base.remove(&f),
        };
        if ch.is_empty() {
            self.owner_changes.remove(&f);
        } else {
            self.owner_changes.insert(f, ch);
        }
        self.refresh_face_owner(f);
    }

    /// Record that `f` belongs to `who`. With a year selected this is a
    /// change from that year on (until the face's next recorded change);
    /// earlier years are untouched. With no year selected it sets the owner
    /// for all years, which is refused when the face has a history.
    pub fn record_owner(&mut self, f: FaceId, who: Option<EntityId>) -> TResult<()> {
        // on a share layer, owning a face means holding all of it
        if self.active_is_shares() {
            return self.set_share(f, who, FULL_SHARE);
        }
        if self.owner_at(f, self.year) == who {
            return Ok(());
        }
        if let Some(e) = who.and_then(|w| self.entities.get(&w)).filter(|e| !e.exists_in(self.year)) {
            return Err(format!("{} 在 {} 年不存续（{}），不能拥有领土", e.name, self.year.unwrap_or_default(), e.lifespan_text()));
        }
        let base = self.owner_base.get(&f).copied();
        let mut ch = self.owner_changes.get(&f).cloned().unwrap_or_default();
        match self.year {
            Some(y) => {
                insert_event(&base, &mut ch, y, who);
                self.set_history(f, base, ch);
            }
            None if ch.is_empty() => self.set_history(f, who, ch),
            None => return Err("这个面的归属随年份变化，请先选择年份再修改".into()),
        }
        Ok(())
    }

    /// Whether two faces have the same ownership in every year, in every layer.
    pub fn same_history(&self, a: FaceId, b: FaceId) -> bool {
        self.kind_history(a) == self.kind_history(b)
            && self.owner_base.get(&a) == self.owner_base.get(&b)
            && self.owner_changes.get(&a) == self.owner_changes.get(&b)
            && self.layers.iter().filter(|(l, _)| **l != self.active_layer).all(|(_, x)| x.parked.same(a, b))
            && self.layers.values().all(|x| x.shares.same(a, b))
    }

    /// All years at which some ownership, share or parent relation changes
    /// (shares only for the active layer, like ownership).
    pub fn change_years(&self) -> std::collections::BTreeSet<i32> {
        let shares = self.layers.get(&self.active_layer).filter(|x| x.kind == LayerKind::Shares).map(|x| x.shares.changes.values().flat_map(|c| c.keys().copied()).collect::<Vec<_>>()).unwrap_or_default();
        self.owner_changes
            .values()
            .flat_map(|c| c.keys().copied())
            .chain(shares)
            .chain(self.kind_changes.values().flat_map(|c| c.keys().copied()))
            .chain(self.entities.values().flat_map(|e| e.parent_changes.keys().copied()))
            .chain(self.diplomacy.iter().flat_map(|d| d.records.values())
                .flat_map(|r| r.from.into_iter().chain(r.until.and_then(|year| year.checked_add(1)))))
            // founding years, and the year after an end
            .chain(self.entities.values().flat_map(|e| e.founded.into_iter().chain(e.ended.map(|y| y + 1))))
            .collect()
    }

    /// Years at which entity `e` gains or loses territory, changes parent,
    /// or gains or loses a member.
    pub fn entity_change_years(&self, e: EntityId) -> std::collections::BTreeSet<i32> {
        let mut out = std::collections::BTreeSet::new();
        if let Some(x) = self.entities.get(&e) {
            out.extend(x.founded.into_iter().chain(x.ended.map(|y| y + 1)));
        }
        for (id, x) in self.entities.iter() {
            let mut cur = x.parent_base;
            for (y, p) in &x.parent_changes {
                if *id == e || cur == Some(e) || *p == Some(e) {
                    out.insert(*y);
                }
                cur = *p;
            }
        }
        for (f, ch) in self.owner_changes.iter() {
            let mut cur = self.owner_base.get(f).copied();
            for (y, o) in ch {
                if cur == Some(e) || *o == Some(e) {
                    out.insert(*y);
                }
                cur = *o;
            }
        }
        out
    }

    /// Ownership history of a face: base owner and change points.
    pub fn face_history(&self, f: FaceId) -> (Option<EntityId>, Vec<(i32, Option<EntityId>)>) {
        (self.owner_base.get(&f).copied(), self.owner_changes.get(&f).map(|c| c.iter().map(|(y, o)| (*y, *o)).collect()).unwrap_or_default())
    }

    /// Rewrite every parent in all years (merging or deleting entities).
    /// A parent that becomes the entity itself is dropped (merging a unit's
    /// parent into it).
    fn map_parents(&mut self, m: impl Fn(EntityId) -> Option<EntityId>) {
        let ids: Vec<EntityId> = self.entities.keys().copied().collect();
        for id in ids {
            let x = &self.entities[&id];
            let base = x.parent_base.and_then(&m).filter(|p| *p != id);
            let mut ch: OwnerChanges = x.parent_changes.iter().map(|(y, p)| (*y, p.and_then(&m).filter(|p| *p != id))).collect();
            if base != x.parent_base || ch != x.parent_changes {
                normalize(base, &mut ch);
                self.set_parent_history(id, base, ch);
            }
        }
    }

    /// Rewrite every owner in all years (merging or deleting entities).
    fn map_owners(&mut self, m: impl Fn(EntityId) -> Option<EntityId>) {
        self.each_parked(|o| o.map_entities(&m));
        self.each_share_layer(|s| s.map_entities(&m));
        let faces: HashSet<FaceId> = self.owner_base.keys().chain(self.owner_changes.keys()).copied().collect();
        for f in faces {
            let base = self.owner_base.get(&f).and_then(|e| m(*e));
            let mut ch: OwnerChanges = self.owner_changes.get(&f).map(|c| c.iter().map(|(y, o)| (*y, o.and_then(&m))).collect()).unwrap_or_default();
            normalize(base, &mut ch);
            self.set_history(f, base, ch);
        }
    }

    /// After an edit replaced faces: `self.owner` holds the intended owners
    /// in the selected year for every face; `src_of` names, for each new
    /// face, the old face it was cut from (None = new land). New faces start
    /// with their source's history, then the intended owner is recorded for
    /// the selected year, so other years keep what they had.
    pub(crate) fn adopt_owner_history(&mut self, old: &Document, src_of: &HashMap<FaceId, Option<FaceId>>) -> TResult<()> {
        let intended = self.owner.clone();
        let intended_kind: HashMap<FaceId, FaceKind> = self.topo.faces.iter().map(|(f, x)| (*f, x.kind)).collect();
        let live = |f: &FaceId| self.topo.faces.contains_key(f) && !src_of.contains_key(f);
        // land/water: surviving faces keep their history; a new face starts
        // from its source's (a fixed-kind source: that kind), then the
        // intended kind is recorded for the selected year
        self.kind_base = old.kind_base.iter().filter(|(f, _)| live(f)).map(|(f, k)| (*f, *k)).collect();
        self.kind_changes = old.kind_changes.iter().filter(|(f, _)| live(f)).map(|(f, c)| (*f, c.clone())).collect();
        for (f, src) in src_of {
            let Some(s) = src else { continue };
            let (kb, kc) = old.kind_history(*s);
            if !kc.is_empty() || kb != intended_kind[f] {
                self.kind_base.insert(*f, kb);
                self.kind_changes.insert(*f, kc.into_iter().collect());
            }
        }
        let faces: Vec<FaceId> = self.topo.faces.keys().copied().collect();
        for f in &faces {
            if !self.kind_changes.contains_key(f) {
                continue;
            }
            let want = intended_kind[f];
            let (kb, kc) = self.kind_history(*f);
            let mut ch: BTreeMap<i32, FaceKind> = kc.into_iter().collect();
            let now = {
                let last = match self.year {
                    Some(y) => ch.range(..=y).next_back(),
                    None => ch.iter().next_back(),
                };
                last.map_or(kb, |(_, k)| *k)
            };
            match self.year {
                _ if now == want => {}
                Some(y) => insert_event(&kb, &mut ch, y, want),
                None if ch.is_empty() => {
                    // a fixed kind after all
                    self.kind_base.remove(f);
                    self.kind_changes.remove(f);
                    continue;
                }
                None => return Err("这些地块的海陆随年份变化，请先选择年份再绘制".into()),
            }
            // set without the owner refresh (owners are adopted below)
            if ch.is_empty() {
                self.kind_base.remove(f);
                self.kind_changes.remove(f);
            } else {
                self.kind_base.insert(*f, kb);
                self.kind_changes.insert(*f, ch);
            }
        }
        // inactive layers: new faces inherit, nothing else changes
        let ids: Vec<LayerId> = self.layers.keys().copied().filter(|l| *l != self.active_layer).collect();
        for l in ids {
            let mut x = self.layers[&l].clone();
            x.parked = Ownership::inherit(&old.layers.get(&l).map(|o| o.parked.clone()).unwrap_or_default(), &live, src_of);
            self.layers.insert(l, x);
        }
        let ids: Vec<LayerId> = self.layers.iter().filter(|(_, x)| x.kind == LayerKind::Shares).map(|(l, _)| *l).collect();
        for l in ids {
            let mut x = self.layers[&l].clone();
            x.shares = ShareStore::inherit(&old.layers.get(&l).map(|o| o.shares.clone()).unwrap_or_default(), &live, src_of);
            self.layers.insert(l, x);
        }
        self.owner_base = old.owner_base.iter().filter(|(f, _)| live(f)).map(|(f, e)| (*f, *e)).collect();
        self.owner_changes = old.owner_changes.iter().filter(|(f, _)| live(f)).map(|(f, c)| (*f, c.clone())).collect();
        for (f, src) in src_of {
            if let Some(s) = src {
                if let Some(e) = old.owner_base.get(s) {
                    self.owner_base.insert(*f, *e);
                }
                if let Some(c) = old.owner_changes.get(s) {
                    self.owner_changes.insert(*f, c.clone());
                }
            }
        }
        self.refresh_owner();
        let faces: Vec<FaceId> = self.topo.faces.keys().copied().collect();
        for f in faces {
            let want = intended.get(&f).copied();
            if self.owner.get(&f).copied() != want {
                self.record_owner(f, want)?;
            }
        }
        Ok(())
    }

    /// Make the background sea or land. A sea background loses its owner.
    pub fn set_background_kind(&mut self, kind: FaceKind) {
        let Some(f) = self.topo.faces.get(&BACKGROUND) else { return };
        if f.kind == kind {
            return;
        }
        let mut f = f.clone();
        f.kind = kind;
        self.topo.faces.insert(BACKGROUND, f);
        self.topo.touch(BACKGROUND);
        if kind == FaceKind::Water {
            self.set_history(BACKGROUND, None, OwnerChanges::new());
            self.each_parked(|o| o.set(BACKGROUND, None, OwnerChanges::new()));
            self.each_share_layer(|s| s.set(BACKGROUND, Shares::new(), BTreeMap::new()));
            self.refresh_owner();
        }
    }

    pub fn background_kind(&self) -> FaceKind {
        self.topo.faces.get(&BACKGROUND).map_or(FaceKind::Water, |f| f.kind)
    }

    pub fn faces_of(&self, e: EntityId) -> Vec<FaceId> {
        self.owner.iter().filter(|(_, o)| **o == e).map(|(f, _)| *f).collect()
    }

    /// The faces of an entity and of everything under it.
    pub fn faces_of_tree(&self, e: EntityId) -> Vec<FaceId> {
        let mut belongs = HashMap::<EntityId, bool>::new();
        self.owner
            .iter()
            .filter(|(_, o)| *belongs.entry(**o).or_insert_with(|| self.ancestors(**o).any(|a| a == e)))
            .map(|(f, _)| *f)
            .collect()
    }

    /// Top-level country for an entity (follows `parent`).
    pub fn top_ancestor(&self, e: EntityId) -> EntityId {
        self.ancestors(e).last().unwrap_or(e)
    }

    /// The entity followed by its parents in the active historical view.
    pub fn ancestors(&self, e: EntityId) -> impl Iterator<Item = EntityId> + '_ {
        std::iter::successors(Some(e), |id| {
            self.entities.get(id)?.parent.filter(|parent| self.entities.contains_key(parent))
        }).take(self.entities.len().max(1))
    }

    /// Merge entity `from` into `into`: territory, cities and missing values move.
    pub fn merge_entities(&mut self, from: EntityId, into: EntityId) -> TResult<()> {
        if from == into {
            return Err("不能与自身合并".into());
        }
        if self.entities.get(&from).map(|e| e.layer) != self.entities.get(&into).map(|e| e.layer) {
            return Err("只能合并同一层中的单位".into());
        }
        self.map_owners(|e| Some(if e == from { into } else { e }));
        self.map_parents(|p| Some(if p == from { into } else { p }));
        self.break_cycles_through(into);
        // what named it as a successor now names the one it became
        let ids: Vec<EntityId> = self.entities.keys().copied().collect();
        for id in ids {
            let x = self.entities.get_mut(&id).unwrap();
            if x.successors.contains(&from) {
                x.successors.retain(|s| *s != from && *s != id);
                if !x.successors.contains(&into) && id != into {
                    x.successors.push(into);
                }
            }
        }
        self.delete_entity(from);
        Ok(())
    }

    /// After a merge into `into`: a chain of parents that ran from `into`
    /// down through the merged unit's descendants now comes back to `into`
    /// (into → x → from became into → x → into). Such parents of `into` are
    /// dropped; what was under it stays under it.
    fn break_cycles_through(&mut self, into: EntityId) {
        let Some(x) = self.entities.get(&into) else { return };
        let (old_base, old_ch) = (x.parent_base, x.parent_changes.clone());
        let mut years: Vec<Option<i32>> = vec![None, Some(i32::MIN)];
        years.extend(self.change_years().into_iter().map(Some));
        let (base, mut ch) = {
            let back = |p: EntityId| {
                years.iter().any(|y| {
                    let mut cur = p;
                    for _ in 0..=self.entities.len() {
                        if cur == into {
                            return true;
                        }
                        match self.parent_at(cur, *y) {
                            Some(q) => cur = q,
                            None => return false,
                        }
                    }
                    true
                })
            };
            (old_base.filter(|p| !back(*p)), old_ch.iter().map(|(y, p)| (*y, p.filter(|p| !back(*p)))).collect::<OwnerChanges>())
        };
        if base != old_base || ch != old_ch {
            normalize(base, &mut ch);
            self.set_parent_history(into, base, ch);
        }
    }

    /// Parents that are the entity itself (left by merges before they were
    /// dropped there) removed; for files written before.
    pub fn drop_self_parents(&mut self) {
        let ids: Vec<EntityId> = self.entities.iter().filter(|(id, x)| x.parent_base == Some(**id) || x.parent_changes.values().any(|p| *p == Some(**id))).map(|(id, _)| *id).collect();
        for id in ids {
            let x = &self.entities[&id];
            let base = x.parent_base.filter(|p| *p != id);
            let mut ch: OwnerChanges = x.parent_changes.iter().map(|(y, p)| (*y, p.filter(|p| *p != id))).collect();
            normalize(base, &mut ch);
            self.set_parent_history(id, base, ch);
        }
    }

    pub fn delete_entity(&mut self, e: EntityId) {
        self.map_owners(|x| (x != e).then_some(x));
        self.map_parents(|x| (x != e).then_some(x));
        let keys: Vec<_> = self.values.keys().filter(|(id, _)| *id == e).copied().collect();
        for k in keys {
            self.values.remove(&k);
        }
        let refs: Vec<EntityId> = self
            .entities
            .iter()
            .filter(|(_, x)| x.capital == Some(e))
            .map(|(id, _)| *id)
            .collect();
        for id in refs {
            self.entities.get_mut(&id).unwrap().capital = None;
        }
        let ids: Vec<EntityId> = self.entities.iter().filter(|(_, x)| x.successors.contains(&e)).map(|(id, _)| *id).collect();
        for id in ids {
            self.entities.get_mut(&id).unwrap().successors.retain(|s| *s != e);
        }
        self.name_styles.remove(&e);
        self.entities.remove(&e);
    }

    // -------------------------------------------------------- topology ops
    // Wrappers keep ownership consistent with topology changes.

    pub fn split_face(&mut self, cut: &[LonLat]) -> TResult<(FaceId, FaceId)> {
        let r = self.radius();
        let (a, b) = self.topo.split_face(cut, r)?;
        // the subdivision holds in every year: both halves share a's history
        let (base, ch) = (self.owner_base.get(&a).copied(), self.owner_changes.get(&a).cloned().unwrap_or_default());
        self.set_history(b, base, ch);
        let (kb, kc) = self.kind_history(a);
        if !kc.is_empty() {
            self.set_kind_history(b, kb, kc.into_iter().collect());
        }
        self.each_parked(|o| {
            let (base, ch) = o.history(a);
            o.set(b, base, ch);
        });
        self.each_share_layer(|s| {
            let (base, ch) = s.history(a);
            s.set(b, base, ch);
        });
        self.refresh_owner();
        Ok((a, b))
    }

    pub fn merge_faces(&mut self, a: FaceId, b: FaceId) -> TResult<FaceId> {
        if !self.same_history(a, b) {
            return Err("这两个面在某些年份归属不同，合并会改变那些年份的地图".into());
        }
        let id = self.topo.merge_faces(a, b)?;
        self.set_history(b, None, OwnerChanges::new());
        self.kind_base.remove(&b);
        self.kind_changes.remove(&b);
        self.each_parked(|o| o.set(b, None, OwnerChanges::new()));
        self.each_share_layer(|s| s.set(b, Shares::new(), BTreeMap::new()));
        self.refresh_owner();
        Ok(id)
    }

    /// The river passing nearest to `p` within `tol` degrees: its id, the
    /// segment, and the closest point on it.
    pub fn river_at(&self, p: LonLat, tol: f64) -> Option<(LineId, usize, LonLat)> {
        self.river_at_where(p, tol, |_, _| true)
    }

    pub fn river_at_where(&self, p: LonLat, tol: f64, visible: impl Fn(LineId, &LineFeature) -> bool) -> Option<(LineId, usize, LonLat)> {
        let mut best: Option<(f64, LineId, usize, LonLat)> = None;
        for (id, r) in self.rivers.iter().filter(|(id, r)| !r.hidden && !self.group_hides(**id) && visible(**id, r)) {
            for (i, w) in r.pts.windows(2).enumerate() {
                let (q, d, _) = geom::closest_on_segment(p, w[0], w[1]);
                if d <= tol && best.is_none_or(|b| d < b.0) {
                    best = Some((d, *id, i, q));
                }
            }
        }
        best.map(|(_, id, i, q)| (id, i, q))
    }

    /// The river point nearest to `p` within `tol` degrees, with its distance.
    pub fn river_point_at(&self, p: LonLat, tol: f64) -> Option<(LineId, usize, f64)> {
        let mut best: Option<(LineId, usize, f64)> = None;
        for (id, r) in self.rivers.iter().filter(|(id, r)| !r.hidden && !self.group_hides(**id)) {
            for (i, q) in r.pts.iter().enumerate() {
                let d = geom::geo_dist_deg(p, *q);
                if d <= tol && best.is_none_or(|b| d < b.2) {
                    best = Some((*id, i, d));
                }
            }
        }
        best
    }

    /// Length of a river in km (great-circle segments).
    pub fn river_length_km(&self, id: LineId) -> f64 {
        let r = self.radius();
        self.rivers.get(&id).map_or(0.0, |x| {
            x.pts
                .windows(2)
                .map(|w| {
                    let (p1, p2) = (w[0].lat.to_radians(), w[1].lat.to_radians());
                    let dl = (w[1].lon - w[0].lon).to_radians();
                    let h = ((p2 - p1) * 0.5).sin().powi(2) + p1.cos() * p2.cos() * (dl * 0.5).sin().powi(2);
                    2.0 * r * h.sqrt().min(1.0).asin()
                })
                .sum()
        })
    }

    /// Remove one point of a river; a river keeps at least two.
    pub fn delete_river_point(&mut self, id: LineId, i: usize) -> TResult<()> {
        let mut r = self.rivers.get(&id).ok_or("河流不存在")?.clone();
        if r.pts.len() <= 2 {
            return Err("河流至少需要两个点（可改为删除整条河流）".into());
        }
        if i >= r.pts.len() {
            return Err("点不存在".into());
        }
        r.pts.remove(i);
        self.rivers.insert(id, r);
        self.points_changed(id);
        Ok(())
    }

    /// Merge every pair of neighbouring pieces whose whole history is the
    /// same (land/water, owner in every layer, shares). Dated edits split
    /// land into pieces that differ only from some year; once pieces agree
    /// in every year the border between them carries nothing, so no year's
    /// map changes. Returns how many merges were made.
    pub fn tidy_faces(&mut self) -> usize {
        let mut merged = 0;
        loop {
            let pairs: BTreeSet<(FaceId, FaceId)> = self
                .topo
                .edges
                .values()
                .filter(|e| e.left != e.right && e.left != BACKGROUND && e.right != BACKGROUND)
                .map(|e| (e.left.min(e.right), e.left.max(e.right)))
                .collect();
            let mut gone: HashSet<FaceId> = HashSet::new();
            let mut any = false;
            for (a, b) in pairs {
                if gone.contains(&a) || gone.contains(&b) || !self.same_history(a, b) {
                    continue;
                }
                // a refused merge must not leave half a merge behind
                let mut trial = self.clone();
                if trial.merge_faces(a, b).is_ok() {
                    *self = trial;
                    gone.extend([a, b]);
                    merged += 1;
                    any = true;
                }
            }
            if !any {
                return merged;
            }
        }
    }

    /// Smooth or roughen every boundary of a face (shared borders change
    /// for both sides; geometry is the same in every year). End nodes stay,
    /// so borders keep meeting where they did. With `keep_area` each edge
    /// is corrected so the area on its two sides is unchanged, which keeps
    /// every face's (and so every country's, in every year) area. Returns
    /// how many edges changed.
    pub fn restyle_face(&mut self, f: FaceId, style: crate::shape::LineStyle, amount: f64, min_len: f64, keep_area: bool) -> TResult<usize> {
        let face = self.topo.faces.get(&f).ok_or("面不存在")?;
        let mut edges: Vec<crate::topo::EdgeId> = std::iter::once(&face.outer).chain(face.holes.iter()).flatten().map(|r| r.edge).collect();
        edges.sort();
        edges.dedup();
        let mut n = 0;
        // drop detail below the target scale first, so the style shapes the
        // line instead of only its tiny steps
        let thin = |pts: Vec<LonLat>| -> Vec<LonLat> {
            if pts.len() <= 2 {
                return pts;
            }
            let k = (pts.iter().map(|p| p.lat).sum::<f64>() / pts.len() as f64).to_radians().cos().max(0.05);
            let planar: Vec<[f64; 2]> = geom::unwrap_path(&pts, None).into_iter().map(|q| [q[0] * k, q[1]]).collect();
            crate::trace::simplify(&planar, min_len * 0.75).into_iter().map(|i| pts[i]).collect()
        };
        for id in edges {
            let e = &self.topo.edges[&id];
            let original = self.topo.edge_full(e);
            let full = thin(original.clone());
            let interior = if e.a == e.b {
                // a ring of its own (an island's whole coast): closed, the node fixed
                let ring = &full[..full.len() - 1];
                let mut fixed = vec![false; ring.len()];
                fixed[0] = true;
                let (mut out, flags) = crate::shape::apply(ring, &fixed, true, style, amount, min_len);
                if keep_area {
                    // no area-keeping shape: this coast stays as it is
                    let Some(k) = crate::shape::preserve_area(&original[..original.len() - 1], &out, &flags, true) else { continue };
                    out = k;
                }
                out[1..].to_vec()
            } else {
                let mut fixed = vec![false; full.len()];
                fixed[0] = true;
                *fixed.last_mut().unwrap() = true;
                let (mut out, flags) = crate::shape::apply(&full, &fixed, false, style, amount, min_len);
                if keep_area {
                    let Some(k) = crate::shape::preserve_area(&original, &out, &flags, false) else { continue };
                    out = k;
                }
                out[1..out.len() - 1].to_vec()
            };
            if interior != e.pts {
                self.topo.replace_edge_points(id, interior)?;
                n += 1;
            }
        }
        Ok(n)
    }

    /// Add a closed shape (island, lake) inside one face. With a year
    /// selected the shape appears from that year: before it, the area was
    /// part of the face it was drawn in (same land/water, same owners in
    /// every layer).
    pub fn add_ring_face(&mut self, pts: &[LonLat], kind: FaceKind) -> TResult<FaceId> {
        let container = pts.first().map(|p| self.topo.face_at(*p));
        let f = self.topo.add_ring_face(pts, kind)?;
        if let (Some(y), Some(c)) = (self.year, container) {
            // the area's past is the container's
            let (kb, kc) = self.kind_history(c);
            let mut ch: BTreeMap<i32, FaceKind> = kc.into_iter().filter(|(cy, _)| *cy < y).collect();
            insert_event(&kb, &mut ch, y, kind);
            let (ob, oc) = (self.owner_base.get(&c).copied(), self.owner_changes.get(&c).cloned().unwrap_or_default());
            self.set_history(f, ob, oc);
            self.each_parked(|o| {
                let (base, ch) = o.history(c);
                o.set(f, base, ch);
            });
            self.each_share_layer(|s| {
                let (base, ch) = s.history(c);
                s.set(f, base, ch);
            });
            self.set_kind_history(f, kb, ch);
            self.refresh_owner();
        }
        Ok(f)
    }

    // -------------------------------------------------------------- fields

    pub fn add_field(&mut self, key: &str, name: &str, kind: FieldKind, expr: Option<String>) -> TResult<FieldId> {
        let key = key.trim();
        if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Err("字段键只能包含字母、数字和下划线".into());
        }
        if key == AREA_KEY || self.field_by_key(key).is_some() {
            return Err(format!("字段键 `{key}` 已存在"));
        }
        if let Some(src) = &expr {
            expr::parse(src)?;
        }
        let id = FieldId(self.topo.alloc());
        self.fields.insert(id, FieldDef { key: key.into(), name: name.into(), kind, unit: String::new(), expr, inherit: None, base_year: None });
        Ok(id)
    }

    pub fn field_by_key(&self, key: &str) -> Option<FieldId> {
        self.fields.iter().find(|(_, f)| f.key == key).map(|(id, _)| *id)
    }

    pub fn set_value(&mut self, e: EntityId, f: FieldId, year: Option<i32>, v: Option<Value>) {
        let mut s = self.values.get(&(e, f)).cloned().unwrap_or_default();
        match (year, v) {
            (None, v) => s.base = v,
            (Some(y), Some(v)) => {
                s.by_year.insert(y, v);
            }
            (Some(y), None) => {
                s.by_year.remove(&y);
            }
        }
        if s.base.is_none() && s.by_year.is_empty() {
            self.values.remove(&(e, f));
        } else {
            self.values.insert((e, f), s);
        }
    }

    /// The value of a stored field in the selected year, under the field's
    /// inheritance policy.
    pub fn raw_value(&self, e: EntityId, f: FieldId) -> Option<Value> {
        self.value_with_source(e, f).0
    }

    /// The value and where it comes from.
    pub fn value_with_source(&self, e: EntityId, f: FieldId) -> (Option<Value>, Source) {
        let (Some(s), Some(def)) = (self.values.get(&(e, f)), self.fields.get(&f)) else { return (None, Source::Unknown) };
        s.resolve(self.year, def.policy(), def.base_year)
    }

    /// What is actually stored for the selected year: that year's record,
    /// or the base value when no year is selected. This is what editing
    /// changes.
    pub fn stored_value(&self, e: EntityId, f: FieldId) -> Option<Value> {
        let s = self.values.get(&(e, f))?;
        match self.year {
            Some(y) => s.by_year.get(&y).cloned(),
            None => s.base.clone(),
        }
    }

    /// Source of a field value by key (derived fields: `Derived` when they
    /// have a value).
    pub fn source_of(&self, e: EntityId, key: &str) -> Source {
        if key == AREA_KEY {
            return Source::Derived;
        }
        let Some(fid) = self.field_by_key(key) else { return Source::Unknown };
        if self.fields[&fid].expr.is_some() {
            return crate::stats::Stats::new(self).stat(e, key).source;
        }
        self.value_with_source(e, fid).1
    }

    /// Area per entity in km² (sum of owned faces).
    pub fn areas_by_entity(&self) -> HashMap<EntityId, f64> {
        let r = self.radius();
        self.areas_by_entity_with(|f| self.topo.face_area(f, r))
    }

    /// As [`Document::areas_by_entity`], with each face's area given (a
    /// face's shape is the same in every year, so callers going through
    /// many years work the areas out once).
    pub fn areas_by_entity_with(&self, face_area: impl Fn(FaceId) -> f64) -> HashMap<EntityId, f64> {
        let mut out: HashMap<EntityId, f64> = HashMap::new();
        // a share layer: each unit gets its share of every face
        if self.active_is_shares() {
            let st = &self.layers[&self.active_layer].shares;
            let faces: HashSet<FaceId> = st.base.keys().chain(st.changes.keys()).copied().collect();
            for f in faces.into_iter().filter(|f| self.topo.faces.contains_key(f)) {
                let s = self.shares_shown(f);
                if s.is_empty() {
                    continue;
                }
                let a = face_area(f);
                for (e, bp) in s {
                    *out.entry(e).or_default() += a * bp as f64 / FULL_SHARE as f64;
                }
            }
            return out;
        }
        for (f, e) in self.owner.iter() {
            if self.topo.faces.contains_key(f) {
                *out.entry(*e).or_default() += face_area(*f);
            }
        }
        out
    }

    /// Numeric value of a field key for an entity, including the built-in
    /// area and derived (expression) fields.
    /// One numeric value (see [`crate::stats::Stats`] for many lookups:
    /// areas of other years are then computed once).
    pub fn num_value(&self, e: EntityId, key: &str, areas: &HashMap<EntityId, f64>) -> Option<f64> {
        crate::stats::Stats::with_areas(self, areas.clone()).value(e, key)
    }

    // ------------------------------------------------------------- dissolve

    /// Outline of everything owned by `e` as planar polygons (shared internal
    /// boundaries removed). Used for labels, SVG/GeoJSON export.
    pub fn dissolve(&self, e: EntityId) -> Vec<PlanarPolygon> {
        let faces: HashSet<FaceId> = self.faces_of(e).into_iter().filter(|f| self.topo.faces.contains_key(f)).collect();
        self.dissolve_faces(&faces)
    }

    pub fn dissolve_faces(&self, faces: &HashSet<FaceId>) -> Vec<PlanarPolygon> {
        let mut refs: Vec<ArcRef> = Vec::new();
        // in a fixed order, so the same outline comes out the same way every
        // time (an exported SVG does not change from one run to the next)
        let mut sorted: Vec<&FaceId> = faces.iter().collect();
        sorted.sort();
        for f in sorted {
            let face = &self.topo.faces[f];
            for ring in std::iter::once(&face.outer).chain(face.holes.iter()) {
                for r in ring {
                    let edge = &self.topo.edges[&r.edge];
                    let other = if r.rev { edge.left } else { edge.right };
                    if !faces.contains(&other) {
                        refs.push(*r);
                    }
                }
            }
        }
        let Ok(mut rings) = self.topo.assemble_rings(&refs) else {
            return vec![];
        };
        // land around a pole, in pieces cut along meridians: the pieces'
        // edges on the pole make a ring of their own that encloses nothing
        rings.retain(|ring| !ring.iter().all(|r| self.topo.on_pole(&self.topo.edges[&r.edge])));
        let mut outers: Vec<(f64, PlanarPolygon, [f64; 4])> = Vec::new();
        let mut holes: Vec<Vec<P2>> = Vec::new();
        for ring in rings {
            let coords = self.topo.ring_coords(&ring);
            let planar = geom::ring_to_planar(&coords);
            let a = geom::signed_area(&planar);
            if a > 0.0 {
                let bb = geom::bbox(&planar);
                outers.push((a, PlanarPolygon { outer: planar, holes: vec![] }, bb));
            } else {
                holes.push(geom::unwrap_path(&coords, None));
            }
        }
        outers.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        for mut h in holes {
            let s = h[0];
            if let Some(o) = outers.iter_mut().find(|o| geom::point_in_poly_wrapped(s, &o.1.outer, &o.2)) {
                let c = (o.2[0] + o.2[2]) * 0.5;
                let k = ((c - s[0]) / 360.0).round() * 360.0;
                for p in &mut h {
                    p[0] += k;
                }
                o.1.holes.push(h);
            }
        }
        outers.into_iter().rev().map(|o| o.1).collect()
    }

    /// Label position: interior point of the largest dissolved part.
    pub fn label_anchor(&self, e: EntityId) -> Option<(LonLat, f64)> {
        let polys = self.dissolve(e);
        let r = self.radius();
        let best = polys.iter().max_by(|a, b| geom::spherical_area(&a.outer, r).partial_cmp(&geom::spherical_area(&b.outer, r)).unwrap());
        // the background has no outer ring, so dissolving skips it
        if self.owner.get(&BACKGROUND) == Some(&e) {
            let bg = self.topo.face_area(BACKGROUND, r);
            if best.is_none_or(|b| geom::spherical_area(&b.outer, r).abs() < bg) {
                return self.background_anchor();
            }
        }
        let best = best?;
        // a cap over a pole: its middle is the pole, not some place along
        // the map's top or bottom edge. The label sits towards the prime
        // meridian from the pole, inside the cap.
        let bb = geom::bbox(&best.outer);
        if bb[2] - bb[0] > 359.9 {
            if let Some(pole) = best.outer.iter().map(|p| p[1]).find(|y| y.abs() == 90.0) {
                let reach = best.outer.iter().map(|p| (pole - p[1]).abs()).filter(|d| *d > 0.0).fold(f64::INFINITY, f64::min);
                let free = best.holes.iter().flatten().map(|p| (pole - p[1]).abs()).fold(reach, f64::min);
                if free.is_finite() && free > 0.0 {
                    return Some((LonLat::new(0.0, pole - pole.signum() * free * 0.35), free * 1.3));
                }
            }
        }
        let (p, w) = geom::interior_point(&best.outer, &best.holes)?;
        Some((LonLat::new(geom::wrap_lon(p[0]), p[1]), w))
    }

    /// The point of the background farthest from everything else, on a 5°
    /// grid (distance in grid steps, east-west scaled by cos lat), with twice
    /// that distance in degrees as the usable width.
    fn background_anchor(&self) -> Option<(LonLat, f64)> {
        const STEP: f64 = 5.0;
        let (nx, ny) = ((360.0 / STEP) as usize, (170.0 / STEP) as usize + 1);
        let lat = |j: usize| -85.0 + j as f64 * STEP;
        let lon = |i: usize| -180.0 + i as f64 * STEP + STEP * 0.5;
        let free: Vec<bool> = (0..nx * ny).map(|k| self.topo.face_at(LonLat::new(lon(k % nx), lat(k / nx))) == BACKGROUND).collect();
        let blocked: Vec<(usize, usize)> = (0..nx * ny).filter(|k| !free[*k]).map(|k| (k % nx, k / nx)).collect();
        let mut best: Option<(usize, f64)> = None;
        for k in (0..nx * ny).filter(|k| free[*k]) {
            let (i, j) = (k % nx, k / nx);
            let c = lat(j).to_radians().cos();
            let d = blocked
                .iter()
                .map(|(bi, bj)| {
                    let di = (i as f64 - *bi as f64).abs();
                    let di = di.min(nx as f64 - di) * c;
                    let dj = j as f64 - *bj as f64;
                    (di * di + dj * dj).sqrt()
                })
                .fold(f64::INFINITY, f64::min);
            // nothing else on the planet: prefer the equator
            let d = if d.is_infinite() { 1e9 - lat(j).abs() } else { d };
            if best.is_none_or(|b| d > b.1) {
                best = Some((k, d));
            }
        }
        let (k, d) = best?;
        let w = if d >= 1e8 { 60.0 } else { (2.0 * d * STEP).min(60.0) };
        Some((LonLat::new(lon(k % nx), lat(k / nx)), w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, s: f64) -> Vec<LonLat> {
        vec![LonLat::new(x0, y0), LonLat::new(x0 + s, y0), LonLat::new(x0 + s, y0 + s), LonLat::new(x0, y0 + s)]
    }

    #[test]
    fn countries_merge_dissolve_and_values() {
        let mut d = Document::default();
        let f = d.add_ring_face(&square(0.0, 0.0, 10.0), FaceKind::Land).unwrap();
        let (a, b) = d.split_face(&[LonLat::new(5.0, 0.0), LonLat::new(5.0, 10.0)]).unwrap();
        assert_eq!(a, f);
        let x = d.add_entity(EntityKind::Country, "X");
        let y = d.add_entity(EntityKind::Country, "Y");
        d.assign(a, Some(x)).unwrap();
        d.assign(b, Some(y)).unwrap();
        assert_eq!(d.dissolve(x).len(), 1);
        // pop & derived density
        let pop = d.add_field("pop", "人口", FieldKind::Number, None).unwrap();
        d.add_field("density", "密度", FieldKind::Number, Some("pop / area_km2".into())).unwrap();
        d.set_value(x, pop, None, Some(Value::Num(1000.0)));
        d.set_value(x, pop, Some(1900), Some(Value::Num(10.0)));
        let areas = d.areas_by_entity();
        let dens = d.num_value(x, "density", &areas).unwrap();
        assert!((dens - 1000.0 / areas[&x]).abs() < 1e-12);
        // numbers are strict by default: only exact records count
        d.year = Some(1950);
        assert_eq!(d.num_value(x, "pop", &areas), None);
        d.year = Some(1900);
        assert_eq!(d.num_value(x, "pop", &areas), Some(10.0));
        d.year = None;
        assert_eq!(d.num_value(x, "pop", &areas), Some(1000.0), "no year: the base value");
        d.year = Some(1950);
        d.fields.get_mut(&pop).unwrap().inherit = Some(Inherit::Nearest);
        assert_eq!(d.num_value(x, "pop", &areas), Some(10.0));
        // merge: Y joins X, dissolved outline becomes one square
        d.merge_entities(y, x).unwrap();
        let polys = d.dissolve(x);
        assert_eq!(polys.len(), 1);
        assert_eq!(polys[0].outer.len(), 6); // 4 corners + 2 cut nodes
        assert!(d.label_anchor(x).is_some());
    }

    #[test]
    fn exclave_dissolves_to_two_parts() {
        let mut d = Document::default();
        let a = d.add_ring_face(&square(0.0, 0.0, 5.0), FaceKind::Land).unwrap();
        let b = d.add_ring_face(&square(20.0, 0.0, 5.0), FaceKind::Land).unwrap();
        let x = d.add_entity(EntityKind::Country, "X");
        d.assign(a, Some(x)).unwrap();
        d.assign(b, Some(x)).unwrap();
        assert_eq!(d.dissolve(x).len(), 2);
    }
}
