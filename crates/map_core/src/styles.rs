//! Named styles for drawn features, and the document's colour swatches.
//!
//! A style is a look with a name: how a route is drawn (colour, width,
//! dashes, arrow, band, teeth...), how a hatched area is filled, how map
//! text or a symbol looks. Features of the right kind can be linked to a
//! style; changing the style changes every feature linked to it.
//!
//! As with curves, the look stays where it always was, in the feature
//! itself (a route's `route`, an area's colour and pattern...), and a
//! linked feature's look is kept equal to its style's. Drawing, export
//! and every other reader therefore work unchanged, and features that are
//! not linked to anything — every feature of a file from before there were
//! styles — keep their own look: a "local style", exactly as before.
//! Links are kept beside the features (`Document::style_of`, by feature id).
//!
//! Editing a linked feature's look edits its style (and so every feature
//! linked to it): [`Document::settle_styles`] carries such a change over.
//! To change one feature alone, unlink it first.

use crate::area::{AreaMark, Pattern};
use crate::doc::{Annotation, Document, LineId, RouteStyle};
use crate::features::FeatureRef;
use crate::symbol::MapSymbol;
use crate::topo::TResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct StyleId(pub u64);

/// How a hatched area is filled and outlined.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct AreaLook {
    pub color: [u8; 4],
    pub spacing: f32,
    pub angle: f32,
    pub tint: bool,
    pub pattern: Pattern,
    pub dashed: bool,
}

/// How map text looks (its size grows and shrinks with the map).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TextLook {
    pub size: f32,
    pub color: [u8; 4],
    pub spacing: f32,
    pub italic: bool,
    /// A font family (none: the default font), and its bold face.
    #[serde(default)]
    pub font: Option<String>,
    #[serde(default)]
    pub bold: bool,
}

/// How a symbol looks (its kind and name stay its own).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct SymbolLook {
    pub size: f32,
    pub color: [u8; 4],
}

/// A look of one kind of feature.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Look {
    Route(RouteStyle),
    Area(AreaLook),
    Text(TextLook),
    Symbol(SymbolLook),
}

/// The kinds of feature a style can be for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StyleKind {
    Route,
    Area,
    Text,
    Symbol,
}

impl StyleKind {
    pub const ALL: [StyleKind; 4] = [StyleKind::Route, StyleKind::Area, StyleKind::Text, StyleKind::Symbol];

    pub fn name(self) -> &'static str {
        match self {
            StyleKind::Route => "线路",
            StyleKind::Area => "斜线区域",
            StyleKind::Text => "地图文字",
            StyleKind::Symbol => "符号",
        }
    }

    /// The kind of style a feature takes (rivers take none: they are
    /// drawn with the map's river style).
    pub fn of(f: FeatureRef) -> Option<StyleKind> {
        match f {
            FeatureRef::Route(_) => Some(StyleKind::Route),
            FeatureRef::Area(_) => Some(StyleKind::Area),
            FeatureRef::Text(_) => Some(StyleKind::Text),
            FeatureRef::Symbol(_) => Some(StyleKind::Symbol),
            FeatureRef::River(_) => None,
        }
    }
}

impl Look {
    pub fn kind(&self) -> StyleKind {
        match self {
            Look::Route(_) => StyleKind::Route,
            Look::Area(_) => StyleKind::Area,
            Look::Text(_) => StyleKind::Text,
            Look::Symbol(_) => StyleKind::Symbol,
        }
    }

    /// Its main colour (for a swatch beside its name).
    pub fn color(&self) -> [u8; 4] {
        match self {
            Look::Route(r) => r.color,
            Look::Area(a) => a.color,
            Look::Text(t) => t.color,
            Look::Symbol(s) => s.color,
        }
    }

    pub fn of_area(a: &AreaMark) -> Self {
        Look::Area(AreaLook { color: a.color, spacing: a.spacing, angle: a.angle, tint: a.tint, pattern: a.pattern, dashed: a.dashed })
    }

    pub fn of_text(a: &Annotation) -> Self {
        Look::Text(TextLook { size: a.size, color: a.color, spacing: a.spacing, italic: a.italic, font: a.font.clone(), bold: a.bold })
    }

    pub fn of_symbol(s: &MapSymbol) -> Self {
        Look::Symbol(SymbolLook { size: s.size, color: s.color })
    }
}

/// A named style.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NamedStyle {
    pub id: StyleId,
    pub name: String,
    pub look: Look,
}

/// A named colour of the document's palette.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Swatch {
    pub name: String,
    pub color: [u8; 4],
}

