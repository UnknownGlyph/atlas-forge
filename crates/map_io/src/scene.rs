//! Projection-aware vector scene used by the SVG and PNG exporters.
//! Built directly from the document (not from screen pixels), so exports
//! are resolution-independent and keep real geometry.

use map_core::doc::{Document, EntityId, EntityKind, PlanarPolygon};
use map_core::labels::LabelPath;
use map_core::geom::{self, P2};
use map_core::projection::Projection;
use map_core::theme::ThemeResult;
use map_core::topo::{FaceId, FaceKind, BACKGROUND};
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub struct ExportOptions {
    pub width: u32,
    pub height: u32,
    pub projection: Projection,
    pub lon0: f64,
    /// Region in projected units [xmin, ymin, xmax, ymax]; None = whole world.
    pub bounds: Option<[f64; 4]>,
    pub labels: bool,
    pub legend: bool,
    pub graticule: bool,
    pub background: [u8; 4],
    pub title: String,
    /// A scale bar (true at the image centre) in the lower right.
    pub scale_bar: bool,
    /// An arrow pointing north at the image centre, above the scale bar.
    pub north_arrow: bool,
    /// Where the furniture sits and the inset maps (default: the usual places, none).
    pub placement: map_core::layout::Placement,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            width: 2400,
            height: 1300,
            projection: Projection::EqualEarth,
            lon0: 0.0,
            bounds: None,
            labels: true,
            legend: true,
            graticule: true,
            background: [255, 255, 255, 255],
            title: String::new(),
            scale_bar: false,
            north_arrow: false,
            placement: Default::default(),
        }
    }
}

impl ExportOptions {
    /// The options a saved layout stands for (white background).
    pub fn from_layout(l: &map_core::layout::Layout) -> Self {
        Self {
            width: l.width,
            height: l.height,
            projection: l.projection,
            lon0: l.lon0,
            bounds: l.bounds,
            labels: l.labels,
            legend: l.legend,
            graticule: l.graticule,
            background: [255, 255, 255, 255],
            title: l.title.clone(),
            scale_bar: l.scale_bar,
            north_arrow: l.north_arrow,
            placement: l.placement.clone(),
        }
    }

    /// These options saved as a layout called `name`.
    pub fn to_layout(&self, name: &str) -> map_core::layout::Layout {
        map_core::layout::Layout {
            name: name.into(),
            width: self.width,
            height: self.height,
            projection: self.projection,
            lon0: self.lon0,
            bounds: self.bounds,
            title: self.title.clone(),
            labels: self.labels,
            legend: self.legend,
            graticule: self.graticule,
            scale_bar: self.scale_bar,
            north_arrow: self.north_arrow,
            placement: self.placement.clone(),
        }
    }
}

pub struct ScenePoly {
    pub fill: [u8; 4],
    pub rings: Vec<Vec<P2>>,
}

#[derive(Clone, Debug)]
pub struct SceneLine {
    pub color: [u8; 4],
    pub width: f32,
    pub pts: Vec<P2>,
    /// Dash and gap lengths (round caps); `None` is solid.
    pub dash: Option<[f32; 2]>,
}

pub struct SceneLabel {
    pub text: String,
    /// Centre x and baseline y.
    pub pos: P2,
    pub size: f32,
    pub color: [u8; 4],
    /// Degrees, counter-clockwise.
    pub angle: f32,
    pub italic: bool,
    /// A font family (none: the default font), and its bold face.
    pub font: Option<String>,
    pub bold: bool,
}

/// What a legend row's swatch shows: a colour (thematic classes,
/// countries), or a sample of a route, hatched area or symbol.
#[derive(Clone, Debug, PartialEq)]
pub enum LegendKey {
    Color([u8; 4]),
    Route(map_core::doc::RouteStyle),
    Area { color: [u8; 4], spacing: f32, angle: f32, tint: bool, dashed: bool, pattern: map_core::area::Pattern },
    Symbol(map_core::symbol::SymbolKind, [u8; 4]),
    /// A character symbol: the character and its colour.
    Glyph(String, [u8; 4]),
    Pixels(std::sync::Arc<map_core::pixel_art::PixelArt>),
}

/// One drawn part of a legend swatch.
pub enum LegendPart {
    Rect([f32; 4], [u8; 4]),
    Line(SceneLine),
    Fill(Vec<P2>, [u8; 4]),
    /// Text from its left end on its baseline: (text, [x, y], size, colour).
    Text(String, [f32; 2], f32, [u8; 4]),
}

pub struct LegendRow {
    pub parts: Vec<LegendPart>,
    /// Text start (left, baseline).
    pub text: [f32; 2],
    pub label: String,
}

/// One screen-space pixel mark, retaining its shared pixels until rendering.
pub struct PixelMark {
    pub art: std::sync::Arc<map_core::pixel_art::PixelArt>,
    pub rect: [f32; 4],
    pub opacity: f32,
}

impl PixelMark {
    pub fn runs(&self) -> impl Iterator<Item = ([f32; 4], [u8; 4])> + '_ {
        pixel_rects(&self.art, self.rect).map(|(rect, color)| (rect, faded(color, self.opacity)))
    }
}

fn pixel_rects(art: &map_core::pixel_art::PixelArt, rect: [f32; 4]) -> impl Iterator<Item = ([f32; 4], [u8; 4])> + '_ {
    let [x, y, w, h] = rect;
    let [cols, rows] = art.size().map(f32::from);
    let (dx, dy) = (w / cols, h / rows);
    art.runs().map(move |([start, end, row], color)| (
        [x + f32::from(start) * dx, y + f32::from(row) * dy, f32::from(end - start) * dx, dy], color
    ))
}

/// The shapes of one layer of drawn features, its opacity already in
/// their colours.
pub struct SceneLayer {
    pub name: String,
    pub polys: Vec<ScenePoly>,
    pub lines: Vec<SceneLine>,
    pub fills: Vec<(Vec<P2>, [u8; 4])>,
    pub pixels: Vec<PixelMark>,
}

/// A colour made more see-through (`k` from 0 to 1).
fn faded(c: [u8; 4], k: f32) -> [u8; 4] {
    [c[0], c[1], c[2], (c[3] as f32 * k.clamp(0.0, 1.0)).round() as u8]
}

pub struct Scene {
    pub width: u32,
    pub height: u32,
    pub background: [u8; 4],
    pub polys: Vec<ScenePoly>,
    /// Bands along the coasts (a map theme's look): drawn over the first
    /// polygon (the sea) and under the rest.
    pub glow: Vec<SceneLine>,
    /// The terrain's hill shading over the land, a picture the size of
    /// the scene (straight-alpha RGBA): drawn over the areas, under the lines.
    pub shade: Option<Vec<u8>>,
    pub lines: Vec<SceneLine>,
    pub labels: Vec<SceneLabel>,
    pub dots: Vec<(P2, f32, [u8; 4])>,
    /// Filled polygons over the lines (route arrowheads, symbols); any
    /// shape, filled with the nonzero rule.
    pub fills: Vec<(Vec<P2>, [u8; 4])>,
    pub pixels: Vec<PixelMark>,
    /// The layers of drawn features, from the bottom of the stack up:
    /// laid over everything above, each whole (its washes, then its
    /// lines, then its filled shapes) before the next.
    pub layers: Vec<SceneLayer>,
    /// Title and rows (label, what the swatch shows).
    pub legend: Option<(String, Vec<(String, LegendKey)>)>,
    pub title: String,
    /// The title's box, top left (px), when it is moved from the top centre.
    pub title_at: Option<[f32; 2]>,
    /// The legend's top left (px), when it is moved from the lower left.
    pub legend_at: Option<[f32; 2]>,
    /// Each piece of furniture as drawn: its box [x, y, w, h] (px). For the
    /// layout view, which lets them be dragged.
    pub furniture: Vec<(Furniture, [f32; 4])>,
    /// Inset maps, drawn over the map (under the legend and the title).
    pub insets: Vec<SceneInset>,
    /// The scale bar and the north arrow: a transparent scene of their own,
    /// drawn over the insets (so an inset never hides them).
    pub top: Option<Box<Scene>>,
}

/// A scene with nothing in it.
pub fn empty_scene(width: u32, height: u32, background: [u8; 4]) -> Scene {
    Scene {
        width,
        height,
        background,
        polys: vec![],
        glow: vec![],
        shade: None,
        lines: vec![],
        labels: vec![],
        dots: vec![],
        fills: vec![],
        pixels: vec![],
        layers: vec![],
        legend: None,
        title: String::new(),
        title_at: None,
        legend_at: None,
        furniture: vec![],
        insets: vec![],
        top: None,
    }
}

/// A piece of a page's furniture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Furniture {
    Title,
    Legend,
    ScaleBar,
    NorthArrow,
    /// The inset at this index of the placement's list.
    Inset(usize),
}

/// An inset map: its frame on the page (px) and its own scene.
pub struct SceneInset {
    pub rect: [f32; 4],
    pub scene: Box<Scene>,
    pub title: String,
}

/// The title's size scale, as the legend's: 1 up to 1600 px wide, larger
/// on larger pages (a fixed size was tiny on a big page).
pub fn title_scale(s: &Scene) -> f32 {
    (s.width as f32 / 1600.0).clamp(1.0, 3.0)
}

/// The title's box [x, y, w, h] (px), where it is written (its left end
/// and baseline) and its size; None without a title. Shared by the exporters.
pub fn title_box(s: &Scene) -> Option<([f32; 4], [f32; 2], f32)> {
    if s.title.is_empty() {
        return None;
    }
    let k = title_scale(s);
    let size = 28.0 * k;
    let tw = text_width(&s.title, size);
    let (x, y) = match s.title_at {
        Some([x, y]) => (x, y),
        None => (s.width as f32 / 2.0 - tw * 0.5 - 6.0 * k, 10.0 * k),
    };
    Some(([x, y, tw + 12.0 * k, 38.0 * k], [x + 6.0 * k, y + 30.0 * k], size))
}

/// Approximate text width (CJK ≈ 1 em, Latin ≈ 0.56 em); shared by both
/// exporters so layout decisions are identical.
pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars().map(|c| if c.is_ascii() { 0.56 } else { 1.0 }).sum::<f32>() * size
}

/// Split rings (relative unwrapped lon) into parts inside lon ∈ [-180, 180].
pub fn split_strip(rings: &[Vec<P2>]) -> Vec<Vec<Vec<P2>>> {
    map_core::seam::split(rings)
}

/// Split an open polyline (relative unwrapped lon) into pieces inside the strip.
pub fn split_polyline(pts: &[P2]) -> Vec<Vec<P2>> {
    let mut out = Vec::new();
    let b = geom::bbox(pts);
    for k in [-720.0, -360.0, 0.0, 360.0, 720.0] {
        if b[2] + k < -180.0 || b[0] + k > 180.0 {
            continue;
        }
        let mut cur: Vec<P2> = Vec::new();
        for w in pts.windows(2) {
            let (mut a, mut c) = ([w[0][0] + k, w[0][1]], [w[1][0] + k, w[1][1]]);
            let clip = |a: &mut P2, c: &mut P2| -> bool {
                for (edge, keep_greater) in [(-180.0, true), (180.0, false)] {
                    let ins = |p: &P2| if keep_greater { p[0] >= edge } else { p[0] <= edge };
                    match (ins(a), ins(c)) {
                        (true, true) => {}
                        (false, false) => return false,
                        (ia, _) => {
                            let t = (edge - a[0]) / (c[0] - a[0]);
                            let m = [edge, a[1] + (c[1] - a[1]) * t];
                            if ia {
                                *c = m;
                            } else {
                                *a = m;
                            }
                        }
                    }
                }
                true
            };
            let starts_inside = a[0] >= -180.0 && a[0] <= 180.0;
            if clip(&mut a, &mut c) {
                if cur.is_empty() || !starts_inside {
                    if cur.len() >= 2 {
                        out.push(std::mem::take(&mut cur));
                    }
                    cur = vec![a];
                }
                cur.push(c);
            } else if cur.len() >= 2 {
                out.push(std::mem::take(&mut cur));
            } else {
                cur.clear();
            }
        }
        if cur.len() >= 2 {
            out.push(cur);
        }
    }
    out
}

fn densify(pts: &[P2], closed: bool, step: f64) -> Vec<P2> {
    let n = pts.len();
    let segs = if closed { n } else { n.saturating_sub(1) };
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..segs {
        let (a, b) = (pts[i], pts[(i + 1) % n]);
        let d = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let k = ((d / step).ceil() as usize).max(1);
        for j in 0..k {
            let t = j as f64 / k as f64;
            out.push([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]);
        }
    }
    if !closed {
        if let Some(l) = pts.last() {
            out.push(*l);
        }
    }
    out
}

struct Mapper {
    proj: Projection,
    lon0: f64,
    scale: f64,
    cx: f64,
    cy: f64,
    w: f64,
    h: f64,
}

impl Mapper {
    fn new(o: &ExportOptions) -> Self {
        let b = o.bounds.unwrap_or_else(|| o.projection.bounds());
        let (w, h) = (o.width as f64, o.height as f64);
        let scale = (w / (b[2] - b[0])).min(h / (b[3] - b[1]));
        Mapper { proj: o.projection, lon0: o.lon0, scale, cx: (b[0] + b[2]) * 0.5, cy: (b[1] + b[3]) * 0.5, w, h }
    }
    fn px(&self, rel: P2) -> P2 {
        let p = self.proj.forward_deg(rel[0], rel[1]);
        [(p[0] - self.cx) * self.scale + self.w * 0.5, self.h * 0.5 - (p[1] - self.cy) * self.scale]
    }
    fn rel(&self, ring: &[P2]) -> Vec<P2> {
        ring.iter().map(|p| [p[0] - self.lon0, p[1]]).collect()
    }
}

/// Where a lon/lat point lands in the image made with `o`: the same
/// transform the scene is drawn with, so positions reported next to an
/// exported image match it.
pub struct PixelMap(Mapper);

impl PixelMap {
    pub fn new(o: &ExportOptions) -> Self {
        PixelMap(Mapper::new(o))
    }

    /// Pixel position (x right, y down) of a point.
    pub fn px(&self, p: map_core::LonLat) -> P2 {
        self.0.px([geom::wrap_lon(p.lon - self.0.lon0), p.lat])
    }
}

