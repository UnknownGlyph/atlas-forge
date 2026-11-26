//! Working with several drawn features at once (map text, symbols, rivers
//! and routes, hatched areas): moving, turning and scaling them together
//! on the sphere, aligning and spacing them, deleting, and copying them
//! out of and into a document. The interface's multiple selection, its
//! clipboard and transform panel are thin layers over these; so will be a
//! script's or an assistant's edits.

use crate::area::AreaMark;
use crate::copy::moved;
use crate::doc::{Annotation, Document, LineFeature, LineId};
use crate::features::FeatureRef;
use crate::geom::{lon_delta, wrap_lon, LonLat};
use crate::symbol::MapSymbol;
use crate::topo::TResult;
use serde::{Deserialize, Serialize};

/// A feature's content, detached from any document (what a clipboard holds).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum FeatureData {
    Text(Annotation),
    Symbol(MapSymbol),
    Line(LineFeature),
    Area(AreaMark),
}

/// Features copied out of a document (`map_io::clip` writes it as text
/// for the system clipboard, so it can be pasted into another window).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    /// Marks the text as ours, with the format's version.
    pub atlas_forge_features: u32,
    pub items: Vec<FeatureData>,
    /// The curves of items drawn with one, by the item's place in `items`
    /// (older clipboard text has none).
    #[serde(default)]
    pub curves: std::collections::BTreeMap<usize, crate::curve::Curve>,
    /// The styles items were linked to, by place in `items`, with the
    /// style's look then: pasted into a document that has that style with
    /// that look, the copy is linked to it too.
    #[serde(default)]
    pub styles: std::collections::BTreeMap<usize, (crate::styles::StyleId, crate::styles::Look)>,
}

fn to_vec(p: LonLat) -> [f64; 3] {
    let (lo, la) = (p.lon.to_radians(), p.lat.to_radians());
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

fn from_vec(v: [f64; 3]) -> LonLat {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-300);
    LonLat::new(v[1].atan2(v[0]).to_degrees(), (v[2] / n).clamp(-1.0, 1.0).asin().to_degrees())
}

/// The centre of some points on the sphere (their mean direction).
pub fn centre_of(pts: &[LonLat]) -> Option<LonLat> {
    let mut s = [0.0f64; 3];
    for p in pts {
        let v = to_vec(*p);
        s = [s[0] + v[0], s[1] + v[1], s[2] + v[2]];
    }
    ((s[0] * s[0] + s[1] * s[1] + s[2] * s[2]).sqrt() > 1e-9).then(|| from_vec(s))
}

/// `p` turned about `centre` by `deg` degrees, counter-clockwise as seen
/// on a north-up map. Distances from the centre are kept.
pub fn rotate_about(p: LonLat, centre: LonLat, deg: f64) -> LonLat {
    let (k, v) = (to_vec(centre), to_vec(p));
    let (s, c) = deg.to_radians().sin_cos();
    let dot = k[0] * v[0] + k[1] * v[1] + k[2] * v[2];
    let cross = [k[1] * v[2] - k[2] * v[1], k[2] * v[0] - k[0] * v[2], k[0] * v[1] - k[1] * v[0]];
    // Rodrigues' rotation about the centre's direction
    from_vec([0, 1, 2].map(|i| v[i] * c + cross[i] * s + k[i] * dot * (1.0 - c)))
}

/// `p` moved along the great circle through `centre` so that its distance
/// from the centre is `factor` times what it was (at most to the far side).
pub fn scale_about(p: LonLat, centre: LonLat, factor: f64) -> LonLat {
    let (k, v) = (to_vec(centre), to_vec(p));
    let dot = (k[0] * v[0] + k[1] * v[1] + k[2] * v[2]).clamp(-1.0, 1.0);
    let d = dot.acos();
    if d < 1e-12 {
        return p;
    }
    // the unit vector at the centre pointing towards p
    let t = [0, 1, 2].map(|i| (v[i] - k[i] * dot) / d.sin().max(1e-300));
    let nd = (d * factor).clamp(0.0, std::f64::consts::PI - 1e-9);
    from_vec([0, 1, 2].map(|i| k[i] * nd.cos() + t[i] * nd.sin()))
}

