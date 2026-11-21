//! PDF export: the same scene as the PNG and the SVG, as vector paths on
//! one page (a pixel of the picture is 0.75 pt, i.e. the picture at 96
//! dpi). Text is written as the outlines of its glyphs, taken from the
//! fonts the labels name (or the default font, then a symbol font for
//! what it lacks), so the file needs no embedded fonts and looks the same
//! in every viewer and printer. The page's drawing is compressed.

use crate::scene::{legend_layout, LegendPart, Scene, SceneLabel, SceneLine};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;
use ttf_parser::{Face, GlyphId, OutlineBuilder};

mod book_pages;
pub(crate) use book_pages::render as dossier_pdf;

/// Points per pixel of the picture.
const PT: f64 = 0.75;

/// A font's file and the face in it.
type FontFile = (std::sync::Arc<Vec<u8>>, u32);

/// The drawing of the page, and the opacities it uses.
struct Page {
    ops: String,
    alphas: BTreeMap<u16, String>,
}

fn rgb(c: [u8; 4]) -> String {
    format!("{:.3} {:.3} {:.3}", c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0)
}

impl Page {
    /// Use opacity `a` for what follows (fill and stroke alike).
    fn alpha(&mut self, a: u8) {
        let key = (a as u32 * 1000 / 255) as u16;
        let n = self.alphas.len();
        let name = self.alphas.entry(key).or_insert_with(|| format!("GS{n}")).clone();
        let _ = writeln!(self.ops, "/{name} gs");
    }

    fn poly(&mut self, pts: &[[f64; 2]], close: bool) {
        for (i, q) in pts.iter().enumerate() {
            let _ = write!(self.ops, "{:.2} {:.2} {} ", q[0], q[1], if i == 0 { "m" } else { "l" });
        }
        if close {
            self.ops.push_str("h ");
        }
        self.ops.push('\n');
    }

    /// Rings filled even-odd (holes stay open).
    fn fill(&mut self, rings: &[Vec<[f64; 2]>], c: [u8; 4]) {
        if rings.iter().all(|r| r.len() < 3) {
            return;
        }
        self.ops.push_str("q\n");
        self.alpha(c[3]);
        let _ = writeln!(self.ops, "{} rg", rgb(c));
        for r in rings.iter().filter(|r| r.len() >= 3) {
            self.poly(r, true);
        }
        self.ops.push_str("f*\nQ\n");
    }

    /// A line with round caps and joins, dashed when it says so.
    fn stroke(&mut self, l: &SceneLine) {
        if l.pts.len() < 2 {
            return;
        }
        self.ops.push_str("q\n");
        self.alpha(l.color[3]);
        let _ = writeln!(self.ops, "{} RG {:.2} w 1 J 1 j", rgb(l.color), l.width);
        if let Some([a, b]) = l.dash {
            let _ = writeln!(self.ops, "[{a:.2} {b:.2}] 0 d");
        }
        self.poly(&l.pts, false);
        self.ops.push_str("S\nQ\n");
    }

    /// A circle as four Bézier arcs.
    fn circle(&mut self, c: [f64; 2], r: f64) {
        let k = 0.552_284_75 * r;
        let (x, y) = (c[0], c[1]);
        let _ = writeln!(
            self.ops,
            "{:.2} {:.2} m {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c {:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c h",
            x + r, y,
            x + r, y + k, x + k, y + r, x, y + r,
            x - k, y + r, x - r, y + k, x - r, y,
            x - r, y - k, x - k, y - r, x, y - r,
            x + k, y - r, x + r, y - k, x + r, y
        );
    }
}

/// The faces a piece of text is written in: its own, then the fallbacks.
struct Faces<'a> {
    list: Vec<Face<'a>>,
}

impl Faces<'_> {
    /// Each character's face and glyph (none: a character no face has).
    fn glyphs(&self, text: &str) -> Vec<Option<(usize, GlyphId)>> {
        text.chars().map(|ch| self.list.iter().enumerate().find_map(|(i, f)| f.glyph_index(ch).filter(|g| g.0 != 0).map(|g| (i, g)))).collect()
    }

    /// Width of the text at `size` px.
    fn width(&self, text: &str, size: f64) -> f64 {
        let fallback = size * 0.5;
        text.chars()
            .zip(self.glyphs(text))
            .map(|(ch, g)| match g {
                Some((i, g)) => {
                    let f = &self.list[i];
                    f.glyph_hor_advance(g).unwrap_or(0) as f64 * size / f.units_per_em() as f64
                }
                None if ch == ' ' => size * 0.3,
                None => fallback,
            })
            .sum()
    }
}