/// Styles a new map can start from ("添加常用样式").
pub fn presets() -> Vec<(String, Look)> {
    use crate::deco::Deco;
    use crate::doc::Dash;
    let route = |color: [u8; 4], width: f32, dash: Dash, arrow: bool| RouteStyle { color, width, dash, arrow, broad: false, hollow: false, deco: Deco::None, deco_flip: false };
    let area = |color: [u8; 4], pattern: Pattern, tint: bool, dashed: bool| AreaLook { color, spacing: 8.0, angle: 45.0, tint, pattern, dashed };
    let text = |size: f32, color: [u8; 4], spacing: f32, italic: bool| TextLook { size, color, spacing, italic, font: None, bold: false };
    vec![
        ("进军路线".into(), Look::Route(route([170, 40, 40, 255], 2.5, Dash::Dashed, true))),
        ("大军进攻".into(), Look::Route(RouteStyle { broad: true, width: 14.0, ..route([190, 40, 30, 255], 14.0, Dash::Solid, true) })),
        ("撤退".into(), Look::Route(RouteStyle { broad: true, hollow: true, width: 14.0, ..route([60, 60, 60, 255], 14.0, Dash::Dashed, true) })),
        ("前线".into(), Look::Route(RouteStyle { deco: Deco::Teeth, ..route([150, 30, 30, 255], 2.0, Dash::Solid, false) })),
        ("铁路".into(), Look::Route(RouteStyle { deco: Deco::Ticks, ..route([50, 50, 50, 255], 1.6, Dash::Solid, false) })),
        ("商路".into(), Look::Route(route([150, 100, 40, 255], 2.0, Dash::Dotted, false))),
        ("航线".into(), Look::Route(route([40, 80, 150, 255], 1.6, Dash::Dashed, true))),
        ("争议地区".into(), Look::Area(area([150, 30, 30, 255], Pattern::Lines, false, true))),
        ("占领区".into(), Look::Area(area([90, 90, 90, 255], Pattern::Cross, true, false))),
        ("势力范围".into(), Look::Area(area([40, 90, 160, 255], Pattern::Dots, true, true))),
        ("海洋名称".into(), Look::Text(text(18.0, [40, 70, 110, 255], 0.5, true))),
        ("山脉名称".into(), Look::Text(text(13.0, [110, 80, 50, 255], 1.0, false))),
        ("地区名称".into(), Look::Text(text(14.0, [60, 60, 60, 255], 0.8, false))),
    ]
}

impl Document {
    pub fn style(&self, s: StyleId) -> Option<&NamedStyle> {
        self.styles.iter().find(|x| x.id == s)
    }

    /// The style a feature is linked to.
    pub fn style_of_feature(&self, f: FeatureRef) -> Option<StyleId> {
        self.style_of.get(&f.id()).copied().filter(|s| self.style(*s).is_some())
    }

    /// The features linked to a style.
    pub fn style_users(&self, s: StyleId) -> Vec<FeatureRef> {
        self.features().into_iter().map(|f| f.what).filter(|f| self.style_of.get(&f.id()) == Some(&s)).collect()
    }

    /// A feature's look, if it has one that a style can hold.
    pub fn feature_look(&self, f: FeatureRef) -> Option<Look> {
        match f {
            FeatureRef::Route(id) => self.rivers.get(&id).and_then(|r| r.route).map(Look::Route),
            FeatureRef::Area(id) => self.areas.get(&id).map(Look::of_area),
            FeatureRef::Text(id) => self.annotations.get(&id).map(Look::of_text),
            FeatureRef::Symbol(id) => self.symbols.get(&id).map(Look::of_symbol),
            FeatureRef::River(_) => None,
        }
    }

    /// Give a feature a look (it must be of its kind); the rest of the
    /// feature (its place, name, years...) is untouched.
    pub fn set_feature_look(&mut self, f: FeatureRef, look: &Look) -> TResult<()> {
        match (f, look) {
            (FeatureRef::Route(id), Look::Route(st)) => {
                let r = self.rivers.get_mut(&id).ok_or("线路不存在")?;
                if r.route.is_none() {
                    return Err("这是河流，不是线路".into());
                }
                r.route = Some(*st);
            }
            (FeatureRef::Area(id), Look::Area(l)) => {
                let a = self.areas.get_mut(&id).ok_or("区域不存在")?;
                (a.color, a.spacing, a.angle, a.tint, a.pattern, a.dashed) = (l.color, l.spacing, l.angle, l.tint, l.pattern, l.dashed);
            }
            (FeatureRef::Text(id), Look::Text(l)) => {
                let a = self.annotations.get_mut(&id).ok_or("文字不存在")?;
                (a.size, a.color, a.spacing, a.italic, a.font, a.bold) = (l.size, l.color, l.spacing, l.italic, l.font.clone(), l.bold);
            }
            (FeatureRef::Symbol(id), Look::Symbol(l)) => {
                let s = self.symbols.get_mut(&id).ok_or("符号不存在")?;
                (s.size, s.color) = (l.size, l.color);
            }
            _ => return Err(format!("“{}”的样式不能用于{}", look.kind().name(), f.kind_name())),
        }
        Ok(())
    }