/// How to line features up: on the westernmost, middle or easternmost
/// longitude of their centres, or the northernmost, middle or southernmost
/// latitude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    West,
    MidLon,
    East,
    North,
    MidLat,
    South,
}

impl Document {
    pub fn feature_exists(&self, f: FeatureRef) -> bool {
        match f {
            FeatureRef::Text(id) => self.annotations.contains_key(&id),
            FeatureRef::Symbol(id) => self.symbols.contains_key(&id),
            FeatureRef::Route(id) | FeatureRef::River(id) => self.rivers.contains_key(&id),
            FeatureRef::Area(id) => self.areas.contains_key(&id),
        }
    }

    /// Every point of a feature (an area's holes and pieces too).
    pub fn feature_points(&self, f: FeatureRef) -> Vec<LonLat> {
        match f {
            FeatureRef::Text(id) => self.annotations.get(&id).map(|a| std::iter::once(a.at).chain(a.path.iter().copied()).collect()),
            FeatureRef::Symbol(id) => self.symbols.get(&id).map(|s| vec![s.at]),
            FeatureRef::Route(id) | FeatureRef::River(id) => self.rivers.get(&id).map(|r| r.pts.clone()),
            FeatureRef::Area(id) => self.areas.get(&id).map(|a| a.ring.iter().chain(a.extra.iter().flatten()).copied().collect()),
        }
        .unwrap_or_default()
    }

    /// Where a feature is: its point, or the centre of its points.
    pub fn feature_anchor(&self, f: FeatureRef) -> Option<LonLat> {
        match f {
            FeatureRef::Text(id) => self.annotations.get(&id).map(|a| a.at),
            FeatureRef::Symbol(id) => self.symbols.get(&id).map(|s| s.at),
            _ => centre_of(&self.feature_points(f)),
        }
    }

    /// The centre of several features together.
    pub fn features_centre(&self, fs: &[FeatureRef]) -> Option<LonLat> {
        centre_of(&fs.iter().flat_map(|f| self.feature_points(*f)).collect::<Vec<_>>())
    }

    /// Put every point of a feature through `m`.
    pub(crate) fn map_feature(&mut self, f: FeatureRef, m: &dyn Fn(LonLat) -> LonLat) -> TResult<()> {
        let fix = |p: LonLat| {
            let q = m(p);
            LonLat::new(wrap_lon(q.lon), q.lat.clamp(-89.999, 89.999))
        };
        match f {
            FeatureRef::Text(id) => {
                let a = self.annotations.get_mut(&id).ok_or("文字不存在")?;
                a.at = fix(a.at);
                a.path = a.path.iter().map(|p| fix(*p)).collect();
            }
            FeatureRef::Symbol(id) => {
                let s = self.symbols.get_mut(&id).ok_or("符号不存在")?;
                s.at = fix(s.at);
            }
            FeatureRef::Route(id) | FeatureRef::River(id) => {
                let r = self.rivers.get_mut(&id).ok_or("线不存在")?;
                r.pts = r.pts.iter().map(|p| fix(*p)).collect();
            }
            FeatureRef::Area(id) => {
                let a = self.areas.get_mut(&id).ok_or("区域不存在")?;
                a.ring = a.ring.iter().map(|p| fix(*p)).collect();
                a.extra = a.extra.iter().map(|r| r.iter().map(|p| fix(*p)).collect()).collect();
            }
        }
        // a curve moves, turns and scales with its feature; its points are
        // worked out again from it (the same shape, now exactly on the curve)
        if let Some(mut c) = self.curves.get(&f.id()).cloned() {
            c.map(&fix);
            self.set_curve(f.id(), c)?;
        }
        Ok(())
    }

    /// Move features together: the turn of the sphere that takes `from`
    /// to `to`, so shapes and their distances to each other are kept.
    pub fn move_features(&mut self, fs: &[FeatureRef], from: LonLat, to: LonLat) -> TResult<()> {
        for f in fs {
            self.map_feature(*f, &|p| moved(&[p], from, to)[0])?;
        }
        Ok(())
    }