pub fn build_scene(doc: &Document, theme: &ThemeResult, o: &ExportOptions) -> Scene {
    let paper_opts;
    let o = if let Some(paper) = doc.paper {
        paper_opts = ExportOptions {
            projection: Projection::Equirectangular,
            lon0: 0.0,
            bounds: Some(paper.bounds()),
            graticule: false,
            scale_bar: false,
            north_arrow: false,
            ..o.clone()
        };
        &paper_opts
    } else { o };
    let m = Mapper::new(o);
    let (w, h, scale) = (m.w, m.h, m.scale);
    let st = &doc.style;
    let blank = || empty_scene(o.width, o.height, o.background);
    let mut s = blank();
    s.title.clone_from(&o.title);

    // ocean = whole projected world
    let world = densify(&[[-180.0, -90.0], [180.0, -90.0], [180.0, 90.0], [-180.0, 90.0]], true, 1.0);
    s.polys.push(ScenePoly { fill: theme.face_colors.get(&BACKGROUND).copied().unwrap_or(st.ocean), rings: vec![world.iter().map(|p| m.px(*p)).collect()] });

    let add_poly = |s: &mut Scene, poly: &PlanarPolygon, fill: [u8; 4]| {
        let mut rings = vec![m.rel(&poly.outer)];
        rings.extend(poly.holes.iter().map(|hh| m.rel(hh)));
        for part in split_strip(&rings) {
            let rings = part.iter().map(|r| densify(r, true, 1.0).iter().map(|p| m.px(*p)).collect()).collect();
            s.polys.push(ScenePoly { fill, rings });
        }
    };
    // owned land, dissolved per entity (no hairline seams between provinces)
    // (on a share layer every face has its own mixed colour: no dissolving)
    let shares = doc.active_is_shares();
    let mut owners: Vec<_> = if shares { vec![] } else { doc.owner.values().copied().collect::<HashSet<_>>().into_iter().collect() };
    owners.sort();
    for e in &owners {
        let fill = theme.entity_colors.get(e).copied().unwrap_or(st.land);
        for poly in doc.dissolve(*e) {
            add_poly(&mut s, &poly, fill);
        }
    }
    // unowned land, then water bodies on top
    for kind in [FaceKind::Land, FaceKind::Water] {
        for (fid, f) in doc.topo.faces.iter() {
            if *fid == BACKGROUND || f.kind != kind || (kind == FaceKind::Land && !shares && doc.owner.contains_key(fid)) {
                continue;
            }
            let (outer, holes) = doc.topo.face_planar(*fid);
            let fill = theme.face_colors.get(fid).copied().unwrap_or(st.land);
            add_poly(&mut s, &PlanarPolygon { outer, holes }, fill);
        }
    }

    let add_line = |s: &mut Scene, pts: &[P2], color: [u8; 4], width: f32| {
        for piece in split_polyline(&m.rel(pts)) {
            let d = densify(&piece, false, 1.0);
            s.lines.push(SceneLine { color, width, pts: d.iter().map(|p| m.px(*p)).collect(), dash: None });
        }
    };
    if o.graticule {
        for i in 0..12 {
            let lon = -180.0 + 30.0 * i as f64 + o.lon0;
            add_line(&mut s, &[[lon, -90.0], [lon, 90.0]], [255, 255, 255, 110], 0.6);
        }
        for i in 1..6 {
            let lat = -90.0 + 30.0 * i as f64;
            add_line(&mut s, &[[o.lon0 - 180.0, lat], [o.lon0 + 180.0, lat]], [255, 255, 255, 110], 0.6);
        }
    }
    let kind = |f: FaceId| doc.topo.faces[&f].kind;
    let k = (o.width as f32 / 1600.0).max(1.0);
    let mut by_rank = [Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut ancestry = std::collections::HashMap::<EntityId, Vec<EntityId>>::new();
    for (_, edge) in &doc.topo.edges {
        if doc.topo.on_pole(edge) {
            continue;
        }
        let rank = match (kind(edge.left), kind(edge.right)) {
            (FaceKind::Water, FaceKind::Water) => continue,
            (FaceKind::Land, FaceKind::Land) => {
                let (a, b) = (doc.owner.get(&edge.left), doc.owner.get(&edge.right));
                if a == b {
                    if st.hide_face_seams_in_export { continue; }
                    2
                } else if let (Some(&a), Some(&b)) = (a,b) {
                    for id in [a,b] {
                        ancestry.entry(id).or_insert_with(|| doc.ancestors(id).collect());
                    }
                    let shared = ancestry[&a].iter().rev().zip(ancestry[&b].iter().rev())
                        .take_while(|(a,b)| a == b).count();
                    match shared { 0 => 4, 1 => 2, 2 => 1, _ => 0 }
                } else {
                    4
                }
            }
            _ => 3,
        };
        if rank >= 3 || st.internal_width > 0.0 {
            by_rank[rank].push(edge);
        }
    }
    for (class, edges) in by_rank.into_iter().enumerate() {
        for e in edges {
            let (c, wd) = match class {
                0 => (st.internal, st.internal_width * 0.43),
                1 => (st.internal, st.internal_width * 0.68),
                2 => (st.internal, st.internal_width),
                3 => (st.coast, st.coast_width),
                _ => (st.border, st.border_width),
            };
            let pts = geom::unwrap_path(&doc.topo.edge_full(e), None);
            let from = s.lines.len();
            add_line(&mut s, &pts, c, wd * k);
            // a map theme's bands along the coast, widest first (sorted below)
            if class == 3 && st.coast_glow_width > 0.0 {
                let pieces: Vec<Vec<P2>> = s.lines[from..].iter().map(|l| l.pts.clone()).collect();
                for (c, f) in st.glow_bands().into_iter().zip([2.0, 4.0 / 3.0, 2.0 / 3.0]) {
                    for p in &pieces {
                        s.glow.push(SceneLine { color: c, width: st.coast_glow_width * f * k, pts: p.clone(), dash: None });
                    }
                }
            }
        }
    }
    // the bands: all the widest first, the narrowest last
    s.glow.sort_by(|a, b| b.width.total_cmp(&a.width));
    for guide in doc.guides.iter().filter(|g| g.visible && g.include_export && g.valid()) {
        let path = geom::unwrap_path(&guide.path(o.lon0), None);
        add_line(&mut s, &path, guide.color, 1.0 * k);
        if o.labels && !guide.name.is_empty() {
            let at = guide.label_position(o.lon0);
            let p = m.px([geom::wrap_lon(at.lon - o.lon0), at.lat]);
            if p[0] >= 0.0 && p[0] <= w as f64 && p[1] >= 0.0 && p[1] <= h as f64 {
                s.labels.push(SceneLabel { text: guide.name.clone(), pos: [p[0], p[1] - 3.0 * k as f64],
                    size: 10.0 * k, color: guide.color, angle: 0.0, italic: false, font: None, bold: false });
            }
        }
    }
    // drawn features, layer by layer. Those in no layer go straight into
    // the scene, where they always were; each layer is gathered apart and
    // laid over that, from the bottom of the stack up, as see-through as
    // the layer is.
    for pass in doc.draw_passes() {
        let in_pass = |id: &map_core::doc::LineId| doc.group_of_id(*id) == pass.group;
        let mut own = blank();
        let to: &mut Scene = if pass.group.is_none() { &mut s } else { &mut own };
        for (_, r) in doc.rivers.iter().filter(|(id, r)| r.route.is_none() && !r.hidden && in_pass(id)) {
            add_line(to, &geom::unwrap_path(&r.pts, None), st.river, st.river_width * k);
        }
        // routes over rivers and borders, styled, the arrow on the last piece
        for (_, r) in doc.rivers.iter().filter(|(id, r)| !r.hidden && in_pass(id)) {
            let Some(rs) = r.route else { continue };
            let width = rs.width * k;
            let pieces = split_polyline(&m.rel(&geom::unwrap_path(&r.pts, None)));
            let last = pieces.len().saturating_sub(1);
            for (i, piece) in pieces.into_iter().enumerate() {
                let mut px: Vec<P2> = densify(&piece, false, 1.0).iter().map(|p| m.px(*p)).collect();
                if rs.broad {
                    let f: Vec<[f32; 2]> = px.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                    if let Some(outline) = map_core::broad::broad_arrow(&f, width, rs.arrow && i == last) {
                        let poly: Vec<P2> = outline.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
                        let mut ring = poly.clone();
                        ring.push(poly[0]);
                        let (edge, dash) = rs.broad_edge(k);
                        if !rs.hollow {
                            to.fills.push((poly, rs.broad_fill()));
                        }
                        to.lines.push(SceneLine { color: rs.color, width: edge, pts: ring, dash });
                    }
                    continue;
                }
                if rs.arrow && i == last {
                    let f: Vec<[f32; 2]> = px.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                    if let Some((line, head)) = map_core::doc::arrow_end(&f, width) {
                        px = line.iter().map(|p| [p[0] as f64, p[1] as f64]).collect();
                        to.fills.push((head.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), rs.color));
                    }
                }
                // a front line's teeth, a railway's ties, along the (trimmed) line
                let f: Vec<[f32; 2]> = px.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                for mk in map_core::deco::decorate(&f, rs.deco, width, rs.deco_flip) {
                    match mk {
                        map_core::deco::DecoMark::Fill(t) => to.fills.push((t.iter().map(|p| [p[0] as f64, p[1] as f64]).collect(), rs.color)),
                        map_core::deco::DecoMark::Stroke([a, b]) => to.lines.push(SceneLine { color: rs.color, width, pts: vec![[a[0] as f64, a[1] as f64], [b[0] as f64, b[1] as f64]], dash: None }),
                    }
                }
                to.lines.push(SceneLine { color: rs.color, width, pts: px, dash: rs.pattern(width) });
            }
        }
        // hatched areas shown in the document's year: wash, stripes, outline
        for (_, a) in doc.areas.iter().filter(|(id, a)| a.shown_in(doc.year) && a.ring.len() >= 3 && in_pass(id)) {
            let un = geom::unwrap_path(&a.ring, None);
            // room for the name at the middle (where the label section writes it)
            let gap = (!a.name.is_empty())
                .then(|| {
                    let px: Vec<[f32; 2]> = densify(&m.rel(&un), true, 1.0)
                        .iter()
                        .map(|p| {
                            let q = m.px(*p);
                            [q[0] as f32, q[1] as f32]
                        })
                        .collect();
                    map_core::area::centre(&px)
                })
                .flatten()
                .map(|[cx, cy]| {
                    let size = map_core::area::NAME_PX * k;
                    let hw = text_width(&a.name, size) * 0.5 + 4.0 * k;
                    [cx - hw, cy - size * 0.65, cx + hw, cy + size * 0.65]
                });
            // every ring (outline, pieces, holes), each cut at the seam, filled
            // together even-odd so pieces fill and holes stay open
            let all: Vec<Vec<P2>> = a.rings().into_iter().map(|r| geom::unwrap_path(r, None)).collect();
            let mut rings: Vec<Vec<P2>> = Vec::new();
            for r in &all {
                for part in split_strip(&[m.rel(r)]) {
                    rings.extend(part.iter().map(|x| densify(x, true, 1.0).iter().map(|p| m.px(*p)).collect::<Vec<P2>>()));
                }
            }
            if a.fills() && !rings.is_empty() {
                to.polys.push(ScenePoly { fill: a.tint_color(), rings: rings.clone() });
            }
            let f: Vec<Vec<[f32; 2]>> = rings.iter().map(|r| r.iter().map(|p| [p[0] as f32, p[1] as f32]).collect()).collect();
            let (stripes, dots) = map_core::area::fill_pattern(&f, a.pattern, a.spacing * k, a.angle, gap);
            for [p, q] in stripes {
                to.lines.push(SceneLine { color: a.color, width: map_core::area::HATCH_WIDTH * k, pts: vec![[p[0] as f64, p[1] as f64], [q[0] as f64, q[1] as f64]], dash: None });
            }
            for p in dots {
                to.fills.push((dot_poly(p, map_core::area::DOT_RADIUS * k), a.color));
            }
            if a.pattern == map_core::area::Pattern::Solid { continue; }
            // the outlines along the rings themselves, broken at the seam (not along the cut)
            let width = map_core::area::OUTLINE_WIDTH * k;
            for r in &all {
                let mut closed = r.clone();
                closed.push(r[0]);
                for piece in split_polyline(&m.rel(&closed)) {
                    let pts = densify(&piece, false, 1.0).iter().map(|p| m.px(*p)).collect();
                    to.lines.push(SceneLine { color: a.color, width, pts, dash: a.dashed.then_some([7.0 * k, 4.0 * k + width]) });
                }
            }
        }
        // symbols shown in the document's year, drawn from the shared marks
        for (_, sym) in doc.symbols.iter().filter(|(id, x)| x.shown_in(doc.year) && in_pass(id)) {
            let p = m.px([geom::wrap_lon(sym.at.lon - o.lon0), sym.at.lat]);
            if let Some(art) = &sym.pixels {
                let [w, h] = sym.drawn_size(k);
                to.pixels.push(PixelMark {
                    art: art.clone(), rect: [p[0] as f32 - w * 0.5, p[1] as f32 - h * 0.5, w, h],
                    opacity: 1.0,
                });
                continue;
            }
            if sym.kind == map_core::symbol::SymbolKind::Glyph {
                // a character symbol is text, centred on its point
                let size = map_core::symbol::glyph_px(sym.size * k);
                to.labels.push(SceneLabel { text: sym.glyph.clone(), pos: [p[0], p[1] + size as f64 * 0.35], size, color: sym.color, angle: 0.0, italic: false, font: None, bold: false });
                continue;
            }
            for mk in map_core::symbol::marks(sym.kind, [p[0] as f32, p[1] as f32], sym.size * k, sym.color) {
                let px = |v: &[[f32; 2]]| v.iter().map(|q| [q[0] as f64, q[1] as f64]).collect::<Vec<P2>>();
                match mk {
                    map_core::symbol::Mark::Fill { outline, color, .. } => to.fills.push((px(&outline), color)),
                    map_core::symbol::Mark::Stroke { pts, closed, width, color } => {
                        let mut v = px(&pts);
                        if closed {
                            v.push(v[0]);
                        }
                        to.lines.push(SceneLine { color, width, pts: v, dash: None });
                    }
                }
            }
        }
        if let Some(g) = pass.group {
            let k = pass.opacity;
            let name = doc.group(g).map(|x| x.name.clone()).unwrap_or_default();
            s.layers.push(SceneLayer {
                name,
                polys: own.polys.into_iter().map(|p| ScenePoly { fill: faded(p.fill, k), rings: p.rings }).collect(),
                lines: own.lines.into_iter().map(|l| SceneLine { color: faded(l.color, k), ..l }).collect(),
                fills: own.fills.into_iter().map(|(p, c)| (p, faded(c, k))).collect(),
                pixels: own.pixels.into_iter().map(|item| PixelMark { opacity: k, ..item }).collect(),
            });
            // character symbols are text: with the other labels, on top
            s.labels.extend(own.labels.into_iter().map(|l| SceneLabel { color: faded(l.color, k), ..l }));
        }
    }
    // a second partition layer's boundaries, over everything else
    if let Some(l) = doc.overlay_layer() {
        for id in doc.layer_border_edges(l) {
            let pts = geom::unwrap_path(&doc.topo.edge_full(&doc.topo.edges[&id]), None);
            add_line(&mut s, &pts, st.overlay_color, st.overlay_width * k);
        }
    }

    let mut boxes: Vec<[f32; 4]> = Vec::new();
    // where the labels of each feature begin, and the opacity of its layer
    let mut fades: Vec<(usize, f32)> = Vec::new();
    for icon in &theme.data_icons {
        let p = m.px([geom::wrap_lon(icon.at.lon - o.lon0), icon.at.lat]);
        let radius = icon.size * k;
        let ring: Vec<_> = icon.icon.vertices(radius).into_iter()
            .map(|[x, y]| [p[0] + f64::from(x), p[1] + f64::from(y)]).collect();
        s.fills.push((ring, icon.color));
        if o.labels && !icon.label.is_empty() {
            let size = (radius * 1.2).max(9.0 * k);
            let center = p[0] as f32;
            let y = p[1] as f32 - radius - 2.0;
            let half = text_width(&icon.label, size) * 0.5;
            let bounds = [center - half, y - size, center + half, y + 2.0];
            if !boxes.iter().any(|q| !(bounds[2] < q[0] || q[2] < bounds[0] || bounds[3] < q[1] || q[3] < bounds[1])) {
                s.labels.push(SceneLabel {
                    text: icon.label.clone(), pos: [p[0], p[1] - radius as f64 - 2.0],
                    size, color: icon.color, angle: 0.0, italic: false, font: None, bold: false,
                });
                boxes.push(bounds);
            }
        }
    }
    // free text first: always shown, every other label keeps clear of it
    if o.labels {
        // sizes are geographic: as given when the whole world is 1600 px wide
        let zoom = (scale * std::f64::consts::PI / 180.0 / map_core::doc::Annotation::REF_PX_PER_DEG) as f32;
        for (id, a) in doc.annotations.iter().filter(|(id, a)| !a.hidden && !doc.group_hides(**id)) {
            fades.push((s.labels.len(), doc.group_opacity(*id)));
            let p = m.px([geom::wrap_lon(a.at.lon - o.lon0), a.at.lat]);
            let size = a.size * zoom;
            let text = a.spaced();
            // a curved line goes glyph by glyph along its arc or its drawn curve
            let along = a.on_path() || a.curved();
            let glyphs: Vec<String> = if along { text.chars().map(String::from).collect() } else { vec![text] };
            let widths: Vec<f32> = glyphs.iter().map(|g| text_width(g, size)).collect();
            let (sa, ca) = a.angle.to_radians().sin_cos();
            // each glyph's centre on the image and its turn (degrees, counter-clockwise)
            let on_curve = a.on_path().then(|| longest_px_piece(&m, &a.path)).flatten().and_then(|path| map_core::path_text::along_path(&path, &widths, 0.0));
            let places: Vec<(f32, f32, f32)> = match on_curve {
                Some(v) => v.into_iter().map(|([x, y], turn)| (x, y, turn)).collect(),
                // local y is up; on the image y is down
                None => map_core::doc::arc_layout(&widths, a.bend).into_iter().map(|([x, y], turn)| (p[0] as f32 + x * ca - y * sa, p[1] as f32 - (x * sa + y * ca), a.angle + turn)).collect(),
            };
            let mut bx = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
            for ((g, tw), (cx, cy, ang)) in glyphs.into_iter().zip(&widths).zip(places) {
                let (c, sn) = (ang.to_radians().cos().abs(), ang.to_radians().sin().abs());
                let (hw, hh) = ((tw * c + size * sn) * 0.5 + 2.0, (tw * sn + size * c) * 0.5 + 2.0);
                bx = [bx[0].min(cx - hw), bx[1].min(cy - hh), bx[2].max(cx + hw), bx[3].max(cy + hh)];
                if !g.trim().is_empty() {
                    s.labels.push(SceneLabel { text: g, pos: [cx as f64, (cy + size * 0.35) as f64], size, color: a.color, angle: ang, italic: a.italic, font: a.font.clone(), bold: a.bold });
                }
            }
            if bx[0] <= bx[2] {
                boxes.push(bx);
            }
        }
        // hatched areas' names at their middle, in italics
        for (id, a) in doc.areas.iter().filter(|(id, a)| a.shown_in(doc.year) && !a.name.is_empty() && a.ring.len() >= 3 && !doc.group_hides(**id)) {
            fades.push((s.labels.len(), doc.group_opacity(*id)));
            let px: Vec<[f32; 2]> = densify(&m.rel(&geom::unwrap_path(&a.ring, None)), true, 1.0)
                .iter()
                .map(|p| {
                    let q = m.px(*p);
                    [q[0] as f32, q[1] as f32]
                })
                .collect();
            let Some([cx, cy]) = map_core::area::centre(&px) else { continue };
            let size = map_core::area::NAME_PX * k;
            let tw = text_width(&a.name, size);
            boxes.push([cx - tw * 0.5 - 2.0, cy - size * 0.65, cx + tw * 0.5 + 2.0, cy + size * 0.65]);
            s.labels.push(SceneLabel { text: a.name.clone(), pos: [cx as f64, (cy + size * 0.35) as f64], size, color: a.color, angle: 0.0, italic: true, font: None, bold: false });
        }
        // symbol names beside their symbols; labels drawn after keep clear
        for (id, sym) in doc.symbols.iter().filter(|(id, x)| x.shown_in(doc.year) && !doc.group_hides(**id)) {
            fades.push((s.labels.len(), doc.group_opacity(*id)));
            let p = m.px([geom::wrap_lon(sym.at.lon - o.lon0), sym.at.lat]);
            let (cx, cy) = (p[0] as f32, p[1] as f32);
            let [rx, ry] = sym.drawn_size(k).map(|v| v * 0.5);
            boxes.push([cx - rx - 2.0, cy - ry - 2.0, cx + rx + 2.0, cy + ry + 2.0]);
            if sym.name.is_empty() {
                continue;
            }
            let size = (sym.size * 1.5).max(9.0) * k;
            let (tw, th) = (text_width(&sym.name, size), size * 1.2);
            let width_or_height = match sym.name_place {
                map_core::symbol::NamePlace::Left | map_core::symbol::NamePlace::Right => rx,
                _ => ry,
            };
            let [x, y] = sym.name_place.top_left([cx, cy], width_or_height, tw, th, 3.0 * k);
            boxes.push([x - 2.0, y, x + tw + 2.0, y + th]);
            s.labels.push(SceneLabel { text: sym.name.clone(), pos: [(x + tw * 0.5) as f64, (y + th * 0.5 + size * 0.35) as f64], size, color: sym.color, angle: 0.0, italic: false, font: None, bold: false });
        }
        // country names next, before the names of rivers and routes: they
        // matter more on a map of countries
        if st.show_labels {
            fades.push((s.labels.len(), 1.0));
            // country names: along the line through each country's land (as in
            // the application), bent glyph by glyph; placed ones where put
            let kk = k.min(2.0);
            let (min_px, max_px, min_fit) = (9.0 * kk, 38.0 * kk, 8.5 * kk);
            struct Candidate<'a> {
                depth: usize,
                name: &'a str,
                path: Option<LabelPath>,
                place: Option<map_core::doc::LabelPlace>,
                manual: bool,
            }
            let mut cands = Vec::<Candidate<'_>>::new();
            for id in doc.countries() {
                let e = &doc.entities[&id];
                // no label in a year the country does not exist; a unit under
                // another is named with its country
                if e.label_hidden || !e.exists_in(doc.year) || e.name.trim().is_empty()
                    || (!st.admin_typography && e.parent.is_some() && e.label.is_none()) {
                    continue;
                }
                let depth = doc.ancestors(id).count().saturating_sub(1);
                let path = doc.label_path(id);
                let place = e.label.or_else(|| {
                    if !st.admin_typography || depth == 0 { return None; }
                    let (at, free) = doc.label_anchor(id)?;
                    let projected = m.scale * free.to_radians();
                    (projected > 10.0).then(|| map_core::doc::LabelPlace::at(at))
                });
                if place.is_none() && path.is_none() {
                    continue;
                }
                cands.push(Candidate { depth, name:e.name.trim(), path, place, manual:e.label.is_some() });
            }
            // placed labels first: they always show, automatic ones keep clear of them
            cands.sort_by(|a, b| {
                let tier = if st.admin_typography { a.depth.cmp(&b.depth) } else { std::cmp::Ordering::Equal };
                tier.then(b.manual.cmp(&a.manual)).then_with(|| {
                    b.path.as_ref().map_or(0.0, |p| p.fit()).total_cmp(&a.path.as_ref().map_or(0.0, |p| p.fit()))
                })
            });
            let px_of = |ll: map_core::geom::LonLat| {
                let q = m.px([geom::wrap_lon(ll.lon - o.lon0), ll.lat]);
                [q[0] as f32, q[1] as f32]
            };
            // the city dots, for automatic names to keep off where they can
            let dots: Vec<[f32; 4]> = if st.show_cities {
                let r = 2.5 * k + 1.5;
                doc.entities
                    .values()
                    .filter(|e| e.kind == EntityKind::City && e.exists_in(doc.year))
                    .filter_map(|e| e.pos)
                    .map(|p| {
                        let q = px_of(p);
                        [q[0] - r, q[1] - r, q[0] + r, q[1] + r]
                    })
                    .collect()
            } else {
                vec![]
            };
            // a glyph's box, turned
            let glyph_box = |cx: f32, cy: f32, tw: f32, size: f32, ang: f32| {
                let (c, sn) = (ang.to_radians().cos().abs(), ang.to_radians().sin().abs());
                let (hw, hh) = ((tw * c + size * sn) * 0.5 + 1.5, (tw * sn + size * c) * 0.5 + 1.5);
                [cx - hw, cy - hh, cx + hw, cy + hh]
            };
            for Candidate { depth, name: text, path, place, manual } in cands {
                let face = st.label_type(map_core::label_type::Tier::administrative(depth));
                let font = || face.font.map(str::to_owned);
                let em = map_core::labels::name_em(text) as f32;
                // the line on the image (its longest piece, left to right), its
                // length and the room across it, and how large the name can be there
                let line = path.as_ref().and_then(|p| longest_px_piece(&m, &p.line(32)).map(|l| (p, l)));
                let fitted = line.as_ref().map(|(p, l)| {
                    let len: f32 = l.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
                    let mid = p.length() * 0.5;
                    let (a, b) = (px_of(p.point(mid, p.height * 0.5)), px_of(p.point(mid, -p.height * 0.5)));
                    let room = (a[0] - b[0]).hypot(a[1] - b[1]);
                    (len, map_core::labels::name_size(len as f64, room as f64, em as f64) as f32, room)
                });
                match place {
                    Some(pl) => {
                        let base = fitted.map_or(face.size * kk, |(_, f, _)| f.clamp(min_px, max_px));
                        let size = base * pl.scale;
                        let tw = text_width(text, size);
                        let p = px_of(pl.at);
                        // the box of the turned label, for the automatic ones to avoid
                        let b = glyph_box(p[0], p[1], tw, size, pl.angle);
                        if st.admin_typography && !manual && (b[0] < 0.0 || b[1] < 0.0 || b[2] > w as f32
                            || b[3] > h as f32 || boxes.iter().any(|q| !(b[2] < q[0] || q[2] < b[0] || b[3] < q[1] || q[3] < b[1]))) {
                            continue;
                        }
                        boxes.push(b);
                        let (sa, ca) = pl.angle.to_radians().sin_cos();
                        s.labels.push(SceneLabel { text:text.to_owned(), pos: [(p[0] + 0.35 * size * sa) as f64, (p[1] + 0.35 * size * ca) as f64], size, color: st.label, angle: pl.angle, italic: false, font:font(), bold:face.bold });
                    }
                    None => {
                        let (Some((_, l)), Some((len, fit, room))) = (line, fitted) else { continue };
                        if fit < min_fit {
                            continue;
                        }
                        let size0 = fit.clamp(min_px, max_px);
                        let glyphs: Vec<String> = text.chars().map(String::from).collect();
                        let inside = |b: &[f32; 4]| b[0] >= 0.0 && b[1] >= 0.0 && b[2] <= w as f32 && b[3] <= h as f32;
                        let meets = |b: &[f32; 4], q: &[f32; 4]| !(b[2] < q[0] || q[2] < b[0] || b[3] < q[1] || q[3] < b[1]);
                        // spread along its line in the middle; where that meets a
                        // river's or a symbol's name or free text: slid along the
                        // line or off it to either side (within the land's room
                        // across), set closer, or a little smaller — and off the
                        // city dots too where it can be (as in the application)
                        let spread = map_core::labels::name_spacing(len as f64, text_width(&text, size0) as f64, size0 as f64, glyphs.len()) as f32;
                        let mut ways = vec![(spread, size0)];
                        if spread > 0.0 {
                            ways.push((0.0, size0));
                        }
                        // (down to the smallest size an automatic name has)
                        for f in [0.8, 0.65, 0.5] {
                            if size0 * f >= min_px {
                                ways.push((0.0, size0 * f));
                            }
                        }
                        if size0 * 0.5 > min_px {
                            ways.push((0.0, min_px));
                        }
                        let (mut found, mut fallback) = (None, None);
                        'ways: for (spacing, size) in ways {
                            // smaller than four fifths only to clear other names
                            // and symbols, not a city dot (as in the application)
                            if fallback.is_some() && size < size0 * 0.79 {
                                break;
                            }
                            // each glyph's width with the space after it (none after the last)
                            let widths: Vec<f32> = glyphs.iter().enumerate().map(|(i, g)| text_width(g, size) + if i + 1 < glyphs.len() { spacing } else { 0.0 }).collect();
                            let total: f32 = widths.iter().sum();
                            let slack = ((len * 0.9 - total) * 0.5).max(0.0);
                            // how far off its line the name may sit
                            let off = (room * 0.5 - size * 0.6).max(0.0) * 0.7;
                            for lift in [0.0, off, -off] {
                                if lift != 0.0 && off < size * 0.3 {
                                    break;
                                }
                                for t in [0.0, 0.5, -0.5, 1.0, -1.0] {
                                    if t != 0.0 && slack < 1.0 {
                                        break;
                                    }
                                    let Some(places) = map_core::path_text::along_path_shifted(&l, &widths, lift, t * slack) else { continue 'ways };
                                    // (along_path centres each glyph's width with the space
                                    // after it: back by half a space)
                                    let places: Vec<([f32; 2], f32)> = places
                                        .into_iter()
                                        .enumerate()
                                        .map(|(i, (c, ang))| {
                                            let back = if i + 1 < glyphs.len() { spacing * 0.5 } else { 0.0 };
                                            let (sa, ca) = ang.to_radians().sin_cos();
                                            ([c[0] - back * ca, c[1] + back * sa], ang)
                                        })
                                        .collect();
                                    let gb: Vec<[f32; 4]> = glyphs.iter().zip(&places).map(|(g, (c, ang))| glyph_box(c[0], c[1], text_width(g, size), size, *ang)).collect();
                                    if !gb.iter().all(|b| inside(b) && boxes.iter().all(|q| !meets(b, q))) {
                                        continue;
                                    }
                                    // (a dot in the gap between two letters is on the name too)
                                    let spans = gb.windows(2).map(|w| [w[0][0].min(w[1][0]), w[0][1].min(w[1][1]), w[0][2].max(w[1][2]), w[0][3].max(w[1][3])]).chain(gb.first().copied());
                                    if spans.into_iter().all(|b| dots.iter().all(|q| !meets(&b, q))) {
                                        found = Some((spacing, size, places, gb));
                                        break 'ways;
                                    }
                                    if fallback.is_none() {
                                        fallback = Some((spacing, size, places, gb));
                                    }
                                }
                            }
                        }
                        let Some((spacing, size, places, gb)) = found.or(fallback) else { continue };
                        boxes.extend(gb);
                        let label = |text: String, c: [f32; 2], ang: f32| {
                            let (sa, ca) = ang.to_radians().sin_cos();
                            SceneLabel { text, pos: [(c[0] + 0.35 * size * sa) as f64, (c[1] + 0.35 * size * ca) as f64], size, color: st.label, angle: ang, italic: false, font:font(), bold:face.bold }
                        };
                        let (a0, a1) = places.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p.1), hi.max(p.1)));
                        if spacing < size * 0.05 && a1 - a0 < 1.0 {
                            // straight and close set: one piece of text
                            let (first, last) = (places[0].0, places[places.len() - 1].0);
                            let (wf, wl) = (text_width(&glyphs[0], size), text_width(&glyphs[glyphs.len() - 1], size));
                            let ang = (a0 + a1) * 0.5;
                            let (sa, ca) = ang.to_radians().sin_cos();
                            // the middle between the outer edges of the first and last glyphs
                            let d = (wl - wf) * 0.25;
                            let c = [(first[0] + last[0]) * 0.5 + d * ca, (first[1] + last[1]) * 0.5 - d * sa];
                            s.labels.push(label(text.to_owned(), c, ang));
                        } else {
                            for (g, (c, ang)) in glyphs.into_iter().zip(places) {
                                if !g.trim().is_empty() {
                                    s.labels.push(label(g, c, ang));
                                }
                            }
                        }
                    }
                }
            }
        }
        // names along their rivers and routes, on the longest piece of the
        // line, clear of the names already placed: in the middle, else slid
        // along the line (left out when nowhere on it is clear)
        for (id, r) in doc.rivers.iter().filter(|(id, r)| r.show_name && !r.name.is_empty() && !r.hidden && !doc.group_hides(**id)) {
            fades.push((s.labels.len(), doc.group_opacity(*id)));
            let size = map_core::path_text::NAME_PX * k;
            let path = split_polyline(&m.rel(&geom::unwrap_path(&r.pts, None)))
                .into_iter()
                .map(|piece| densify(&piece, false, 1.0).iter().map(|p| {
                    let q = m.px(*p);
                    [q[0] as f32, q[1] as f32]
                }).collect::<Vec<[f32; 2]>>())
                .max_by(|a, b| {
                    let len = |p: &Vec<[f32; 2]>| p.windows(2).map(|w| ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt()).sum::<f32>();
                    len(a).total_cmp(&len(b))
                });
            let Some(path) = path else { continue };
            let glyphs: Vec<String> = r.name.chars().map(String::from).collect();
            let widths: Vec<f32> = glyphs.iter().map(|g| text_width(g, size)).collect();
            let width = r.route.map_or(st.river_width, |x| x.width) * k;
            let len: f32 = path.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum();
            let slack = ((len * 0.9 - widths.iter().sum::<f32>()) * 0.5).max(0.0);
            let mut placed = None;
            for t in [0.0, 0.5, -0.5, 1.0, -1.0, 0.25, -0.25, 0.75, -0.75] {
                let Some(places) = map_core::path_text::along_path_shifted(&path, &widths, size * 0.6 + width * 0.5, t * slack) else { break };
                let gb: Vec<[f32; 4]> = places.iter().map(|([x, y], _)| [x - size * 0.5, y - size * 0.5, x + size * 0.5, y + size * 0.5]).collect();
                if gb.iter().all(|b| boxes.iter().all(|q| b[2] < q[0] || q[2] < b[0] || b[3] < q[1] || q[3] < b[1])) {
                    placed = Some((places, gb));
                    break;
                }
                if slack < 1.0 {
                    break;
                }
            }
            let Some((places, gb)) = placed else { continue };
            boxes.extend(gb);
            let color = r.name_color(st.river);
            for (g, ([x, y], turn)) in glyphs.into_iter().zip(places) {
                s.labels.push(SceneLabel { text: g, pos: [x as f64, (y + size * 0.35) as f64], size, color, angle: turn, italic: r.route.is_none(), font: None, bold: false });
            }
        }
    }
    fades.push((s.labels.len(), 1.0));
    for w in fades.windows(2) {
        for l in &mut s.labels[w[0].0..w[1].0] {
            l.color = faded(l.color, w[0].1);
        }
    }
    if o.labels && st.show_labels {
        let fits = |boxes: &mut Vec<[f32; 4]>, x: f32, y: f32, tw: f32, size: f32| {
            let bx = [x - tw * 0.5 - 2.0, y - size, x + tw * 0.5 + 2.0, y + size * 0.3];
            if bx[0] < 0.0 || bx[2] > w as f32 || bx[1] < 0.0 || bx[3] > h as f32 {
                return false;
            }
            if boxes.iter().any(|q| !(bx[2] < q[0] || q[2] < bx[0] || bx[3] < q[1] || q[3] < bx[1])) {
                return false;
            }
            boxes.push(bx);
            true
        };
        // city names dark, or a dark theme's light ones
        let city_ink = if map_core::doc::halo_for(st.label) == [255, 255, 255, 255] { [40, 40, 40, 255] } else { st.label };
        if st.show_cities {
            let capitals: HashSet<_> = doc.entities.values().filter(|e| e.parent.is_none() && e.layer == doc.active_layer && e.exists_in(doc.year))
                .filter_map(|e| e.capital).collect();
            for (id, e) in doc.entities.iter() {
                let (EntityKind::City, Some(pos)) = (e.kind, e.pos) else { continue };
                if !e.exists_in(doc.year) {
                    continue;
                }
                let p = m.px([geom::wrap_lon(pos.lon - o.lon0), pos.lat]);
                if p[0] < 0.0 || p[1] < 0.0 || p[0] > w || p[1] > h {
                    continue;
                }
                let r = 2.5 * k;
                s.dots.push((p, r, [40, 40, 40, 255]));
                let tier = if capitals.contains(id) { map_core::label_type::Tier::Capital } else { map_core::label_type::Tier::City };
                let face = st.label_type(tier);
                let size = if st.admin_typography { face.size } else { 11.0 } * k.min(2.0);
                let tx = p[0] as f32 + r + 3.0 + text_width(&e.name, size) * 0.5;
                if fits(&mut boxes, tx, p[1] as f32 + size * 0.35, text_width(&e.name, size), size) {
                    s.labels.push(SceneLabel { text: e.name.clone(), pos: [tx as f64, p[1] + size as f64 * 0.35], size, color: city_ink, angle: 0.0, italic: false, font:face.font.map(str::to_owned), bold:face.bold });
                }
            }
        }
    }
    s.shade = surface_shading(doc, &m);
    furniture(&mut s, doc, &m, o, k);
    if o.legend {
        let mut items: Vec<(String, LegendKey)> = theme.legend.iter().map(|i| (i.label.clone(), LegendKey::Color(i.color))).collect();
        let push = |items: &mut Vec<(String, LegendKey)>, label: &str, key: LegendKey| {
            if !items.iter().any(|x| x.0 == label && x.1 == key) {
                items.push((label.to_string(), key));
            }
        };
        // symbol kinds shown in this year, in the kinds' order
        for kind in map_core::symbol::SymbolKind::ALL {
            for (_, sym) in doc.symbols.iter().filter(|(id, x)| x.kind == kind && x.shown_in(doc.year) && !doc.group_hides(**id)) {
                let Some(label) = sym.legend_text() else { continue };
                let key = if let Some(art) = &sym.pixels {
                    LegendKey::Pixels(art.clone())
                } else if kind == map_core::symbol::SymbolKind::Glyph {
                    LegendKey::Glyph(sym.glyph.clone(), sym.color)
                } else {
                    LegendKey::Symbol(kind, sym.color)
                };
                push(&mut items, &label, key);
            }
        }
        // routes and areas the author described for the legend
        for (_, r) in doc.rivers.iter().filter(|(id, r)| !r.hidden && !doc.group_hides(**id)) {
            if let Some(st) = r.route.filter(|_| !r.legend.trim().is_empty()) {
                push(&mut items, r.legend.trim(), LegendKey::Route(st));
            }
        }
        for (_, a) in doc.areas.iter().filter(|(id, a)| a.shown_in(doc.year) && !a.legend.trim().is_empty() && !doc.group_hides(**id)) {
            push(&mut items, a.legend.trim(), LegendKey::Area { color: a.color, spacing: a.spacing, angle: a.angle, tint: a.tint, dashed: a.dashed, pattern: a.pattern });
        }
        if !items.is_empty() {
            let title = if theme.legend.is_empty() || theme.title.is_empty() { "图例".to_string() } else { theme.title.clone() };
            s.legend = Some((title, items));
        }
    }
    // furniture moved in the layout view, and the boxes as drawn
    let (fw, fh) = (o.width as f32, o.height as f32);
    let pl = &o.placement;
    s.title_at = pl.title.map(|p| [p[0] * fw, p[1] * fh]);
    s.legend_at = pl.legend.map(|p| [p[0] * fw, p[1] * fh]);
    if let Some((b, _, _)) = title_box(&s) {
        s.furniture.push((Furniture::Title, b));
    }
    if let Some((b, _, _)) = legend_layout(&s) {
        s.furniture.push((Furniture::Legend, b));
    }
    // insets: the same map, another region, in a frame of their own size
    for (i, inset) in pl.insets.iter().enumerate() {
        let rect = [inset.rect[0] * fw, inset.rect[1] * fh, inset.rect[2] * fw, inset.rect[3] * fh];
        let (iw, ih) = (rect[2].round().max(16.0) as u32, rect[3].round().max(16.0) as u32);
        let sub = ExportOptions {
            width: iw,
            height: ih,
            bounds: Some(inset.bounds),
            legend: false,
            title: String::new(),
            scale_bar: false,
            north_arrow: false,
            placement: Default::default(),
            ..o.clone()
        };
        let scene = build_scene(doc, theme, &sub);
        s.insets.push(SceneInset { rect, scene: Box::new(scene), title: inset.title.clone() });
        s.furniture.push((Furniture::Inset(i), rect));
    }
    s
}