    /// A new style, at the end of its kind's list.
    pub fn add_style(&mut self, name: &str, look: Look) -> StyleId {
        let id = StyleId(self.topo.alloc());
        let name = if name.trim().is_empty() { format!("{}样式 {}", look.kind().name(), self.styles.iter().filter(|s| s.look.kind() == look.kind()).count() + 1) } else { name.trim().to_string() };
        self.styles.push(NamedStyle { id, name, look });
        id
    }

    /// A new style from a feature's look; the feature is linked to it.
    pub fn style_from_feature(&mut self, f: FeatureRef, name: &str) -> TResult<StyleId> {
        let look = self.feature_look(f).ok_or_else(|| format!("{}没有可保存为样式的外观", f.kind_name()))?;
        let id = self.add_style(name, look);
        self.style_of.insert(f.id(), id);
        Ok(id)
    }

    /// Link features to a style: each takes its look. Features of another
    /// kind are left alone; returns how many were linked.
    pub fn link_style(&mut self, fs: &[FeatureRef], s: StyleId) -> TResult<usize> {
        let look = self.style(s).ok_or("样式不存在")?.look.clone();
        let mut n = 0;
        for f in fs {
            if StyleKind::of(*f) == Some(look.kind()) && self.set_feature_look(*f, &look).is_ok() {
                self.style_of.insert(f.id(), s);
                n += 1;
            }
        }
        if n == 0 && !fs.is_empty() {
            return Err(format!("所选要素中没有{}，不能使用这个样式", look.kind().name()));
        }
        Ok(n)
    }

    /// Unlink features from their styles; their look stays as it is (a
    /// local style from now on).
    pub fn unlink_style(&mut self, fs: &[FeatureRef]) {
        for f in fs {
            self.style_of.remove(&f.id());
        }
    }

    /// Change a style's look: every feature linked to it takes it.
    pub fn update_style(&mut self, s: StyleId, look: Look) -> TResult<usize> {
        let st = self.styles.iter_mut().find(|x| x.id == s).ok_or("样式不存在")?;
        if st.look.kind() != look.kind() {
            return Err("样式的种类不能改变".into());
        }
        st.look = look.clone();
        let users = self.style_users(s);
        for f in &users {
            self.set_feature_look(*f, &look)?;
        }
        Ok(users.len())
    }

    /// Give a style the look of one of the features (which is linked to it
    /// then too): "update the style from this one".
    pub fn redefine_style(&mut self, s: StyleId, from: FeatureRef) -> TResult<usize> {
        let look = self.feature_look(from).ok_or("这个要素没有外观")?;
        let n = self.update_style(s, look)?;
        self.style_of.insert(from.id(), s);
        Ok(n.max(1))
    }

    pub fn rename_style(&mut self, s: StyleId, name: &str) -> TResult<()> {
        if name.trim().is_empty() {
            return Err("样式名称不能为空".into());
        }
        self.styles.iter_mut().find(|x| x.id == s).ok_or("样式不存在")?.name = name.trim().to_string();
        Ok(())
    }

    /// Remove a style; its features keep their look (as local styles).
    /// Returns how many there were.
    pub fn remove_style(&mut self, s: StyleId) -> TResult<usize> {
        let i = self.styles.iter().position(|x| x.id == s).ok_or("样式不存在")?;
        self.styles.remove(i);
        let before = self.style_of.len();
        self.style_of.retain(|_, x| *x != s);
        Ok(before - self.style_of.len())
    }

    /// A copy of a style, with no features yet.
    pub fn duplicate_style(&mut self, s: StyleId) -> TResult<StyleId> {
        let st = self.style(s).ok_or("样式不存在")?.clone();
        Ok(self.add_style(&format!("{} 副本", st.name), st.look.clone()))
    }