    /// Turn features about `centre` by `deg` degrees counter-clockwise.
    /// Straight map text turns with them.
    pub fn rotate_features(&mut self, fs: &[FeatureRef], centre: LonLat, deg: f64) -> TResult<()> {
        if !deg.is_finite() {
            return Err("角度无效".into());
        }
        for f in fs {
            self.map_feature(*f, &|p| rotate_about(p, centre, deg))?;
            if let FeatureRef::Text(id) = f {
                let a = self.annotations.get_mut(id).ok_or("文字不存在")?;
                if !a.on_path() {
                    a.angle = (a.angle + deg as f32).rem_euclid(360.0);
                }
            }
        }
        Ok(())
    }

    /// Make features `factor` times as large about `centre` (their points'
    /// distances from it; sizes of text and symbols are not changed).
    pub fn scale_features(&mut self, fs: &[FeatureRef], centre: LonLat, factor: f64) -> TResult<()> {
        if !(factor.is_finite() && factor > 0.0) {
            return Err("缩放比例须为正数".into());
        }
        for f in fs {
            self.map_feature(*f, &|p| scale_about(p, centre, factor))?;
        }
        Ok(())
    }

    /// Put a feature's content back (as it was copied by
    /// [`Document::copy_features`]); the kinds must agree.
    pub fn set_feature(&mut self, f: FeatureRef, data: &FeatureData) -> TResult<()> {
        match (f, data.clone()) {
            (FeatureRef::Text(id), FeatureData::Text(a)) => self.annotations.insert(id, a).map(|_| ()),
            (FeatureRef::Symbol(id), FeatureData::Symbol(s)) => self.symbols.insert(id, s).map(|_| ()),
            (FeatureRef::Route(id) | FeatureRef::River(id), FeatureData::Line(l)) => self.rivers.insert(id, l).map(|_| ()),
            (FeatureRef::Area(id), FeatureData::Area(a)) => self.areas.insert(id, a).map(|_| ()),
            _ => return Err("要素种类不符".into()),
        };
        Ok(())
    }

    /// Put the `i`th feature of a clipboard back into `f`, its curve too
    /// (or none, if it had none).
    pub fn set_feature_from(&mut self, f: FeatureRef, clip: &Clip, i: usize) -> TResult<()> {
        self.set_feature(f, clip.items.get(i).ok_or("剪贴板里没有这一项")?)?;
        match clip.styles.get(&i) {
            Some((s, _)) if self.style(*s).is_some() => self.style_of.insert(f.id(), *s),
            _ => self.style_of.remove(&f.id()),
        };
        match clip.curves.get(&i) {
            Some(c) => self.set_curve(f.id(), c.clone()),
            None => {
                self.curves.remove(&f.id());
                Ok(())
            }
        }
    }

    /// Delete features; returns how many there were.
    pub fn delete_features(&mut self, fs: &[FeatureRef]) -> usize {
        let mut n = 0;
        let board = crate::world_book::WorldBook::current(self);
        for f in fs {
            let removed = match f {
                FeatureRef::Text(id) => self.annotations.remove(id).is_some(),
                FeatureRef::Symbol(id) => self.symbols.remove(id).is_some(),
                FeatureRef::Route(id) | FeatureRef::River(id) => self.rivers.remove(id).is_some(),
                FeatureRef::Area(id) => self.areas.remove(id).is_some(),
            };
            if removed {
                n += 1;
                if let Some(book) = &mut self.book {
                    std::sync::Arc::make_mut(book).unlink(crate::world_book::feature(board, *f));
                }
            }
            self.locked.remove(&f.id());
            self.group_of.remove(&f.id());
            self.curves.remove(&f.id());
            self.style_of.remove(&f.id());
        }
        n
    }