/// Draw the same optional surface rasters as the map, below linework.
fn surface_shading(doc: &Document, m: &Mapper) -> Option<Vec<u8>> {
    let k = doc.style.hillshade.clamp(0.0, 1.0);
    let mut base = doc.terrain.as_ref().filter(|_| k > 0.).map(|t| {
        project_raster(&t.shade(doc.radius(), 1.0), t.w as usize, t.h as usize, k, m)
    });
    if let Some(p) = doc.climate.as_deref().filter(|p| p.visible && p.opacity > 0.0) {
        if let Some(scenario) = p.shown() {
            let (w, h, rgba) = scenario.raster(p.field, p.period);
            composite_raster(&mut base, project_raster(&rgba, w, h, p.opacity, m));
        }
    }
    if let Some(p) = doc.environment.as_deref().filter(|p| p.visible && p.opacity > 0.0) {
        if let Some(result) = p.shown() {
            let w = usize::from(result.grid.width);
            let h = usize::from(result.grid.height);
            let georef = result.grid.georef();
            let image = result.image(&p.layer);
            let top = project_georeferenced(
                &image, w, h, p.opacity, m, result.grid.whole_longitude(),
                |lon, lat| {
                    let p = map_core::LonLat::new(lon, lat);
                    result.grid.index(p)?;
                    let [x, y] = georef.pixel_of(p);
                    Some([x - 0.5, y - 0.5])
                },
            );
            composite_raster(&mut base, top);
        }
    }
    base
}

