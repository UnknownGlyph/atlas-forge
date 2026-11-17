//! Every drawn feature of a map in one list: map text, symbols, routes,
//! rivers and hatched areas, each with its kind, name, years and a point
//! to show it at. For finding things again as a map grows (and, later,
//! for anything that needs to see the layout as a whole).

use crate::doc::{Document, LineId};
use crate::geom::LonLat;

/// Which drawn feature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum FeatureRef {
    Text(LineId),
    Symbol(LineId),
    Route(LineId),
    River(LineId),
    Area(LineId),
}

impl FeatureRef {
    /// The feature's id within its kind's collection.
    pub fn id(self) -> LineId {
        let (FeatureRef::Text(id) | FeatureRef::Symbol(id) | FeatureRef::Route(id) | FeatureRef::River(id) | FeatureRef::Area(id)) = self;
        id
    }

    /// The kind's name, for headings.
    pub fn kind_name(self) -> &'static str {
        match self {
            FeatureRef::Text(_) => "地图文字",
            FeatureRef::Symbol(_) => "符号",
            FeatureRef::Route(_) => "线路",
            FeatureRef::River(_) => "河流",
            FeatureRef::Area(_) => "斜线区域",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureInfo {
    pub what: FeatureRef,
    /// Its name or text; may be empty.
    pub name: String,
    /// A sub-kind worth showing (a symbol's kind), else empty.
    pub detail: String,
    /// Its year range as text, empty when it shows in every year.
    pub years: String,
    pub hidden: bool,
    /// Where it is: the text's or symbol's point, a line's middle, an
    /// area's centre.
    pub at: LonLat,
}

/// The mean direction of some points (their centre on the sphere).
fn centre(pts: &[LonLat]) -> Option<LonLat> {
    let mut s = [0.0f64; 3];
    for p in pts {
        let (lo, la) = (p.lon.to_radians(), p.lat.to_radians());
        s[0] += la.cos() * lo.cos();
        s[1] += la.cos() * lo.sin();
        s[2] += la.sin();
    }
    let n = (s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt();
    (n > 1e-9).then(|| LonLat::new(s[1].atan2(s[0]).to_degrees(), (s[2] / n).asin().to_degrees()))
}

impl Document {
    /// Every drawn feature, by kind (text, symbols, routes, rivers, areas)
    /// and then by name.
    pub fn features(&self) -> Vec<FeatureInfo> {
        let mut out = Vec::new();
        for (id, a) in self.annotations.iter() {
            out.push(FeatureInfo { what: FeatureRef::Text(*id), name: a.text.clone(), detail: String::new(), years: String::new(), hidden: a.hidden, at: a.at });
        }
        for (id, s) in self.symbols.iter() {
            out.push(FeatureInfo { what: FeatureRef::Symbol(*id), name: s.name.clone(), detail: if s.kind == crate::symbol::SymbolKind::Glyph { format!("字符 {}", s.glyph) } else { s.kind.name().into() }, years: s.years_text(), hidden: s.hidden, at: s.at });
        }
        for (id, r) in self.rivers.iter() {
            let Some(at) = (r.pts.len() >= 2).then(|| crate::doc::path_middle(&r.pts)) else { continue };
            let what = if r.route.is_some() { FeatureRef::Route(*id) } else { FeatureRef::River(*id) };
            out.push(FeatureInfo { what, name: r.name.clone(), detail: String::new(), years: String::new(), hidden: r.hidden, at });
        }
        for (id, a) in self.areas.iter() {
            let Some(at) = centre(&a.ring) else { continue };
            out.push(FeatureInfo { what: FeatureRef::Area(*id), name: a.name.clone(), detail: String::new(), years: crate::symbol::years_text(a.from, a.to), hidden: a.hidden, at });
        }
        let rank = |f: &FeatureRef| match f {
            FeatureRef::Text(_) => 0,
            FeatureRef::Symbol(_) => 1,
            FeatureRef::Route(_) => 2,
            FeatureRef::River(_) => 3,
            FeatureRef::Area(_) => 4,
        };
        out.sort_by(|a, b| rank(&a.what).cmp(&rank(&b.what)).then_with(|| a.name.cmp(&b.name)).then_with(|| a.what.cmp(&b.what)));
        out
    }
}

impl Document {
    /// Hide a drawn feature, or show it again.
    pub fn set_hidden(&mut self, what: FeatureRef, hidden: bool) -> crate::topo::TResult<()> {
        let flag = match what {
            FeatureRef::Text(id) => &mut self.annotations.get_mut(&id).ok_or("文字不存在")?.hidden,
            FeatureRef::Symbol(id) => &mut self.symbols.get_mut(&id).ok_or("符号不存在")?.hidden,
            FeatureRef::Route(id) | FeatureRef::River(id) => &mut self.rivers.get_mut(&id).ok_or("线不存在")?.hidden,
            FeatureRef::Area(id) => &mut self.areas.get_mut(&id).ok_or("区域不存在")?.hidden,
        };
        *flag = hidden;
        Ok(())
    }
}

/// Whether a feature matches a filter: empty matches all; otherwise the
/// name, the sub-kind, the kind or the years contain it.
pub fn matches(f: &FeatureInfo, filter: &str) -> bool {
    let q = filter.trim();
    q.is_empty() || f.name.contains(q) || f.detail.contains(q) || f.what.kind_name().contains(q) || f.years.contains(q)
}

/// What "select by attribute" looks for; each part left at its default
/// matches every feature.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FeatureQuery {
    /// Kinds to take, by [`FeatureRef::kind_index`] (all false: every kind).
    pub kinds: [bool; 5],
    /// Text the name or the legend text contains (empty: any).
    pub text: String,
    /// In this layer (`Some(None)`: in no layer).
    pub group: Option<Option<crate::stack::GroupId>>,
    /// Following this named style (`Some(None)`: following none).
    pub style: Option<Option<crate::styles::StyleId>>,
    /// Shown in this year (symbols and areas have years; others always are).
    pub year: Option<i32>,
    /// Hidden or shown.
    pub hidden: Option<bool>,
}

impl FeatureRef {
    /// 0 text, 1 symbol, 2 route, 3 river, 4 area.
    pub fn kind_index(self) -> usize {
        match self {
            FeatureRef::Text(_) => 0,
            FeatureRef::Symbol(_) => 1,
            FeatureRef::Route(_) => 2,
            FeatureRef::River(_) => 3,
            FeatureRef::Area(_) => 4,
        }
    }
}

impl Document {
    /// The legend text a feature has (routes, rivers, areas, symbols).
    pub fn feature_legend(&self, f: FeatureRef) -> String {
        match f {
            FeatureRef::Route(id) | FeatureRef::River(id) => self.rivers.get(&id).map(|r| r.legend.clone()).unwrap_or_default(),
            FeatureRef::Area(id) => self.areas.get(&id).map(|a| a.legend.clone()).unwrap_or_default(),
            FeatureRef::Symbol(id) => self.symbols.get(&id).and_then(|s| s.legend_text()).unwrap_or_default(),
            FeatureRef::Text(_) => String::new(),
        }
    }

    /// The features that match a query, in the feature list's order.
    pub fn query_features(&self, q: &FeatureQuery) -> Vec<FeatureRef> {
        let text = q.text.trim().to_lowercase();
        self.features()
            .into_iter()
            .filter(|f| !q.kinds.iter().any(|k| *k) || q.kinds[f.what.kind_index()])
            .filter(|f| text.is_empty() || f.name.to_lowercase().contains(&text) || self.feature_legend(f.what).to_lowercase().contains(&text))
            .filter(|f| q.group.is_none_or(|g| self.group_of.get(&f.what.id()).copied() == g))
            .filter(|f| q.style.is_none_or(|s| self.style_of.get(&f.what.id()).copied() == s))
            .filter(|f| q.hidden.is_none_or(|h| f.hidden == h))
            .filter(|f| match (q.year, f.what) {
                (None, _) => true,
                (Some(y), FeatureRef::Symbol(id)) => self.symbols.get(&id).is_some_and(|s| s.shown_in(Some(y))),
                (Some(y), FeatureRef::Area(id)) => self.areas.get(&id).is_some_and(|a| a.shown_in(Some(y))),
                _ => true,
            })
            .map(|f| f.what)
            .collect()
    }
}