    /// Move a style to place `to` in the list (the order is the user's).
    pub fn move_style(&mut self, s: StyleId, to: usize) -> TResult<()> {
        let i = self.styles.iter().position(|x| x.id == s).ok_or("样式不存在")?;
        let x = self.styles.remove(i);
        self.styles.insert(to.min(self.styles.len()), x);
        Ok(())
    }

    /// Give features another feature's look (the eyedropper), and its link
    /// to a style if it has one; features of another kind are left alone.
    /// Returns how many took it.
    pub fn copy_look(&mut self, from: FeatureRef, to: &[FeatureRef]) -> TResult<usize> {
        let look = self.feature_look(from).ok_or_else(|| format!("{}没有可吸取的外观", from.kind_name()))?;
        let link = self.style_of_feature(from);
        let mut n = 0;
        for f in to.iter().filter(|f| **f != from) {
            if StyleKind::of(*f) != Some(look.kind()) || self.set_feature_look(*f, &look).is_err() {
                continue;
            }
            match link {
                Some(s) => self.style_of.insert(f.id(), s),
                None => self.style_of.remove(&f.id()),
            };
            n += 1;
        }
        Ok(n)
    }

    /// Carry over edits made to linked features' looks: a style whose
    /// linked features no longer all look like it takes the look of the
    /// first that differs (by id), and gives it to all of them. Returns
    /// the styles that changed. The interface calls this after each frame
    /// of editing, so a change made in a feature's own panel changes its
    /// style; changes made through the methods above never need it.
    pub fn settle_styles(&mut self) -> Vec<StyleId> {
        let mut changed = Vec::new();
        let links: Vec<(LineId, StyleId)> = self.style_of.iter().map(|(f, s)| (*f, *s)).collect();
        for (id, s) in links {
            let Some(st) = self.style(s).map(|x| x.look.clone()) else { continue };
            let f = match st.kind() {
                StyleKind::Route => FeatureRef::Route(id),
                StyleKind::Area => FeatureRef::Area(id),
                StyleKind::Text => FeatureRef::Text(id),
                StyleKind::Symbol => FeatureRef::Symbol(id),
            };
            match self.feature_look(f) {
                Some(l) if l != st => {
                    if self.update_style(s, l).is_ok() {
                        changed.push(s);
                    }
                }
                Some(_) => {}
                // gone, or no longer of the style's kind (a route made a river)
                None => {
                    self.style_of.remove(&id);
                }
            }
        }
        changed
    }

    /// Forget links of features or styles that are gone.
    pub fn prune_styles(&mut self) {
        let ids = self.feature_ids();
        let styles: std::collections::BTreeSet<StyleId> = self.styles.iter().map(|s| s.id).collect();
        self.style_of.retain(|f, s| ids.contains(f) && styles.contains(s));
    }

    /// Add the usual styles that are not there yet (by name); returns how
    /// many were added.
    pub fn add_preset_styles(&mut self) -> usize {
        let mut n = 0;
        for (name, look) in presets() {
            if !self.styles.iter().any(|s| s.name == name && s.look.kind() == look.kind()) {
                self.add_style(&name, look);
                n += 1;
            }
        }
        n
    }

    // ------------------------------------------------------------ swatches

    /// Add a colour to the palette (a colour already there is not added
    /// twice); returns its place.
    pub fn add_swatch(&mut self, name: &str, color: [u8; 4]) -> usize {
        if let Some(i) = self.swatches.iter().position(|s| s.color == color) {
            return i;
        }
        let name = if name.trim().is_empty() { format!("颜色 {}", self.swatches.len() + 1) } else { name.trim().to_string() };
        self.swatches.push(Swatch { name, color });
        self.swatches.len() - 1
    }

    pub fn remove_swatch(&mut self, i: usize) -> TResult<()> {
        if i >= self.swatches.len() {
            return Err("色板中没有这个颜色".into());
        }
        self.swatches.remove(i);
        Ok(())
    }

    /// Every colour used by the styles and features, most used first (to
    /// fill the palette from what the map already uses).
    pub fn colors_in_use(&self) -> Vec<([u8; 4], usize)> {
        let mut count: std::collections::BTreeMap<[u8; 4], usize> = std::collections::BTreeMap::new();
        for s in &self.styles {
            *count.entry(s.look.color()).or_default() += 1;
        }
        for f in self.features() {
            if let Some(l) = self.feature_look(f.what) {
                *count.entry(l.color()).or_default() += 1;
            }
        }
        let mut v: Vec<([u8; 4], usize)> = count.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        v
    }
}