/// Glyph outlines into page drawing, through a transform.
struct Pen<'a> {
    out: &'a mut String,
    t: &'a dyn Fn(f32, f32) -> [f64; 2],
    last: [f32; 2],
}

impl OutlineBuilder for Pen<'_> {
    fn move_to(&mut self, x: f32, y: f32) {
        let p = (self.t)(x, y);
        let _ = write!(self.out, "{:.2} {:.2} m ", p[0], p[1]);
        self.last = [x, y];
    }
    fn line_to(&mut self, x: f32, y: f32) {
        let p = (self.t)(x, y);
        let _ = write!(self.out, "{:.2} {:.2} l ", p[0], p[1]);
        self.last = [x, y];
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        // as a cubic: the control points two thirds of the way to the quadratic's
        let [x0, y0] = self.last;
        let c1 = (self.t)(x0 + (x1 - x0) * 2.0 / 3.0, y0 + (y1 - y0) * 2.0 / 3.0);
        let c2 = (self.t)(x + (x1 - x) * 2.0 / 3.0, y + (y1 - y) * 2.0 / 3.0);
        let p = (self.t)(x, y);
        let _ = write!(self.out, "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", c1[0], c1[1], c2[0], c2[1], p[0], p[1]);
        self.last = [x, y];
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (c1, c2, p) = ((self.t)(x1, y1), (self.t)(x2, y2), (self.t)(x, y));
        let _ = write!(self.out, "{:.2} {:.2} {:.2} {:.2} {:.2} {:.2} c ", c1[0], c1[1], c2[0], c2[1], p[0], p[1]);
        self.last = [x, y];
    }
    fn close(&mut self) {
        self.out.push_str("h ");
    }
}

/// How a piece of text is set.
struct Setting {
    size: f64,
    /// Where the baseline's middle (or start) is.
    at: [f64; 2],
    centred: bool,
    /// Degrees, counter-clockwise on the page, about the text's middle.
    angle: f64,
    italic: bool,
    color: [u8; 4],
    halo: Option<[u8; 4]>,
    /// Drawn a little heavier (no bold face at hand).
    heavy: bool,
}

/// The outlines of `text`, as one path (none when nothing can be drawn).
fn outlines(faces: &Faces, text: &str, s: &Setting) -> Option<String> {
    let width = faces.width(text, s.size);
    let x0 = if s.centred { s.at[0] - width * 0.5 } else { s.at[0] };
    let base = s.at[1];
    // turned about the middle of the text, as in the SVG
    let pivot = [s.at[0] + if s.centred { 0.0 } else { width * 0.5 }, base - s.size * 0.35];
    let (sa, ca) = (-s.angle).to_radians().sin_cos();
    let turn = |p: [f64; 2]| -> [f64; 2] {
        let (dx, dy) = (p[0] - pivot[0], p[1] - pivot[1]);
        [pivot[0] + dx * ca - dy * sa, pivot[1] + dx * sa + dy * ca]
    };
    let mut path = String::new();
    let mut pen_x = x0;
    for (ch, g) in text.chars().zip(faces.glyphs(text)) {
        match g {
            Some((i, gid)) => {
                let f = &faces.list[i];
                let k = s.size / f.units_per_em() as f64;
                let (ox, italic) = (pen_x, s.italic);
                let t = move |x: f32, y: f32| -> [f64; 2] {
                    let (x, y) = (x as f64 * k, y as f64 * k);
                    turn([ox + x + if italic { y * 0.2 } else { 0.0 }, base - y])
                };
                let mut pen = Pen { out: &mut path, t: &t, last: [0.0, 0.0] };
                f.outline_glyph(gid, &mut pen);
                pen_x += f.glyph_hor_advance(gid).unwrap_or(0) as f64 * k;
            }
            None => pen_x += if ch == ' ' { s.size * 0.3 } else { s.size * 0.5 },
        }
    }
    (!path.is_empty()).then_some(path)
}