    /// The content of features, for a clipboard.
    pub fn copy_features(&self, fs: &[FeatureRef]) -> Clip {
        let mut items = Vec::new();
        let mut curves = std::collections::BTreeMap::new();
        let mut styles = std::collections::BTreeMap::new();
        for f in fs {
            let data = match f {
                FeatureRef::Text(id) => self.annotations.get(id).cloned().map(FeatureData::Text),
                FeatureRef::Symbol(id) => self.symbols.get(id).cloned().map(FeatureData::Symbol),
                FeatureRef::Route(id) | FeatureRef::River(id) => self.rivers.get(id).cloned().map(FeatureData::Line),
                FeatureRef::Area(id) => self.areas.get(id).cloned().map(FeatureData::Area),
            };
            if let Some(d) = data {
                if let Some(c) = self.curves.get(&f.id()) {
                    curves.insert(items.len(), c.clone());
                }
                if let Some(s) = self.style_of_feature(*f).and_then(|s| self.style(s)) {
                    styles.insert(items.len(), (s.id, s.look.clone()));
                }
                items.push(d);
            }
        }
        Clip { atlas_forge_features: 1, items, curves, styles }
    }

    /// Add copied features as new ones and return them. With `to`, they
    /// are moved together so that their centre is there; without, they go
    /// where they were copied from.
    pub fn paste_features(&mut self, clip: &Clip, to: Option<LonLat>) -> TResult<Vec<FeatureRef>> {
        let mut out = Vec::new();
        for (i, item) in clip.items.iter().enumerate() {
            let id = LineId(self.topo.alloc());
            if let Some(c) = clip.curves.get(&i) {
                self.curves.insert(id, c.clone());
            }
            out.push(match item.clone() {
                FeatureData::Text(a) => {
                    self.annotations.insert(id, a);
                    FeatureRef::Text(id)
                }
                FeatureData::Symbol(s) => {
                    self.symbols.insert(id, s);
                    FeatureRef::Symbol(id)
                }
                FeatureData::Line(l) => {
                    let route = l.route.is_some();
                    self.rivers.insert(id, l);
                    if route {
                        FeatureRef::Route(id)
                    } else {
                        FeatureRef::River(id)
                    }
                }
                FeatureData::Area(a) => {
                    self.areas.insert(id, a);
                    FeatureRef::Area(id)
                }
            });
        }
        self.share_pasted_pixel_art(&out);
        // the copies follow the same styles, where this document has them as they were
        for (i, (s, look)) in &clip.styles {
            if let (Some(f), Some(st)) = (out.get(*i), self.style(*s)) {
                if st.look == *look && self.feature_look(*f).as_ref() == Some(look) {
                    self.style_of.insert(f.id(), *s);
                }
            }
        }
        if let (Some(to), Some(from)) = (to, self.features_centre(&out)) {
            self.move_features(&out, from, to)?;
        }
        Ok(out)
    }

    /// Clipboard text carries every pixel mark's picture in full, so each
    /// pasted mark would hold its own copy. A pasted picture the document
    /// already has (or another pasted mark has) uses that one instead, as a
    /// reopened project does: repeated marks keep one picture in memory and
    /// one texture in the app's and the PNG renderer's caches (both keyed by
    /// the picture's address).
    fn share_pasted_pixel_art(&mut self, pasted: &[FeatureRef]) {
        let marks: Vec<LineId> = pasted.iter().filter_map(|f| match f {
            FeatureRef::Symbol(id) if self.symbols.get(id).is_some_and(|s| s.pixels.is_some()) => Some(*id),
            _ => None,
        }).collect();
        if marks.is_empty() {
            return;
        }
        let new: std::collections::BTreeSet<LineId> = marks.iter().copied().collect();
        let mut known: std::collections::HashSet<std::sync::Arc<crate::pixel_art::PixelArt>> =
            self.symbols.iter().filter(|(id, _)| !new.contains(id)).filter_map(|(_, s)| s.pixels.clone()).collect();
        for id in marks {
            let Some(art) = self.symbols.get(&id).and_then(|s| s.pixels.clone()) else { continue };
            match known.get(&art) {
                Some(same) if !std::sync::Arc::ptr_eq(same, &art) => {
                    let same = std::sync::Arc::clone(same);
                    if let Some(s) = self.symbols.get_mut(&id) {
                        s.pixels = Some(same);
                    }
                }
                Some(_) => {}
                None => {
                    known.insert(art);
                }
            }
        }
    }