fn composite_raster(base: &mut Option<Vec<u8>>, top: Vec<u8>) {
    let Some(bottom) = base.as_mut() else {
        *base = Some(top);
        return;
    };
    for (a, b) in bottom.as_chunks_mut::<4>().0.iter_mut().zip(top.as_chunks::<4>().0) {
        if b[3] == 0 {
            continue;
        }
        let alpha = f32::from(b[3]) / 255.0;
        let remain = f32::from(a[3]) / 255.0 * (1.0 - alpha);
        for i in 0..3 {
            a[i] = ((f32::from(b[i]) * alpha + f32::from(a[i]) * remain) / (alpha + remain)).round() as u8;
        }
        a[3] = ((alpha + remain) * 255.0).round() as u8;
    }
}

fn project_raster(src: &[u8], tw: usize, th: usize, k: f32, m: &Mapper) -> Vec<u8> {
    project_georeferenced(src, tw, th, k, m, true, |lon, lat| Some([
        (lon + 180.0) / 360.0 * tw as f64 - 0.5,
        (90.0 - lat) / 180.0 * th as f64 - 0.5,
    ]))
}

fn project_georeferenced(
    src: &[u8], tw: usize, th: usize, k: f32, m: &Mapper, wraps: bool,
    pixel_at: impl Fn(f64, f64) -> Option<[f64; 2]>,
) -> Vec<u8> {
    let (w, h) = (m.w as usize, m.h as usize);
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let p = [(x as f64 + 0.5 - m.w * 0.5) / m.scale + m.cx, (m.h * 0.5 - y as f64 - 0.5) / m.scale + m.cy];
            let Some(ll) = m.proj.inverse_deg(p[0], p[1]) else { continue };
            let lon = geom::wrap_lon(ll[0] + m.lon0);
            let Some([gx, gy]) = pixel_at(lon, ll[1]) else { continue };
            let (i0, j0) = (gx.floor() as i64, gy.floor() as i64);
            let (fx, fy) = ((gx - gx.floor()) as f32, (gy - gy.floor()) as f32);
            let px = |i: i64, j: i64, c: usize| -> f32 {
                let i = if wraps { i.rem_euclid(tw as i64) } else { i.clamp(0, tw as i64 - 1) } as usize;
                let j = j.clamp(0, th as i64 - 1) as usize;
                src[(j * tw + i) * 4 + c] as f32
            };
            let o = (y * w + x) * 4;
            for c in 0..4 {
                let a = px(i0, j0, c) * (1.0 - fx) + px(i0 + 1, j0, c) * fx;
                let b = px(i0, j0 + 1, c) * (1.0 - fx) + px(i0 + 1, j0 + 1, c) * fx;
                let v = a * (1.0 - fy) + b * fy;
                out[o + c] = if c == 3 { (v * k) as u8 } else { v as u8 };
            }
        }
    }
    out
}