fn text(page: &mut Page, faces: Option<&Faces>, text: &str, s: &Setting) {
    let Some(path) = faces.and_then(|f| outlines(f, text, s)) else { return };
    page.ops.push_str("q\n");
    page.alpha(s.color[3]);
    if let Some(h) = s.halo {
        let _ = writeln!(page.ops, "{} RG {:.2} w 1 j 1 J", rgb(h), s.size * 0.22);
        page.ops.push_str(&path);
        page.ops.push_str("S\n");
    }
    let _ = writeln!(page.ops, "{} rg", rgb(s.color));
    page.ops.push_str(&path);
    page.ops.push_str("f\n");
    if s.heavy {
        let _ = writeln!(page.ops, "{} RG {:.2} w 1 j", rgb(s.color), s.size * 0.04);
        page.ops.push_str(&path);
        page.ops.push_str("S\n");
    }
    page.ops.push_str("Q\n");
}

/// A PDF string in UTF-16 (for the document's title).
fn utf16_hex(s: &str) -> String {
    let mut o = String::from("<FEFF");
    for u in s.encode_utf16() {
        let _ = write!(o, "{u:04X}");
    }
    o.push('>');
    o
}

/// The scene as a PDF file. `font`: the default font's file (the one the
fn pixel_rects(page: &mut Page, marks: &[crate::scene::PixelMark]) {
    if marks.is_empty() { return; }
    page.ops.push_str("q\n");
    for mark in marks {
        for ([x, y, w, h], color) in mark.runs() {
            page.alpha(color[3]);
            let _ = writeln!(page.ops, "{} rg {x:.4} {y:.4} {w:.4} {h:.4} re f", rgb(color));
        }
    }
    page.ops.push_str("Q\n");
}

/// Uses the fallback faces for labels that name none or name an unavailable font.
/// The map of a scene (not its legend or title) onto a page, in the
/// scene's pixels: background, areas, coast bands, hill shading (when
/// `shade` and the page has it), lines, symbols, features, cities, labels.
fn draw_map(page: &mut Page, s: &Scene, fallback: &[Face], shade: bool) {
    let (w, h) = (s.width as f64, s.height as f64);
    if s.background[3] > 0 {
        page.fill(&[vec![[0.0, 0.0], [w, 0.0], [w, h], [0.0, h]]], s.background);
    }
    // the sea, the bands along the coasts (kept to the sea), the land
    if let Some(sea) = s.polys.first() {
        page.fill(&sea.rings, sea.fill);
        if !s.glow.is_empty() {
            page.ops.push_str("q\n");
            for r in sea.rings.iter().filter(|r| r.len() >= 3) {
                page.poly(r, true);
            }
            page.ops.push_str("W* n\n");
            for l in &s.glow {
                page.stroke(l);
            }
            page.ops.push_str("Q\n");
        }
    }
    for p in s.polys.iter().skip(1) {
        page.fill(&p.rings, p.fill);
    }
    // the hill shading over the land (an image; its first row at the top)
    if shade && s.shade.is_some() {
        let _ = writeln!(page.ops, "q {w:.2} 0 0 -{h:.2} 0 {h:.2} cm /Shade Do Q");
    }
    for l in &s.lines {
        page.stroke(l);
    }
    for (poly, c) in &s.fills {
        page.fill(std::slice::from_ref(poly), *c);
    }
    pixel_rects(page, &s.pixels);
    for layer in &s.layers {
        for p in &layer.polys {
            page.fill(&p.rings, p.fill);
        }
        for l in &layer.lines {
            page.stroke(l);
        }
        for (poly, c) in &layer.fills {
            page.fill(std::slice::from_ref(poly), *c);
        }
        pixel_rects(page, &layer.pixels);
    }
    for (p, r, c) in &s.dots {
        page.ops.push_str("q\n");
        page.alpha(c[3]);
        let _ = writeln!(page.ops, "{} rg 1 1 1 RG 1 w", rgb(*c));
        page.circle(*p, *r as f64);
        page.ops.push_str("B\nQ\n");
    }
    let mut named: HashMap<(String, bool), Option<FontFile>> = HashMap::new();
    for l in &s.labels {
        if let Some(f) = &l.font {
            named.entry((f.clone(), l.bold)).or_insert_with(|| crate::fonts::load(f, l.bold));
        }
    }
    let faces_for = |l: &SceneLabel| -> Faces {
        let own = l.font.as_ref().and_then(|f| named.get(&(f.clone(), l.bold))).and_then(|x| x.as_ref()).and_then(|(b, i)| Face::parse(b.as_slice(), *i).ok());
        Faces { list: own.into_iter().chain(fallback.iter().cloned()).collect() }
    };
    for l in &s.labels {
        let faces = faces_for(l);
        let setting = Setting { size: l.size as f64, at: l.pos, centred: true, angle: l.angle as f64, italic: l.italic, color: l.color, halo: Some(map_core::doc::halo_for(l.color)), heavy: l.bold && l.font.is_none() };
        text(page, Some(&faces), &l.text, &setting);
    }
}