    /// Copies of features a little away (`dlon`, `dlat` degrees), returned.
    pub fn duplicate_features(&mut self, fs: &[FeatureRef], dlon: f64, dlat: f64) -> TResult<Vec<FeatureRef>> {
        let clip = self.copy_features(fs);
        let from = self.features_centre(fs).ok_or("没有可复制的要素")?;
        let to = LonLat::new(wrap_lon(from.lon + dlon), (from.lat + dlat).clamp(-89.0, 89.0));
        // the copies stay in their originals' layers
        let sources: Vec<FeatureRef> = fs.iter().copied().filter(|f| self.feature_exists(*f)).collect();
        let new = self.paste_features(&clip, Some(to))?;
        for (n, s) in new.iter().zip(&sources) {
            if let Some(g) = self.group_of_feature(*s) {
                self.group_of.insert(n.id(), g);
            }
        }
        Ok(new)
    }

    /// Line features up (see [`Align`]); each moves whole, keeping its shape.
    pub fn align_features(&mut self, fs: &[FeatureRef], how: Align) -> TResult<()> {
        let anchors: Vec<(FeatureRef, LonLat)> = fs.iter().filter_map(|f| self.feature_anchor(*f).map(|a| (*f, a))).collect();
        let Some(first) = anchors.first().map(|a| a.1) else { return Err("没有可对齐的要素".into()) };
        if anchors.len() < 2 {
            return Err("至少选择两个要素才能对齐".into());
        }
        // longitudes continuous around the first, so a group across the
        // antimeridian lines up where it is
        let lon = |p: LonLat| first.lon + lon_delta(first.lon, p.lon);
        let lons: Vec<f64> = anchors.iter().map(|a| lon(a.1)).collect();
        let lats: Vec<f64> = anchors.iter().map(|a| a.1.lat).collect();
        let ((w, e), (s, n)) = (min_max(&lons), min_max(&lats));
        for (f, a) in anchors {
            let to = match how {
                Align::West => LonLat::new(w, a.lat),
                Align::MidLon => LonLat::new((w + e) * 0.5, a.lat),
                Align::East => LonLat::new(e, a.lat),
                Align::North => LonLat::new(a.lon, n),
                Align::MidLat => LonLat::new(a.lon, (s + n) * 0.5),
                Align::South => LonLat::new(a.lon, s),
            };
            self.move_features(&[f], a, LonLat::new(wrap_lon(to.lon), to.lat))?;
        }
        Ok(())
    }

    /// Space features evenly between the two outermost: by longitude
    /// (`by_lon`) or by latitude. Each moves whole along that direction.
    pub fn distribute_features(&mut self, fs: &[FeatureRef], by_lon: bool) -> TResult<()> {
        let mut anchors: Vec<(FeatureRef, LonLat)> = fs.iter().filter_map(|f| self.feature_anchor(*f).map(|a| (*f, a))).collect();
        if anchors.len() < 3 {
            return Err("至少选择三个要素才能均匀分布".into());
        }
        let first = anchors[0].1;
        let key = |p: LonLat| if by_lon { first.lon + lon_delta(first.lon, p.lon) } else { p.lat };
        anchors.sort_by(|a, b| key(a.1).total_cmp(&key(b.1)));
        let (lo, hi) = (key(anchors[0].1), key(anchors[anchors.len() - 1].1));
        let n = anchors.len() - 1;
        for (i, (f, a)) in anchors.into_iter().enumerate() {
            let v = lo + (hi - lo) * i as f64 / n as f64;
            let to = if by_lon { LonLat::new(wrap_lon(v), a.lat) } else { LonLat::new(a.lon, v) };
            self.move_features(&[f], a, to)?;
        }
        Ok(())
    }

    /// Whether a feature is locked (it cannot be picked or moved on the map).
    pub fn is_locked(&self, f: FeatureRef) -> bool {
        self.locked.contains(&f.id())
    }

    /// Lock a feature, or unlock it.
    pub fn set_locked(&mut self, f: FeatureRef, locked: bool) -> TResult<()> {
        if !self.feature_exists(f) {
            return Err("要素不存在".into());
        }
        let id = f.id();
        if locked {
            self.locked.insert(id);
        } else {
            self.locked.remove(&id);
        }
        Ok(())
    }
}

fn min_max(v: &[f64]) -> (f64, f64) {
    (v.iter().copied().fold(f64::INFINITY, f64::min), v.iter().copied().fold(f64::NEG_INFINITY, f64::max))
}