/// The longest piece (cut at the seam) of a lon/lat path, on the image.
fn longest_px_piece(m: &Mapper, path: &[map_core::geom::LonLat]) -> Option<Vec<[f32; 2]>> {
    split_polyline(&m.rel(&geom::unwrap_path(path, None)))
        .into_iter()
        .map(|piece| densify(&piece, false, 1.0).iter().map(|p| {
            let q = m.px(*p);
            [q[0] as f32, q[1] as f32]
        }).collect::<Vec<[f32; 2]>>())
        .max_by(|a, b| {
            let len = |p: &Vec<[f32; 2]>| p.windows(2).map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1])).sum::<f32>();
            len(a).total_cmp(&len(b))
        })
}

/// A dot as a small polygon (fills are polygons in every exporter).
fn dot_poly(c: [f32; 2], r: f32) -> Vec<P2> {
    (0..10)
        .map(|i| {
            let t = i as f32 / 10.0 * std::f32::consts::TAU;
            [(c[0] + r * t.cos()) as f64, (c[1] + r * t.sin()) as f64]
        })
        .collect()
}

/// The largest 1, 2 or 5 × 10ⁿ not above `t`.
fn nice_length(t: f64) -> f64 {
    let p = 10f64.powf(t.log10().floor());
    [5.0, 2.0, 1.0].into_iter().map(|m| m * p).find(|v| *v <= t).unwrap_or(p)
}

/// A length for the scale bar: whole numbers without decimals.
fn km_text(v: f64) -> String {
    if v >= 1.0 {
        format!("{v:.0}")
    } else {
        format!("{v}")
    }
}

/// Map furniture in the lower right: a scale bar measured at the image
/// centre (with a note when the scale changes much across the image, as
/// on a world map), and a north arrow pointing the way north runs at the
/// centre.
fn furniture(s: &mut Scene, doc: &Document, m: &Mapper, o: &ExportOptions, k: f32) {
    // drawn last, over any inset
    let mut over = empty_scene(s.width, s.height, [0, 0, 0, 0]);
    // the place shown at an image point
    let at = |x: f64, y: f64| -> Option<map_core::geom::LonLat> {
        let p = [(x - m.w * 0.5) / m.scale + m.cx, (m.h * 0.5 - y) / m.scale + m.cy];
        m.proj.inverse_deg(p[0], p[1]).map(|ll| map_core::geom::LonLat::new(geom::wrap_lon(ll[0] + m.lon0), ll[1]))
    };
    // km per pixel along the horizontal through image row y
    let km_px = |y: f64| -> Option<f64> {
        let d = 40.0;
        let (a, b) = (at(m.w * 0.5 - d, y)?, at(m.w * 0.5 + d, y)?);
        Some(map_core::measure::great_circle(a, b, doc.radius()) / (2.0 * d)).filter(|v| *v > 0.0)
    };
    let (w, h) = (m.w as f32, m.h as f32);
    let ink = [20, 20, 20, 255];
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| vec![[x0 as f64, y0 as f64], [x1 as f64, y0 as f64], [x1 as f64, y1 as f64], [x0 as f64, y1 as f64]];
    let label = |text: String, x: f32, base: f32, size: f32| SceneLabel { text, pos: [x as f64, base as f64], size, color: ink, angle: 0.0, italic: false, font: None, bold: false };
    let bar_top = h - 34.0 * k;
    if o.scale_bar {
        if let Some(kpp) = km_px(m.h * 0.5) {
            let nice = nice_length(160.0 * k as f64 * kpp);
            let len = (nice / kpp) as f32;
            let bh = 6.0 * k;
            // in its usual place (lower right), or where the layout view put its box
            let (x0, x1, bar_top) = match o.placement.scale_bar {
                Some(p) => {
                    let x0 = p[0] * w + 8.0 * k;
                    (x0, x0 + len, p[1] * h + 18.0 * k)
                }
                None => (w - 28.0 * k - len, w - 28.0 * k, bar_top),
            };
            let tail = text_width(&format!("{} km", km_text(nice)), 11.0 * k) * 0.5 + 4.0 * k;
            s.furniture.push((Furniture::ScaleBar, [x0 - 8.0 * k, bar_top - 18.0 * k, x1 + tail - (x0 - 8.0 * k), 18.0 * k + bh + 20.0 * k]));
            // black bar, white in the second and fourth quarters
            over.fills.push((rect(x0, bar_top, x1, bar_top + bh), ink));
            for q in [1.0, 3.0] {
                over.fills.push((rect(x0 + len * q / 4.0, bar_top + k, x0 + len * (q + 1.0) / 4.0 - k, bar_top + bh - k), [255, 255, 255, 255]));
            }
            let size = 11.0 * k;
            let base = bar_top - 4.0 * k;
            over.labels.push(label("0".into(), x0, base, size));
            over.labels.push(label(km_text(nice / 2.0), x0 + len * 0.5, base, size));
            over.labels.push(label(format!("{} km", km_text(nice)), x1, base, size));
            // a world map's scale changes from place to place: say so
            let (top, low) = (km_px(m.h * 0.2), km_px(m.h * 0.8));
            let spread = [top, low].into_iter().flatten().map(|v| (v / kpp).max(kpp / v)).fold(1.0, f64::max);
            if spread > 1.1 {
                over.labels.push(label("比例尺仅适用于图中心附近".into(), x0 + len * 0.5, bar_top + bh + 13.0 * k, 9.0 * k));
            }
        }
    }
    if o.north_arrow {
        if let Some(c) = at(m.w * 0.5, m.h * 0.5) {
            let north = map_core::geom::LonLat::new(c.lon, (c.lat + 0.5).min(89.9));
            let (a, b) = (m.px([geom::wrap_lon(c.lon - m.lon0), c.lat]), m.px([geom::wrap_lon(north.lon - m.lon0), north.lat]));
            let (dx, dy) = ((b[0] - a[0]) as f32, (b[1] - a[1]) as f32);
            let n = dx.hypot(dy).max(1e-6);
            let (ux, uy) = (dx / n, dy / n);
            let (px, py) = (-uy, ux);
            let c0 = match o.placement.north_arrow {
                Some(p) => [p[0] * w + 40.0 * k, p[1] * h + 40.0 * k],
                None => [w - 50.0 * k, if o.scale_bar { bar_top - 52.0 * k } else { h - 60.0 * k }],
            };
            s.furniture.push((Furniture::NorthArrow, [c0[0] - 40.0 * k, c0[1] - 40.0 * k, 80.0 * k, 80.0 * k]));
            let p = |along: f32, across: f32| [(c0[0] + ux * along + px * across) as f64, (c0[1] + uy * along + py * across) as f64];
            // a split arrowhead: dark on one side, light on the other
            over.fills.push((vec![p(18.0 * k, 0.0), p(-12.0 * k, 8.0 * k), p(-6.0 * k, 0.0)], ink));
            over.fills.push((vec![p(18.0 * k, 0.0), p(-6.0 * k, 0.0), p(-12.0 * k, -8.0 * k)], [150, 150, 150, 255]));
            let t = p(30.0 * k, 0.0);
            over.labels.push(label("北".into(), t[0] as f32, t[1] as f32 + 5.0 * k, 13.0 * k));
        }
    }
    if !over.fills.is_empty() || !over.labels.is_empty() {
        s.top = Some(Box::new(over));
    }
}

/// Legend layout shared by exporters: the box, the text size, and each
/// row's swatch (already drawn as parts) and label.
pub fn legend_layout(s: &Scene) -> Option<([f32; 4], f32, Vec<LegendRow>)> {
    let (title, items) = s.legend.as_ref()?;
    let k = (s.width as f32 / 1600.0).clamp(1.0, 3.0);
    let size = 13.0 * k;
    let row = size * 1.6;
    let items: Vec<&(String, LegendKey)> = items.iter().take(30).collect();
    let tw = items.iter().map(|i| text_width(&i.0, size)).fold(text_width(title, size * 1.1), f32::max);
    let bw = tw + size * 4.5;
    let bh = row * (items.len() as f32 + 1.0) + size;
    let (x0, y0) = match s.legend_at {
        Some([x, y]) => (x, y),
        None => (size, s.height as f32 - bh - size),
    };
    let rows = items
        .iter()
        .enumerate()
        .map(|(i, (label, key))| {
            let y = y0 + size * 0.5 + row * (i as f32 + 1.0);
            // areas and colours take a box of the usual width; a route's
            // sample runs the whole column so its dashes show before the arrow
            let box_ = [x0 + size * 0.6, y + row * 0.15, size * 1.6, row * 0.7];
            let column = [box_[0], box_[1], size * 2.6, box_[3]];
            LegendRow { parts: swatch(key, box_, column, k), text: [x0 + size * 3.6, y + row * 0.72], label: label.clone() }
        })
        .collect();
    Some(([x0, y0, bw, bh], size, rows))
}