pub fn render(s: &Scene, font: Option<&[u8]>) -> Vec<u8> {
    let mut page = Page { ops: String::new(), alphas: BTreeMap::new() };
    let (w, h) = (s.width as f64, s.height as f64);
    // pixels, y down, as in the scene
    let _ = writeln!(page.ops, "{PT} 0 0 -{PT} 0 {:.2} cm", h * PT);
    // fonts: the default and a symbol font after it; the ones labels name
    let symbols = crate::png::symbol_font();
    let fallback: Vec<Face> = font.and_then(|b| Face::parse(b, 0).ok()).into_iter().chain(symbols.and_then(|b| Face::parse(b, 0).ok())).collect();
    draw_map(&mut page, s, &fallback, true);
    let plain = Faces { list: fallback.clone() };
    let set = |at: [f64; 2], size: f64, color: [u8; 4], centred: bool, heavy: bool| Setting { size, at, centred, angle: 0.0, italic: false, color, halo: None, heavy };
    // inset maps: each drawn in its frame (moved there, cut to it); no
    // hill shading inside (the shading is the page's one picture)
    for inset in &s.insets {
        let r = inset.rect;
        let (iw, ih) = (inset.scene.width as f64, inset.scene.height as f64);
        let _ = writeln!(page.ops, "q 1 0 0 1 {:.2} {:.2} cm 0 0 {iw:.2} {ih:.2} re W n", r[0], r[1]);
        draw_map(&mut page, &inset.scene, &fallback, false);
        page.ops.push_str("Q\n");
        let (x0, y0, x1, y1) = (r[0] as f64, r[1] as f64, r[0] as f64 + iw, r[1] as f64 + ih);
        page.stroke(&SceneLine { color: [60, 60, 60, 255], width: 1.5, pts: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]], dash: None });
        if !inset.title.is_empty() {
            let tw = crate::scene::text_width(&inset.title, 13.0) as f64;
            page.fill(&[vec![[x0 + 1.0, y0 + 1.0], [x0 + tw + 11.0, y0 + 1.0], [x0 + tw + 11.0, y0 + 21.8], [x0 + 1.0, y0 + 21.8]]], [255, 255, 255, 230]);
            text(&mut page, Some(&plain), &inset.title, &set([x0 + 6.0, y0 + 16.25], 13.0, [30, 30, 30, 255], false, false));
        }
    }
    // the scale bar and the north arrow, over the insets
    if let Some(t) = &s.top {
        draw_map(&mut page, t, &fallback, false);
    }
    if let Some((bx, size, rows)) = legend_layout(s) {
        let title = &s.legend.as_ref().unwrap().0;
        page.fill(&[vec![[bx[0] as f64, bx[1] as f64], [(bx[0] + bx[2]) as f64, bx[1] as f64], [(bx[0] + bx[2]) as f64, (bx[1] + bx[3]) as f64], [bx[0] as f64, (bx[1] + bx[3]) as f64]]], [255, 255, 255, 230]);
        let r = [bx[0] as f64, bx[1] as f64, (bx[0] + bx[2]) as f64, (bx[1] + bx[3]) as f64];
        page.stroke(&SceneLine { color: [136, 136, 136, 255], width: 1.0, pts: vec![[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]], [r[0], r[1]]], dash: None });
        text(&mut page, Some(&plain), title, &set([(bx[0] + size * 0.6) as f64, (bx[1] + size * 1.5) as f64], (size * 1.1) as f64, [0, 0, 0, 255], false, true));
        for row in rows {
            for part in &row.parts {
                match part {
                    LegendPart::Rect(q, c) => {
                        let (x0, y0, x1, y1) = (q[0] as f64, q[1] as f64, (q[0] + q[2]) as f64, (q[1] + q[3]) as f64);
                        page.fill(&[vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]], *c);
                        page.stroke(&SceneLine { color: [85, 85, 85, 255], width: 0.5, pts: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1], [x0, y0]], dash: None });
                    }
                    LegendPart::Line(l) => page.stroke(l),
                    LegendPart::Fill(poly, c) => page.fill(std::slice::from_ref(poly), *c),
                    LegendPart::Text(t, at, sz, c) => text(&mut page, Some(&plain), t, &set([at[0] as f64, at[1] as f64], *sz as f64, *c, false, false)),
                }
            }
            text(&mut page, Some(&plain), &row.label, &set([row.text[0] as f64, row.text[1] as f64], size as f64, [0, 0, 0, 255], false, false));
        }
    }
    if !s.title.is_empty() {
        // its usual place (top centre), or where the layout view put it
        let (at, size) = match crate::scene::title_box(s) {
            Some((b, at, size)) if s.title_at.is_some() || size != 28.0 => ([(b[0] + b[2] * 0.5) as f64, at[1] as f64], size as f64),
            _ => ([w * 0.5, 40.0], 28.0),
        };
        text(&mut page, Some(&plain), &s.title, &Setting { halo: Some([255, 255, 255, 255]), ..set(at, size, [0, 0, 0, 255], true, true) });
    }

    // the file: catalogue, page tree, page, its drawing, information
    let content = miniz_oxide::deflate::compress_to_vec_zlib(page.ops.as_bytes(), 6);
    let gs: String = page.alphas.iter().map(|(a, n)| format!("/{n} << /ca {:.3} /CA {:.3} >> ", *a as f64 / 1000.0, *a as f64 / 1000.0)).collect();
    let mut out: Vec<u8> = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    let mut obj = |out: &mut Vec<u8>, body: &[u8]| {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", offsets.len()).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, b"<< /Type /Catalog /Pages 2 0 R >>");
    obj(&mut out, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>");
    // the shading, when there is one: objects 6 (colour) and 7 (its alpha)
    let xobj = if s.shade.is_some() { " /XObject << /Shade 6 0 R >>" } else { "" };
    obj(&mut out, format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {:.2} {:.2}] /Resources << /ExtGState << {gs}>>{xobj} >> /Contents 4 0 R >>", w * PT, h * PT).as_bytes());
    let mut stream = format!("<< /Length {} /Filter /FlateDecode >>\nstream\n", content.len()).into_bytes();
    stream.extend_from_slice(&content);
    stream.extend_from_slice(b"\nendstream");
    obj(&mut out, &stream);
    obj(&mut out, format!("<< /Producer (Atlas Forge) /Title {} >>", utf16_hex(if s.title.is_empty() { "地图" } else { &s.title })).as_bytes());
    if let Some(px) = &s.shade {
        let rgb: Vec<u8> = px.as_chunks::<4>().0.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
        let alpha: Vec<u8> = px.as_chunks::<4>().0.iter().map(|c| c[3]).collect();
        for (body, extra, cs) in [(rgb, " /SMask 7 0 R", "/DeviceRGB"), (alpha, "", "/DeviceGray")] {
            let z = miniz_oxide::deflate::compress_to_vec_zlib(&body, 6);
            let mut img = format!("<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace {cs} /BitsPerComponent 8 /Filter /FlateDecode /Length {}{extra} >>\nstream\n", s.width, s.height, z.len()).into_bytes();
            img.extend_from_slice(&z);
            img.extend_from_slice(b"\nendstream");
            obj(&mut out, &img);
        }
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R /Info 5 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    out
}