/// A legend swatch drawn like the thing it stands for, at export scale
/// `k`: in `r` ([x, y, w, h]), or along the wider `column` for a route.
fn swatch(key: &LegendKey, r: [f32; 4], column: [f32; 4], k: f32) -> Vec<LegendPart> {
    let [x, y, w, h] = r;
    let px = |v: &[[f32; 2]]| v.iter().map(|q| [q[0] as f64, q[1] as f64]).collect::<Vec<P2>>();
    match key {
        LegendKey::Color(c) => vec![LegendPart::Rect(r, *c)],
        LegendKey::Route(st) if st.broad => {
            // drawn at its own width along a longer line, then shrunk as a
            // whole so the head fits the row and the proportions stay
            let width = st.width.clamp(6.0, 40.0);
            let f = (h * 0.95 / (2.0 * (width + 4.0))).min(1.0);
            let (x0, yc) = (x, y + h * 0.5);
            let line = [[x0, yc], [x0 + column[2] / f, yc]];
            match map_core::broad::broad_arrow(&line, width, st.arrow) {
                Some(outline) => {
                    let shrunk: Vec<[f32; 2]> = outline.iter().map(|p| [x0 + (p[0] - x0) * f, yc + (p[1] - yc) * f]).collect();
                    let poly = px(&shrunk);
                    let mut ring = poly.clone();
                    ring.push(poly[0]);
                    let (edge, dash) = st.broad_edge(k);
                    let mut parts = vec![LegendPart::Line(SceneLine { color: st.color, width: edge, pts: ring, dash })];
                    if !st.hollow {
                        parts.insert(0, LegendPart::Fill(poly, st.broad_fill()));
                    }
                    parts
                }
                None => vec![LegendPart::Rect(r, st.broad_fill())],
            }
        }
        LegendKey::Route(st) => {
            let width = (st.width * k).min(h * 0.5);
            let line = [[x + width * 0.5, y + h * 0.5], [x + column[2] - width * 0.5, y + h * 0.5]];
            let mut parts = Vec::new();
            let mut pts = line.to_vec();
            if st.arrow {
                if let Some((l, head)) = map_core::doc::arrow_end(&line, width) {
                    pts = l;
                    parts.push(LegendPart::Fill(px(&head), st.color));
                }
            }
            parts.insert(0, LegendPart::Line(SceneLine { color: st.color, width, pts: px(&pts), dash: st.pattern(width) }));
            for mk in map_core::deco::decorate(&pts, st.deco, width, st.deco_flip) {
                parts.push(match mk {
                    map_core::deco::DecoMark::Fill(t) => LegendPart::Fill(px(&t), st.color),
                    map_core::deco::DecoMark::Stroke(seg) => LegendPart::Line(SceneLine { color: st.color, width, pts: px(&seg), dash: None }),
                });
            }
            parts
        }
        LegendKey::Area { color, spacing, angle, tint, dashed, pattern } => {
            let mut parts = Vec::new();
            if *tint {
                parts.push(LegendPart::Rect(r, [color[0], color[1], color[2], 45]));
            }
            let ring = vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
            let gap = (spacing * k).clamp(3.0, h * 0.45);
            let (stripes, dots) = map_core::area::fill_pattern(&[ring.clone()], *pattern, gap, *angle, None);
            for [p, q] in stripes {
                parts.push(LegendPart::Line(SceneLine { color: *color, width: map_core::area::HATCH_WIDTH * k, pts: px(&[p, q]), dash: None }));
            }
            for p in dots {
                parts.push(LegendPart::Fill(dot_poly(p, map_core::area::DOT_RADIUS * k), *color));
            }
            let mut outline = ring;
            outline.push([x, y]);
            let ow = map_core::area::OUTLINE_WIDTH * k;
            parts.push(LegendPart::Line(SceneLine { color: *color, width: ow, pts: px(&outline), dash: dashed.then_some([4.0 * k, 2.5 * k + ow]) }));
            parts
        }
        LegendKey::Glyph(g, color) => {
            let size = (h * 1.1).min(map_core::symbol::glyph_px(h * 0.48));
            let tw = text_width(g, size);
            vec![LegendPart::Text(g.clone(), [x + w * 0.5 - tw * 0.5, y + h * 0.5 + size * 0.35], size, *color)]
        }
        LegendKey::Pixels(art) => {
            let [pw, ph] = art.drawn_size(w.min(h) * 0.94);
            pixel_rects(art, [x + (w - pw) * 0.5, y + (h - ph) * 0.5, pw, ph])
                .map(|(rect, color)| LegendPart::Rect(rect, color)).collect()
        }
        LegendKey::Symbol(kind, color) => map_core::symbol::marks(*kind, [x + w * 0.5, y + h * 0.5], h * 0.48, *color)
            .into_iter()
            .map(|m| match m {
                map_core::symbol::Mark::Fill { outline, color, .. } => LegendPart::Fill(px(&outline), color),
                map_core::symbol::Mark::Stroke { pts, closed, width, color } => {
                    let mut v = px(&pts);
                    if closed {
                        v.push(v[0]);
                    }
                    LegendPart::Line(SceneLine { color, width, pts: v, dash: None })
                }
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Free text is exported as placed: turned, slanted, spaced, and
    /// sized with the map; labels drawn after it keep clear of it.
    #[test]
    fn annotations_reach_the_export() {
        use map_core::doc::{Annotation, LineId};
        let mut d = map_core::sample::demo_world();
        let mut a = Annotation::new("大洋", map_core::geom::LonLat::new(-30.0, -40.0));
        a.angle = 20.0;
        a.spacing = 1.0;
        let id = LineId(d.topo.alloc());
        d.annotations.insert(id, a.clone());
        assert_eq!(a.spaced(), "大  洋");
        let theme = map_core::theme::evaluate(&d);
        let wide = ExportOptions { width: 3200, height: 1800, ..Default::default() };
        let narrow = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let (s1, s2) = (build_scene(&d, &theme, &narrow), build_scene(&d, &theme, &wide));
        let find = |s: &Scene| s.labels.iter().find(|l| l.text == "大  洋").map(|l| (l.size, l.angle, l.italic)).expect("annotation exported");
        let ((z1, ang, it), (z2, _, _)) = (find(&s1), find(&s2));
        assert_eq!((ang, it), (20.0, true));
        assert!((z2 / z1 - 2.0).abs() < 0.05, "twice the width, twice the text: {z1} {z2}");
        let svg = crate::svg::render(&s1);
        assert!(svg.contains("font-style=\"italic\"") && svg.contains("rotate(-20.00"));
        let font = crate::png::find_system_font();
        crate::png::render(&s1, font.as_deref()).unwrap();
        // without the labels option nothing textual is exported
        let none = build_scene(&d, &theme, &ExportOptions { labels: false, ..narrow });
        assert!(none.labels.is_empty());
    }

    /// A route is exported in its own colour, width and dashes, with an
    /// arrowhead at its end; rivers stay solid; the GPU map leaves routes
    /// to the overlay.
    #[test]
    fn routes_reach_the_export() {
        use map_core::doc::{Dash, LineFeature, LineId, RouteStyle};
        use map_core::geom::LonLat;
        let mut d = map_core::sample::demo_world();
        let style = RouteStyle { color: [9, 8, 7, 255], width: 3.0, dash: Dash::Dashed, arrow: true, ..Default::default() };
        let pts = vec![LonLat::new(-60.0, -50.0), LonLat::new(-20.0, -45.0), LonLat::new(10.0, -30.0)];
        d.rivers.insert(LineId(d.topo.alloc()), LineFeature { name: "进军路线".into(), pts, route: Some(style), legend: String::new(), show_name: false, hidden: false });
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &ExportOptions { width: 3200, height: 1800, ..Default::default() });
        let k = 2.0;
        let route: Vec<&SceneLine> = s.lines.iter().filter(|l| l.color == [9, 8, 7, 255]).collect();
        assert_eq!(route.len(), 1);
        assert_eq!((route[0].width, route[0].dash), (3.0 * k, style.pattern(3.0 * k)));
        assert!(s.lines.iter().filter(|l| l.color != [9, 8, 7, 255]).all(|l| l.dash.is_none()), "rivers and borders stay solid");
        assert_eq!(s.fills.len(), 1);
        let (head, c) = (&s.fills[0].0, s.fills[0].1);
        assert_eq!(head.len(), 3);
        assert_eq!(c, [9, 8, 7, 255]);
        // the tip is the route's last point, and the line stops inside the head
        let end = route[0].pts.last().unwrap();
        let d2 = |a: P2, b: P2| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        assert!(d2(*end, head[0]) < d2(head[1], head[0]), "line ends inside the arrowhead");
        let svg = crate::svg::render(&s);
        assert!(svg.contains("stroke-dasharray") && svg.contains("fill=\"rgb(9,8,7)\""));
        crate::png::render(&s, None).unwrap();
        // without the arrow, no head
        let mut d3 = d.clone();
        let ids: Vec<LineId> = d3.rivers.keys().copied().collect();
        for id in ids {
            if let Some(st) = d3.rivers.get_mut(&id).and_then(|r| r.route.as_mut()) {
                st.arrow = false;
            }
        }
        assert!(build_scene(&d3, &theme, &ExportOptions::default()).fills.is_empty());
    }

    /// A hatched area is exported as stripes clipped to its shape, a
    /// dashed outline, an optional wash and its name, only in its years.
    #[test]
    fn hatched_areas_reach_the_export() {
        use map_core::area::AreaMark;
        use map_core::doc::LineId;
        use map_core::geom::LonLat;
        let mut d = map_core::sample::demo_world();
        let mut a = AreaMark::new(vec![LonLat::new(-60.0, -50.0), LonLat::new(-20.0, -50.0), LonLat::new(-20.0, -30.0), LonLat::new(-60.0, -30.0)]);
        a.color = [3, 4, 5, 255];
        a.name = "争议区".into();
        a.tint = true;
        a.to = Some(1900);
        d.areas.insert(LineId(d.topo.alloc()), a.clone());
        let theme = map_core::theme::evaluate(&d);
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let s = build_scene(&d, &theme, &opts);
        let mine: Vec<&SceneLine> = s.lines.iter().filter(|l| l.color == [3, 4, 5, 255]).collect();
        let stripes: Vec<&&SceneLine> = mine.iter().filter(|l| l.dash.is_none()).collect();
        let outline: Vec<&&SceneLine> = mine.iter().filter(|l| l.dash.is_some()).collect();
        assert_eq!(outline.len(), 1);
        assert!(stripes.len() > 10, "{}", stripes.len());
        // every stripe lies inside the outline's bounding box, at 45 degrees
        let xs: Vec<f64> = outline[0].pts.iter().map(|p| p[0]).collect();
        let ys: Vec<f64> = outline[0].pts.iter().map(|p| p[1]).collect();
        let (x0, x1) = (xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max));
        let (y0, y1) = (ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max));
        for l in &stripes {
            for p in &l.pts {
                assert!(p[0] >= x0 - 0.5 && p[0] <= x1 + 0.5 && p[1] >= y0 - 0.5 && p[1] <= y1 + 0.5, "{p:?}");
            }
            let (dx, dy) = (l.pts[1][0] - l.pts[0][0], l.pts[1][1] - l.pts[0][1]);
            assert!((dx.abs() - dy.abs()).abs() < 1e-3 * (dx.abs() + 1.0), "45 degrees: {dx} {dy}");
        }
        assert!(s.polys.iter().any(|p| p.fill == [3, 4, 5, 45]), "the wash");
        assert!(s.labels.iter().any(|l| l.text == "争议区" && l.italic));
        crate::png::render(&s, None).unwrap();
        // after its last year nothing of it is exported
        d.year = Some(1901);
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &opts);
        assert!(!s.lines.iter().any(|l| l.color == [3, 4, 5, 255]) && !s.labels.iter().any(|l| l.text == "争议区"));
    }

    /// Routes and areas with a legend text, and every symbol kind shown,
    /// get a legend row drawn like them; the same text and look share a
    /// row; with no thematic legend the box is titled 图例.
    #[test]
    fn drawn_features_join_the_legend() {
        use map_core::area::AreaMark;
        use map_core::doc::{LineFeature, LineId, RouteStyle};
        use map_core::geom::LonLat;
        use map_core::symbol::{MapSymbol, SymbolKind};
        let mut d = map_core::sample::demo_world();
        let route = |legend: &str| LineFeature { name: "r".into(), pts: vec![LonLat::new(0.0, -50.0), LonLat::new(20.0, -50.0)], route: Some(RouteStyle::default()), legend: legend.into(), show_name: false, hidden: false };
        d.rivers.insert(LineId(d.topo.alloc()), route("进军路线"));
        d.rivers.insert(LineId(d.topo.alloc()), route("进军路线"));
        d.rivers.insert(LineId(d.topo.alloc()), route(""));
        let mut a = AreaMark::new(vec![LonLat::new(0.0, -40.0), LonLat::new(10.0, -40.0), LonLat::new(10.0, -30.0)]);
        a.legend = "争议地区".into();
        a.to = Some(1900);
        d.areas.insert(LineId(d.topo.alloc()), a);
        d.symbols.insert(LineId(d.topo.alloc()), MapSymbol::new(SymbolKind::Battle, LonLat::new(5.0, -45.0)));
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, ..Default::default() });
        let (_, items) = s.legend.as_ref().unwrap();
        let tail: Vec<&str> = items.iter().filter(|i| !matches!(i.1, LegendKey::Color(_))).map(|i| i.0.as_str()).collect();
        assert_eq!(tail, vec!["战役", "进军路线", "争议地区"]);
        let (_, _, rows) = legend_layout(&s).unwrap();
        let row = |l: &str| rows.iter().find(|r| r.label == l).unwrap();
        // the route's swatch: a dashed line and an arrowhead
        assert!(row("进军路线").parts.iter().any(|p| matches!(p, LegendPart::Line(l) if l.dash.is_some())));
        assert!(row("进军路线").parts.iter().any(|p| matches!(p, LegendPart::Fill(v, _) if v.len() == 3)));
        // the area's: stripes inside the swatch and a dashed outline
        let area = row("争议地区");
        assert!(area.parts.iter().filter(|p| matches!(p, LegendPart::Line(l) if l.dash.is_none())).count() >= 2);
        assert!(area.parts.iter().any(|p| matches!(p, LegendPart::Line(l) if l.dash.is_some() && l.pts.len() == 5)));
        // the battle's: its four strokes over their halos
        assert_eq!(row("战役").parts.len(), 8);
        crate::png::render(&s, None).unwrap();
        let svg = crate::svg::render(&s);
        assert!(svg.contains("进军路线") && svg.contains("争议地区"));
        // outside the area's years its row goes; with no theme legend the title is 图例
        d.year = Some(1950);
        let mut theme = map_core::theme::evaluate(&d);
        theme.legend.clear();
        let s = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, ..Default::default() });
        let (title, items) = s.legend.as_ref().unwrap();
        assert_eq!(title, "图例");
        assert_eq!(items.iter().map(|i| i.0.as_str()).collect::<Vec<_>>(), vec!["战役", "进军路线"]);
    }

    /// A dotted area exports dots (no stripes), a cross-hatched one
    /// stripes both ways; the legend swatch follows the pattern.
    #[test]
    fn area_patterns_reach_the_export() {
        use map_core::area::{AreaMark, Pattern};
        use map_core::doc::LineId;
        use map_core::geom::LonLat;
        let mut d = map_core::sample::demo_world();
        let ring = vec![LonLat::new(-60.0, -50.0), LonLat::new(-20.0, -50.0), LonLat::new(-20.0, -30.0), LonLat::new(-60.0, -30.0)];
        let mut dotted = AreaMark::new(ring.clone());
        dotted.color = [7, 7, 7, 255];
        dotted.pattern = Pattern::Dots;
        dotted.legend = "势力范围".into();
        d.areas.insert(LineId(d.topo.alloc()), dotted);
        let theme = map_core::theme::evaluate(&d);
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let s = build_scene(&d, &theme, &opts);
        let dots = s.fills.iter().filter(|f| f.1 == [7, 7, 7, 255]).count();
        assert!(dots > 30, "{dots}");
        assert!(s.lines.iter().filter(|l| l.color == [7, 7, 7, 255]).all(|l| l.dash.is_some()), "only the dashed outline, no stripes");
        let (_, _, rows) = legend_layout(&s).unwrap();
        let row = rows.iter().find(|r| r.label == "势力范围").unwrap();
        assert!(row.parts.iter().any(|p| matches!(p, LegendPart::Fill(..))));
        assert!(!row.parts.iter().any(|p| matches!(p, LegendPart::Line(l) if l.dash.is_none())));
        // cross-hatching: stripes in two directions
        let mut d2 = map_core::sample::demo_world();
        let mut cross = AreaMark::new(ring);
        cross.color = [8, 8, 8, 255];
        cross.pattern = Pattern::Cross;
        d2.areas.insert(LineId(d2.topo.alloc()), cross);
        let theme = map_core::theme::evaluate(&d2);
        let s = build_scene(&d2, &theme, &opts);
        let slopes: Vec<bool> = s.lines.iter().filter(|l| l.color == [8, 8, 8, 255] && l.dash.is_none()).map(|l| (l.pts[1][0] - l.pts[0][0]) * (l.pts[1][1] - l.pts[0][1]) > 0.0).collect();
        assert!(slopes.iter().any(|x| *x) && slopes.iter().any(|x| !*x), "both directions");
        crate::png::render(&s, None).unwrap();
    }

    /// A line with its name switched on gets the name glyph by glyph along
    /// it, reading left to right, above it; without the switch, nothing.
    #[test]
    fn names_follow_their_lines() {
        use map_core::doc::{LineFeature, LineId};
        use map_core::geom::LonLat;
        let mut d = map_core::sample::demo_world();
        // drawn right to left, rising
        let pts = vec![LonLat::new(-20.0, -50.0), LonLat::new(-40.0, -45.0), LonLat::new(-60.0, -40.0)];
        d.rivers.insert(LineId(d.topo.alloc()), LineFeature { name: "大河".into(), pts: pts.clone(), route: None, legend: String::new(), show_name: true, hidden: false });
        d.rivers.insert(LineId(d.topo.alloc()), LineFeature { name: "无名".into(), pts, route: None, legend: String::new(), show_name: false, hidden: false });
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, ..Default::default() });
        // (a country's name may be written glyph by glyph too: not in italics)
        let g: Vec<&SceneLabel> = s.labels.iter().filter(|l| l.italic && (l.text == "大" || l.text == "河")).collect();
        assert_eq!(g.len(), 2);
        assert!(!s.labels.iter().any(|l| l.text == "无" || l.text == "名"));
        // left to right, turned with the line (it falls to the right), italic
        assert_eq!((g[0].text.as_str(), g[1].text.as_str()), ("大", "河"));
        assert!(g[0].pos[0] < g[1].pos[0] && g[0].pos[1] < g[1].pos[1]);
        assert!(g[0].angle < 0.0 && g[0].angle > -45.0 && g[0].italic);
    }

    /// A broad route is exported as one see-through filled band with a
    /// thin edge, ending in its head; its legend swatch is a small band.
    #[test]
    fn broad_arrows_reach_the_export() {
        use map_core::doc::{LineFeature, LineId, RouteStyle};
        use map_core::geom::LonLat;
        let mut d = map_core::sample::demo_world();
        let st = RouteStyle { color: [6, 5, 4, 255], width: 14.0, broad: true, ..Default::default() };
        let pts = vec![LonLat::new(-60.0, -50.0), LonLat::new(-40.0, -45.0), LonLat::new(-20.0, -35.0)];
        d.rivers.insert(LineId(d.topo.alloc()), LineFeature { name: "进军".into(), pts, route: Some(st), legend: "进攻方向".into(), show_name: false, hidden: false });
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, ..Default::default() });
        let fills: Vec<_> = s.fills.iter().filter(|f| f.1 == st.broad_fill()).collect();
        assert_eq!(fills.len(), 1);
        let edge: Vec<&SceneLine> = s.lines.iter().filter(|l| l.color == [6, 5, 4, 255]).collect();
        assert_eq!(edge.len(), 1, "only the edge, no dashed line");
        assert!(edge[0].dash.is_none() && edge[0].pts.first() == edge[0].pts.last());
        let (_, _, rows) = legend_layout(&s).unwrap();
        let row = rows.iter().find(|r| r.label == "进攻方向").unwrap();
        assert!(row.parts.iter().any(|p| matches!(p, LegendPart::Fill(_, c) if *c == st.broad_fill())));
        crate::png::render(&s, None).unwrap();
        assert!(crate::svg::render(&s).contains("rgb(6,5,4)"));
    }

    /// A country's outline made into a hatched area keeps every piece and
    /// hole: stripes inside the pieces, none in the holes; an area outside
    /// its years is not exported.
    #[test]
    fn country_outlines_export_as_areas() {
        let mut d = map_core::sample::demo_world();
        let e = d.countries().into_iter().max_by_key(|e| d.faces_of(*e).len()).unwrap();
        let id = d.area_from_entity(e, None).unwrap();
        let rings = d.areas[&id].rings().len();
        d.areas.get_mut(&id).unwrap().color = [1, 1, 1, 255];
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, ..Default::default() });
        let outline = s.lines.iter().filter(|l| l.color == [1, 1, 1, 255] && l.dash.is_some()).count();
        assert!(outline >= rings, "an outline for every ring: {outline} < {rings}");
        assert!(s.lines.iter().filter(|l| l.color == [1, 1, 1, 255] && l.dash.is_none()).count() > 10, "stripes");
        crate::png::render(&s, None).unwrap();
    }

    /// Text following a drawn curve is exported glyph by glyph along it,
    /// left to right, turned with the curve; without the curve it is a
    /// straight line again.
    #[test]
    fn text_follows_its_curve() {
        use map_core::doc::{Annotation, LineId};
        use map_core::geom::LonLat;
        let mut d = map_core::sample::demo_world();
        let mut a = Annotation::new("长岭山脉", LonLat::new(0.0, 0.0));
        a.spacing = 0.0;
        a.size = 14.0;
        a.color = [3, 3, 9, 255];
        // drawn right to left, falling towards the west
        a.set_path(vec![LonLat::new(-20.0, -40.0), LonLat::new(-35.0, -45.0), LonLat::new(-50.0, -52.0)]);
        assert!(a.on_path() && (a.at.lon - -35.0).abs() < 2.0);
        let id = LineId(d.topo.alloc());
        d.annotations.insert(id, a);
        let theme = map_core::theme::evaluate(&d);
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let s = build_scene(&d, &theme, &opts);
        let g: Vec<&SceneLabel> = s.labels.iter().filter(|l| l.color == [3, 3, 9, 255]).collect();
        assert_eq!(g.iter().map(|l| l.text.as_str()).collect::<String>(), "长岭山脉");
        assert!(g.windows(2).all(|w| w[0].pos[0] < w[1].pos[0]), "left to right");
        assert!(g.iter().all(|l| l.angle > 5.0), "rising to the east");
        // back to a straight line
        d.annotations.get_mut(&id).unwrap().set_path(Vec::new());
        let s = build_scene(&d, &theme, &opts);
        assert!(s.labels.iter().any(|l| l.color == [3, 3, 9, 255] && l.text == "长岭山脉" && l.angle == 0.0));
    }

    /// A symbol's name goes on the side asked for; a hollow broad arrow is
    /// its dashed outline only, no fill.
    #[test]
    fn name_sides_and_hollow_arrows() {
        use map_core::doc::{Dash, LineFeature, LineId, RouteStyle};
        use map_core::geom::LonLat;
        use map_core::symbol::{MapSymbol, NamePlace, SymbolKind};
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let mut found = Vec::new();
        for place in NamePlace::ALL {
            let mut d = map_core::sample::demo_world();
            let mut s = MapSymbol::new(SymbolKind::Camp, LonLat::new(-40.0, -45.0));
            s.name = "营".into();
            s.name_place = place;
            d.symbols.insert(LineId(d.topo.alloc()), s);
            let theme = map_core::theme::evaluate(&d);
            let sc = build_scene(&d, &theme, &opts);
            found.push(sc.labels.iter().find(|l| l.text == "营").map(|l| l.pos).unwrap());
        }
        let [right, left, above, below] = [found[0], found[1], found[2], found[3]];
        assert!(right[0] > above[0] && left[0] < above[0], "right of and left of the symbol");
        assert!(above[1] < right[1] && below[1] > right[1], "above and below it");
        assert!((above[0] - below[0]).abs() < 1e-6 && (right[1] - left[1]).abs() < 1e-6);

        let mut d = map_core::sample::demo_world();
        let st = RouteStyle { color: [4, 4, 4, 255], width: 14.0, broad: true, hollow: true, dash: Dash::Dashed, ..Default::default() };
        let pts = vec![LonLat::new(-60.0, -50.0), LonLat::new(-20.0, -40.0)];
        d.rivers.insert(LineId(d.topo.alloc()), LineFeature { name: "撤退".into(), pts, route: Some(st), legend: "撤退".into(), show_name: false, hidden: false });
        let theme = map_core::theme::evaluate(&d);
        let sc = build_scene(&d, &theme, &opts);
        assert!(!sc.fills.iter().any(|f| f.1 == st.broad_fill()), "no fill");
        let edge: Vec<&SceneLine> = sc.lines.iter().filter(|l| l.color == [4, 4, 4, 255]).collect();
        assert_eq!(edge.len(), 1);
        assert!(edge[0].dash.is_some() && edge[0].width == 2.0);
        let (_, _, rows) = legend_layout(&sc).unwrap();
        let row = rows.iter().find(|r| r.label == "撤退").unwrap();
        assert!(!row.parts.iter().any(|p| matches!(p, LegendPart::Fill(..))));
    }

    /// Hidden features of every kind are left out of the drawing, the
    /// labels and the legend; shown again they come back.
    #[test]
    fn hidden_features_are_not_exported() {
        use map_core::area::AreaMark;
        use map_core::doc::{Annotation, LineFeature, LineId, RouteStyle};
        use map_core::features::FeatureRef;
        use map_core::geom::LonLat;
        use map_core::symbol::{MapSymbol, SymbolKind};
        let mut d = map_core::sample::demo_world();
        let ids: Vec<LineId> = (0..5).map(|_| LineId(d.topo.alloc())).collect();
        let mut t = Annotation::new("隐文", LonLat::new(-30.0, -40.0));
        t.color = [2, 0, 0, 255];
        d.annotations.insert(ids[0], t);
        let mut s = MapSymbol::new(SymbolKind::Battle, LonLat::new(-20.0, -45.0));
        s.color = [3, 0, 0, 255];
        d.symbols.insert(ids[1], s);
        let st = RouteStyle { color: [4, 0, 0, 255], ..Default::default() };
        d.rivers.insert(ids[2], LineFeature { name: "路".into(), pts: vec![LonLat::new(-60.0, -50.0), LonLat::new(-30.0, -50.0)], route: Some(st), legend: "隐路".into(), show_name: false, hidden: false });
        let mut a = AreaMark::new(vec![LonLat::new(-60.0, -35.0), LonLat::new(-40.0, -35.0), LonLat::new(-40.0, -25.0)]);
        a.color = [5, 0, 0, 255];
        a.legend = "隐区".into();
        d.areas.insert(ids[3], a);
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let mine = |s: &Scene| {
            let c = |k: u8| s.lines.iter().any(|l| l.color[0] == k && l.color[1] == 0) || s.fills.iter().any(|f| f.1[0] == k && f.1[1] == 0) || s.labels.iter().any(|l| l.color[0] == k && l.color[1] == 0);
            [c(2), c(3), c(4), c(5)]
        };
        let theme = map_core::theme::evaluate(&d);
        let before = build_scene(&d, &theme, &opts);
        assert_eq!(mine(&before), [true; 4]);
        for w in [FeatureRef::Text(ids[0]), FeatureRef::Symbol(ids[1]), FeatureRef::Route(ids[2]), FeatureRef::Area(ids[3])] {
            d.set_hidden(w, true).unwrap();
        }
        let theme = map_core::theme::evaluate(&d);
        let hidden = build_scene(&d, &theme, &opts);
        assert_eq!(mine(&hidden), [false; 4]);
        let legend: Vec<String> = hidden.legend.as_ref().map(|l| l.1.iter().map(|x| x.0.clone()).collect()).unwrap_or_default();
        assert!(!legend.iter().any(|x| x == "隐路" || x == "隐区" || x == "战役"), "{legend:?}");
        d.set_hidden(FeatureRef::Route(ids[2]), false).unwrap();
        let theme = map_core::theme::evaluate(&d);
        assert_eq!(mine(&build_scene(&d, &theme, &opts))[2], true);
        assert!(d.set_hidden(FeatureRef::Area(ids[4]), true).is_err());
    }

    /// A front line exports its teeth (filled triangles on one side), a
    /// railway its ties (short strokes across); the legend swatch too.
    #[test]
    fn decorated_lines_reach_the_export() {
        use map_core::deco::Deco;
        use map_core::doc::{Dash, LineFeature, LineId, RouteStyle};
        use map_core::geom::LonLat;
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        for (deco, colour) in [(Deco::Teeth, [7u8, 1, 1, 255]), (Deco::Ticks, [7, 2, 2, 255])] {
            let mut d = map_core::sample::demo_world();
            let st = RouteStyle { color: colour, width: 2.0, dash: Dash::Solid, arrow: false, deco, ..Default::default() };
            let pts = vec![LonLat::new(-60.0, -50.0), LonLat::new(-20.0, -50.0)];
            d.rivers.insert(LineId(d.topo.alloc()), LineFeature { name: "线".into(), pts, route: Some(st), legend: "前线".into(), show_name: false, hidden: false });
            let theme = map_core::theme::evaluate(&d);
            let s = build_scene(&d, &theme, &opts);
            let fills = s.fills.iter().filter(|f| f.1 == colour).count();
            let lines = s.lines.iter().filter(|l| l.color == colour).count();
            match deco {
                Deco::Teeth => assert!(fills > 5 && lines == 1, "{fills} {lines}"),
                _ => assert!(fills == 0 && lines > 6, "{fills} {lines}"),
            }
            let (_, _, rows) = legend_layout(&s).unwrap();
            let row = rows.iter().find(|r| r.label == "前线").unwrap();
            assert!(row.parts.len() > 2, "the swatch is decorated too");
        }
    }

    /// The scale bar's length is a round number of km and matches the
    /// map's scale at the centre (checked against a region export of known
    /// size); a world map's bar carries the note that its scale varies; the
    /// north arrow points up at the central meridian.
    #[test]
    fn scale_bar_and_north_arrow() {
        let d = map_core::sample::demo_world();
        let theme = map_core::theme::evaluate(&d);
        let opts = ExportOptions { width: 1600, height: 900, scale_bar: true, north_arrow: true, legend: false, ..Default::default() };
        let full = build_scene(&d, &theme, &opts);
        // drawn over everything else, insets too
        let s = full.top.as_deref().expect("the scale bar and the arrow");
        let km = s.labels.iter().find(|l| l.text.ends_with(" km")).expect("a length");
        let value: f64 = km.text.trim_end_matches(" km").parse().unwrap();
        let mant = value / 10f64.powf(value.log10().floor());
        assert!([1.0, 2.0, 5.0].iter().any(|m| (m - mant).abs() < 1e-9), "{value}");
        // the bar: the dark rectangle ending at the length label
        let bar = s.fills.iter().find(|f| f.1 == [20, 20, 20, 255] && f.0.len() == 4).unwrap();
        let len = bar.0[1][0] - bar.0[0][0];
        assert!(len > 60.0 && len <= 160.0 + 1e-6, "{len}");
        // at the centre of an Equal Earth world map one degree of longitude
        // on the equator spans scale·k·π/180 px, about 111.2 km
        let r = d.radius();
        let b = opts.projection.bounds();
        let scale = (1600.0 / (b[2] - b[0])).min(900.0 / (b[3] - b[1]));
        let px_per_deg = (opts.projection.forward_deg(1.0, 0.0)[0] - opts.projection.forward_deg(0.0, 0.0)[0]) * scale;
        let km_per_px = r * std::f64::consts::PI / 180.0 / px_per_deg;
        assert!((value / km_per_px / len - 1.0).abs() < 0.01, "{value} km over {len} px");
        assert!(s.labels.iter().any(|l| l.text.contains("图中心")), "world maps say the scale varies");
        // north points up along the central meridian
        let dark = s.fills.iter().find(|f| f.1 == [20, 20, 20, 255] && f.0.len() == 3).unwrap();
        let light = s.fills.iter().find(|f| f.1 == [150, 150, 150, 255]).unwrap();
        let tip = dark.0[0];
        assert!(tip[1] < dark.0[1][1] && (tip[0] - light.0[0][0]).abs() < 1e-6);
        // (not a glyph of a country's name, written glyph by glyph)
        let north = |l: &SceneLabel| l.text == "北" && l.color != d.style.label;
        assert!(s.labels.iter().any(north));
        // off by default
        let plain = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, legend: false, ..Default::default() });
        assert!(!plain.labels.iter().any(|l| l.text.ends_with(" km") || north(l)) && plain.top.is_none());
        crate::png::render(&full, None).unwrap();
    }

    /// A character symbol exports as its character, centred and sized
    /// with the symbol; it joins the legend only with a legend text, and a
    /// built-in symbol's legend text replaces its kind's name.
    #[test]
    fn character_symbols_and_legend_texts() {
        use map_core::doc::LineId;
        use map_core::geom::LonLat;
        use map_core::symbol::{MapSymbol, SymbolKind};
        let mut d = map_core::sample::demo_world();
        let mut g = MapSymbol::new(SymbolKind::Glyph, LonLat::new(-40.0, -45.0));
        g.glyph = "寺".into();
        g.size = 10.0;
        g.color = [6, 6, 1, 255];
        let gid = LineId(d.topo.alloc());
        d.symbols.insert(gid, g);
        let mut p = MapSymbol::new(SymbolKind::Peak, LonLat::new(-20.0, -45.0));
        p.legend = "主峰".into();
        d.symbols.insert(LineId(d.topo.alloc()), p);
        let theme = map_core::theme::evaluate(&d);
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let s = build_scene(&d, &theme, &opts);
        let l = s.labels.iter().find(|l| l.text == "寺" && l.color == [6, 6, 1, 255]).expect("the character");
        assert!((l.size - map_core::symbol::glyph_px(10.0)).abs() < 1e-4);
        let names = |s: &Scene| s.legend.as_ref().map(|l| l.1.iter().map(|x| x.0.clone()).collect::<Vec<_>>()).unwrap_or_default();
        assert!(names(&s).contains(&"主峰".to_string()) && !names(&s).contains(&"山峰".to_string()));
        assert!(!names(&s).iter().any(|n| n == "字符" || n == "寺"), "no legend text, no row");
        d.symbols.get_mut(&gid).unwrap().legend = "寺院".into();
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &opts);
        let (_, _, rows) = legend_layout(&s).unwrap();
        let row = rows.iter().find(|r| r.label == "寺院").expect("a row");
        assert!(matches!(&row.parts[0], LegendPart::Text(t, _, _, c) if t == "寺" && *c == [6, 6, 1, 255]));
        crate::png::render(&s, None).unwrap();
        assert!(crate::svg::render(&s).contains("寺院"));
    }

    /// In a year after a country ended, its land, label (even a placed one)
    /// and legend row are gone from the export; a city founded later is
    /// not drawn before its founding.
    #[test]
    fn lifespans_reach_the_export() {
        use map_core::doc::{EntityKind, LabelPlace};
        let mut d = map_core::sample::demo_world();
        let e = d.countries()[0];
        let name = d.entities[&e].name.clone();
        let at = d.label_anchor(e).unwrap().0;
        d.entities.get_mut(&e).unwrap().label = Some(LabelPlace::at(at));
        d.entities.get_mut(&e).unwrap().ended = Some(1920);
        let city = *d.entities.iter().find(|(_, x)| x.kind == EntityKind::City).unwrap().0;
        d.entities.get_mut(&city).unwrap().founded = Some(1950);
        let opts = ExportOptions { width: 1600, height: 900, ..Default::default() };
        let at_year = |d: &mut map_core::Document, y: i32| {
            d.set_year(Some(y));
            let theme = map_core::theme::evaluate(d);
            build_scene(d, &theme, &opts)
        };
        let before = at_year(&mut d, 1910);
        assert!(before.labels.iter().any(|l| l.text == name));
        assert!(before.legend.as_ref().unwrap().1.iter().any(|x| x.0 == name));
        let dots_1910 = before.dots.len();
        let after = at_year(&mut d, 1930);
        assert!(!after.labels.iter().any(|l| l.text == name));
        assert!(!after.legend.as_ref().unwrap().1.iter().any(|x| x.0 == name));
        assert_eq!(after.dots.len(), dots_1910, "the city is not there in 1910 or 1930");
        let later = at_year(&mut d, 1960);
        assert_eq!(later.dots.len(), dots_1910 + 1, "founded in 1950");
    }

    /// A symbol is exported from the same marks as on screen (halo lines,
    /// then its body), with its name beside it, only in its years; with no
    /// year selected it always shows.
    #[test]
    fn symbols_reach_the_export() {
        use map_core::doc::LineId;
        use map_core::symbol::{MapSymbol, SymbolKind};
        let mut d = map_core::sample::demo_world();
        let mut star = MapSymbol::new(SymbolKind::Capital, map_core::geom::LonLat::new(-40.0, -45.0));
        star.color = [5, 6, 7, 255];
        star.name = "旧都".into();
        star.from = Some(1800);
        star.to = Some(1850);
        d.symbols.insert(LineId(d.topo.alloc()), star);
        let mut swords = MapSymbol::new(SymbolKind::Battle, map_core::geom::LonLat::new(-20.0, -45.0));
        swords.color = [9, 9, 9, 255];
        d.symbols.insert(LineId(d.topo.alloc()), swords);
        let theme = map_core::theme::evaluate(&d);
        let opts = ExportOptions { width: 3200, height: 1800, ..Default::default() };
        let s = build_scene(&d, &theme, &opts);
        // the star: one filled outline of ten points, sized with the export (k = 2)
        let star_fill: Vec<_> = s.fills.iter().filter(|f| f.1 == [5, 6, 7, 255]).collect();
        assert_eq!(star_fill.len(), 1);
        assert_eq!(star_fill[0].0.len(), 10);
        let ys: Vec<f64> = star_fill[0].0.iter().map(|p| p[1]).collect();
        let h = ys.iter().cloned().fold(f64::MIN, f64::max) - ys.iter().cloned().fold(f64::MAX, f64::min);
        assert!(h > 7.0 * 2.0 * 1.5 && h < 7.0 * 2.0 * 2.0, "{h}");
        // the swords: four strokes, each over a wider white halo
        let body: Vec<&SceneLine> = s.lines.iter().filter(|l| l.color == [9, 9, 9, 255]).collect();
        assert_eq!(body.len(), 4);
        assert!(s.lines.iter().filter(|l| l.color == [255, 255, 255, 220] && l.width > body[0].width).count() >= 4);
        let name = s.labels.iter().find(|l| l.text == "旧都").expect("name exported");
        assert_eq!(name.color, [5, 6, 7, 255]);
        crate::png::render(&s, None).unwrap();
        assert!(crate::svg::render(&s).contains("rgb(5,6,7)"));
        // outside its years the star is gone, the swords (no years) stay
        d.year = Some(1900);
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &opts);
        assert!(!s.fills.iter().any(|f| f.1 == [5, 6, 7, 255]) && !s.labels.iter().any(|l| l.text == "旧都"));
        assert_eq!(s.lines.iter().filter(|l| l.color == [9, 9, 9, 255]).count(), 4);
        d.year = Some(1820);
        let theme = map_core::theme::evaluate(&d);
        assert!(build_scene(&d, &theme, &opts).fills.iter().any(|f| f.1 == [5, 6, 7, 255]));
    }

    /// Curved text is exported glyph by glyph along its arc: one label per
    /// visible character, the outer ones turned and lower than the middle.
    #[test]
    fn curved_text_follows_its_arc() {
        use map_core::doc::{Annotation, LineId};
        let mut d = map_core::sample::demo_world();
        let mut a = Annotation::new("南方大洋", map_core::geom::LonLat::new(-30.0, -40.0));
        a.spacing = 0.0;
        a.bend = 60.0;
        a.color = [1, 2, 3, 255];
        d.annotations.insert(LineId(d.topo.alloc()), a);
        let theme = map_core::theme::evaluate(&d);
        let s = build_scene(&d, &theme, &ExportOptions { width: 1600, height: 900, ..Default::default() });
        let g: Vec<&SceneLabel> = s.labels.iter().filter(|l| l.color == [1, 2, 3, 255]).collect();
        assert_eq!(g.iter().map(|l| l.text.as_str()).collect::<String>(), "南方大洋");
        // arching up: ends below the middle, turned outwards
        assert!(g[0].pos[1] > g[1].pos[1] && g[3].pos[1] > g[2].pos[1]);
        assert!(g[0].angle > 0.0 && g[3].angle < 0.0 && (g[0].angle + g[3].angle).abs() < 1e-3);
        assert!(g[0].pos[0] < g[1].pos[0] && g[1].pos[0] < g[2].pos[0] && g[2].pos[0] < g[3].pos[0]);
        assert!(crate::svg::render(&s).contains("南"));
    }

    /// A placed label is always exported, where it was put and turned;
    /// a hidden one never is; automatic ones keep clear of placed ones.
    #[test]
    fn placed_and_hidden_labels_reach_the_export() {
        let mut d = map_core::sample::demo_world();
        let cs = d.countries();
        let (a, b) = (cs[0], cs[1]);
        let at = map_core::geom::LonLat::new(-30.0, -40.0); // open sea, far from every country
        d.entities.get_mut(&a).unwrap().label = Some(map_core::doc::LabelPlace { at, angle: 30.0, scale: 1.5 });
        d.entities.get_mut(&b).unwrap().label_hidden = true;
        let theme = map_core::theme::evaluate(&d);
        let o = ExportOptions::default();
        let s = build_scene(&d, &theme, &o);
        let (na, nb) = (d.entities[&a].name.clone(), d.entities[&b].name.clone());
        let la = s.labels.iter().find(|l| l.text == na).expect("placed label exported");
        assert_eq!(la.angle, 30.0);
        assert!(s.labels.iter().all(|l| l.text != nb), "hidden label left out");
        // where the point projects (centre x; baseline is a little below)
        let auto_size = s.labels.iter().filter(|l| l.angle == 0.0).map(|l| l.size).fold(0.0f32, f32::max);
        assert!(la.size > 0.0 && auto_size > 0.0);
        // SVG turns it about its centre, counter-clockwise on screen
        let svg = crate::svg::render(&s);
        assert!(svg.contains("rotate(-30.00"), "svg rotates the label");
        // PNG draws it (with a font, when one is on this machine)
        let font = crate::png::find_system_font();
        let pm = crate::png::render(&s, font.as_deref()).unwrap();
        assert_eq!((pm.width(), pm.height()), (s.width, s.height));
        // without the placement the label goes back to its automatic place
        d.entities.get_mut(&a).unwrap().label = None;
        let s2 = build_scene(&d, &theme, &o);
        assert!(s2.labels.iter().filter(|l| l.text == na).all(|l| l.angle == 0.0));
    }

    #[test]
    fn export_draws_the_overlay_layer_boundaries() {
        let mut d = map_core::sample::demo_world();
        let ctl = d.add_layer("实际控制");
        d.set_active_layer(ctl).unwrap();
        let x = d.add_entity(map_core::EntityKind::Country, "控制区");
        let f = *d.topo.faces.iter().find(|(_, f)| f.kind == FaceKind::Land).unwrap().0;
        d.assign(f, Some(x)).unwrap();
        d.set_active_layer(map_core::doc::POLITICAL).unwrap();
        let theme = map_core::theme::evaluate(&d);
        let o = ExportOptions { labels: false, legend: false, graticule: false, ..Default::default() };
        let count = |d: &Document| build_scene(d, &theme, &o).lines.iter().filter(|l| l.color == d.style.overlay_color).count();
        assert_eq!(count(&d), 0, "no overlay chosen");
        d.style.overlay_layer = Some(ctl);
        assert!(!d.layer_border_edges(ctl).is_empty());
        assert!(count(&d) >= d.layer_border_edges(ctl).len());
    }
    #[test]
    fn polyline_split_at_seam() {
        let parts = split_polyline(&[[170.0, 0.0], [190.0, 0.0]]);
        assert_eq!(parts.len(), 2);
        for p in &parts {
            assert!(p.iter().all(|q| q[0] >= -180.0 && q[0] <= 180.0));
        }
        let ring = vec![vec![[170.0, -5.0], [190.0, -5.0], [190.0, 5.0], [170.0, 5.0]]];
        let parts = split_strip(&ring);
        assert_eq!(parts.len(), 2);
        // a wavy coast crossing the seam three times: two separate parts on
        // the west side instead of one polygon folded along x = 180
        let wavy = vec![vec![[170.0, 0.0], [190.0, 0.0], [190.0, 4.0], [175.0, 4.0], [175.0, 6.0], [190.0, 6.0], [190.0, 10.0], [170.0, 10.0]]];
        let parts = split_strip(&wavy);
        let west = parts.iter().filter(|p| p[0].iter().all(|q| q[0] <= -179.999999)).count()
            + parts.iter().filter(|p| p[0].iter().any(|q| q[0] < 0.0)).count();
        assert!(parts.len() >= 3, "expected separate parts, got {}", parts.len());
        let _ = west;
        for p in &parts {
            assert!(geom::ring_is_simple(&p[0]));
        }
    }
}
