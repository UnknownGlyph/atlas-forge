//! PNG export with an independent CPU rasteriser (tiny-skia), so any output
//! resolution is possible without GPU read-back.

use crate::scene::{legend_layout, text_width, title_box, LegendPart, Scene, SceneLine};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont};
use tiny_skia::{FillRule, FilterQuality, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Stroke, Transform};

fn paint(c: [u8; 4]) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

/// A line with round caps and joins, dashed when it says so.
fn stroke_line(pm: &mut Pixmap, l: &SceneLine) {
    stroke_line_masked(pm, l, None);
}

fn stroke_line_masked(pm: &mut Pixmap, l: &SceneLine, mask: Option<&tiny_skia::Mask>) {
    let mut pb = PathBuilder::new();
    for (i, q) in l.pts.iter().enumerate() {
        if i == 0 {
            pb.move_to(q[0] as f32, q[1] as f32);
        } else {
            pb.line_to(q[0] as f32, q[1] as f32);
        }
    }
    if let Some(path) = pb.finish() {
        let dash = l.dash.and_then(|[a, b]| tiny_skia::StrokeDash::new(vec![a, b], 0.0));
        let st = Stroke { width: l.width, line_cap: LineCap::Round, line_join: LineJoin::Round, dash, ..Default::default() };
        pm.stroke_path(&path, &paint(l.color), &st, Transform::identity(), mask);
    }
}

/// A filled polygon (any shape, nonzero rule).
fn fill_poly(pm: &mut Pixmap, poly: &[[f64; 2]], c: [u8; 4]) {
    let mut pb = PathBuilder::new();
    for (i, q) in poly.iter().enumerate() {
        if i == 0 {
            pb.move_to(q[0] as f32, q[1] as f32);
        } else {
            pb.line_to(q[0] as f32, q[1] as f32);
        }
    }
    pb.close();
    if let Some(path) = pb.finish() {
        pm.fill_path(&path, &paint(c), FillRule::Winding, Transform::identity(), None);
    }
}

/// Rasterise a scene. `font` should contain CJK glyphs (e.g. Microsoft
/// YaHei); without a font, text is skipped.
pub fn render(s: &Scene, font: Option<&[u8]>) -> Result<Pixmap, String> {
    let font_bytes = font;
    let mut pm = Pixmap::new(s.width, s.height).ok_or("图片尺寸无效")?;
    let bg = s.background;
    pm.fill(tiny_skia::Color::from_rgba8(bg[0], bg[1], bg[2], bg[3]));
    let id = Transform::identity();
    let fill_polys = |pm: &mut Pixmap, polys: &[crate::scene::ScenePoly]| {
        for p in polys {
            let mut pb = PathBuilder::new();
            for r in &p.rings {
                for (i, q) in r.iter().enumerate() {
                    if i == 0 {
                        pb.move_to(q[0] as f32, q[1] as f32);
                    } else {
                        pb.line_to(q[0] as f32, q[1] as f32);
                    }
                }
                pb.close();
            }
            if let Some(path) = pb.finish() {
                pm.fill_path(&path, &paint(p.fill), FillRule::EvenOdd, id, None);
            }
        }
    };
    // the sea, the bands along the coasts, then the land
    fill_polys(&mut pm, &s.polys[..s.polys.len().min(1)]);
    // (kept to the sea's outline: none spill past the map's edge)
    let sea = s.polys.first().and_then(|p| {
        let mut pb = PathBuilder::new();
        for r in &p.rings {
            for (i, q) in r.iter().enumerate() {
                if i == 0 {
                    pb.move_to(q[0] as f32, q[1] as f32);
                } else {
                    pb.line_to(q[0] as f32, q[1] as f32);
                }
            }
            pb.close();
        }
        let path = pb.finish()?;
        let mut m = tiny_skia::Mask::new(pm.width(), pm.height())?;
        m.fill_path(&path, FillRule::EvenOdd, true, Transform::identity());
        Some(m)
    });
    for l in &s.glow {
        stroke_line_masked(&mut pm, l, sea.as_ref());
    }
    fill_polys(&mut pm, &s.polys[s.polys.len().min(1)..]);
    // the hill shading over the land
    if let Some(px) = &s.shade {
        let mut img = Pixmap::new(s.width, s.height).ok_or("图片尺寸无效")?;
        for (d, c) in img.data_mut().chunks_exact_mut(4).zip(px.chunks_exact(4)) {
            let a = c[3] as u16;
            d.copy_from_slice(&[(c[0] as u16 * a / 255) as u8, (c[1] as u16 * a / 255) as u8, (c[2] as u16 * a / 255) as u8, c[3]]);
        }
        pm.draw_pixmap(0, 0, img.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }
    for l in &s.lines {
        stroke_line(&mut pm, l);
    }
    for (poly, c) in &s.fills {
        fill_poly(&mut pm, poly, *c);
    }
    // the layers of drawn features, each whole, the top one last
    for layer in &s.layers {
        fill_polys(&mut pm, &layer.polys);
        for l in &layer.lines {
            stroke_line(&mut pm, l);
        }
        for (poly, c) in &layer.fills {
            fill_poly(&mut pm, poly, *c);
        }
    }
    for (p, r, c) in &s.dots {
        if let Some(path) = PathBuilder::from_circle(p[0] as f32, p[1] as f32, *r) {
            pm.fill_path(&path, &paint(*c), FillRule::Winding, id, None);
            pm.stroke_path(&path, &paint([255, 255, 255, 255]), &Stroke { width: 1.0, ..Default::default() }, id, None);
        }
    }
    let base = font.and_then(|f| FontRef::try_from_slice_and_index(f, 0).ok());
    let symbols = symbol_font().and_then(|b| FontRef::try_from_slice_and_index(b, 0).ok());
    let font = base.clone().map(|main| Fonts { main, fallback: symbols.iter().cloned().collect() });
    // the fonts labels ask for, read once (a missing one: the default font)
    type Face = (std::sync::Arc<Vec<u8>>, u32);
    let mut named: std::collections::HashMap<(String, bool), Face> = std::collections::HashMap::new();
    for l in &s.labels {
        if let Some(f) = &l.font {
            if let std::collections::hash_map::Entry::Vacant(e) = named.entry((f.clone(), l.bold)) {
                if let Some(got) = crate::fonts::load(f, l.bold) {
                    e.insert(got);
                }
            }
        }
    }
    let named: std::collections::HashMap<(String, bool), FontRef> = named.iter().filter_map(|(k, (b, i))| FontRef::try_from_slice_and_index(b, *i).ok().map(|f| (k.clone(), f))).collect();
    let fonts_for = |l: &crate::scene::SceneLabel| -> Option<Fonts> {
        match l.font.as_ref().and_then(|f| named.get(&(f.clone(), l.bold))) {
            // its own font, then the default and the symbols for what it lacks
            Some(own) => Some(Fonts { main: own.clone(), fallback: base.iter().chain(symbols.iter()).cloned().collect() }),
            None => font.clone(),
        }
    };
    if font.is_some() || !named.is_empty() {
        for l in &s.labels {
            let Some(font) = fonts_for(l) else { continue };
            let font = &font;
            // centred on its real width in this font
            let tw = measure(font, &l.text, l.size);
            if l.angle.abs() < 0.01 && !l.italic {
                draw_text(&mut pm, font, &l.text, l.pos[0] as f32 - tw * 0.5, l.pos[1] as f32, l.size, l.color, true);
                continue;
            }
            // a turned or slanted label: drawn upright on its own layer,
            // then slanted and turned about its centre onto the map
            let pad = l.size * 0.5;
            let (w, h) = ((tw + 2.0 * pad).ceil() as u32, (l.size * 1.6 + 2.0 * pad).ceil() as u32);
            let Some(mut layer) = Pixmap::new(w.max(1), h.max(1)) else { continue };
            let baseline = pad + l.size * 1.1;
            draw_text(&mut layer, font, &l.text, pad, baseline, l.size, l.color, true);
            let (cx, cy) = (w as f32 * 0.5, baseline - l.size * 0.35);
            let slant = if l.italic { Transform::from_row(1.0, 0.0, -0.2, 1.0, 0.0, 0.0) } else { Transform::identity() };
            let t = Transform::from_translate(-cx, -cy).post_concat(slant).post_concat(Transform::from_rotate(-l.angle)).post_translate(l.pos[0] as f32, l.pos[1] as f32 - l.size * 0.35);
            pm.draw_pixmap(0, 0, layer.as_ref(), &PixmapPaint { quality: FilterQuality::Bilinear, ..Default::default() }, t, None);
        }
    }
    // inset maps, each drawn as a picture of its own in its frame
    for inset in &s.insets {
        let sub = render(&inset.scene, font_bytes)?;
        let r = inset.rect;
        pm.draw_pixmap(r[0].round() as i32, r[1].round() as i32, sub.as_ref(), &PixmapPaint::default(), id, None);
        if let Some(rect) = Rect::from_xywh(r[0].round(), r[1].round(), sub.width() as f32, sub.height() as f32) {
            let frame = Stroke { width: 1.5, ..Default::default() };
            pm.stroke_path(&PathBuilder::from_rect(rect), &paint([60, 60, 60, 255]), &frame, id, None);
        }
        if let (false, Some(fonts)) = (inset.title.is_empty(), &font) {
            let size = 13.0;
            if let Some(tag) = Rect::from_xywh(r[0] + 1.0, r[1] + 1.0, text_width(&inset.title, size) + 10.0, size * 1.6) {
                pm.fill_rect(tag, &paint([255, 255, 255, 230]), id, None);
            }
            draw_text(&mut pm, fonts, &inset.title, r[0] + 6.0, r[1] + size * 1.25, size, [30, 30, 30, 255], false);
        }
    }
    // the scale bar and the north arrow, over the insets
    if let Some(t) = &s.top {
        let top = render(t, font_bytes)?;
        pm.draw_pixmap(0, 0, top.as_ref(), &PixmapPaint::default(), id, None);
    }
    if let Some((bx, size, rows)) = legend_layout(s) {
        if let Some(rect) = Rect::from_xywh(bx[0], bx[1], bx[2], bx[3]) {
            pm.fill_rect(rect, &paint([255, 255, 255, 230]), id, None);
            let path = PathBuilder::from_rect(rect);
            pm.stroke_path(&path, &paint([136, 136, 136, 255]), &Stroke::default(), id, None);
        }
        if let Some(font) = &font {
            let title = &s.legend.as_ref().unwrap().0;
            draw_text(&mut pm, font, title, bx[0] + size * 0.6, bx[1] + size * 1.5, size * 1.1, [0, 0, 0, 255], false);
        }
        for row in rows {
            for part in &row.parts {
                match part {
                    LegendPart::Rect(r, c) => {
                        if let Some(rect) = Rect::from_xywh(r[0], r[1], r[2], r[3]) {
                            pm.fill_rect(rect, &paint(*c), id, None);
                        }
                    }
                    LegendPart::Line(l) => stroke_line(&mut pm, l),
                    LegendPart::Fill(poly, c) => fill_poly(&mut pm, poly, *c),
                    LegendPart::Text(t, at, size, c) => {
                        if let Some(font) = &font {
                            draw_text(&mut pm, font, t, at[0], at[1], *size, *c, false);
                        }
                    }
                }
            }
            if let Some(font) = &font {
                draw_text(&mut pm, font, &row.label, row.text[0], row.text[1], size, [0, 0, 0, 255], false);
            }
        }
    }
    if !s.title.is_empty() {
        if let Some(font) = &font {
            // its usual place, unchanged, or where the layout view put it
            let (x, base, size) = match title_box(s) {
                Some((_, at, size)) if s.title_at.is_some() || size != 28.0 => (at[0], at[1], size),
                _ => (s.width as f32 / 2.0 - text_width(&s.title, 28.0) * 0.5, 40.0, 28.0),
            };
            draw_text(&mut pm, font, &s.title, x, base, size, [0, 0, 0, 255], true);
        }
    }
    Ok(pm)
}

#[allow(clippy::too_many_arguments)]
/// The main font, and one to draw the characters it lacks (symbols such
/// as ✠ that CJK fonts do not have).
#[derive(Clone)]
struct Fonts<'a> {
    main: FontRef<'a>,
    /// Tried in turn for a character the main font lacks.
    fallback: Vec<FontRef<'a>>,
}

impl<'a> Fonts<'a> {
    /// The font to draw a character with.
    fn for_char(&self, ch: char) -> &FontRef<'a> {
        if self.main.glyph_id(ch).0 != 0 {
            return &self.main;
        }
        self.fallback.iter().find(|f| f.glyph_id(ch).0 != 0).unwrap_or(&self.main)
    }
}

/// The width of a text in these fonts.
fn measure(fonts: &Fonts, text: &str, size: f32) -> f32 {
    let scale = PxScale::from(size);
    text.chars().map(|ch| {
        let f = fonts.for_char(ch);
        f.as_scaled(scale).h_advance(f.glyph_id(ch))
    }).sum()
}

/// A system font with many symbols, read once.
pub(crate) fn symbol_font() -> Option<&'static [u8]> {
    static FONT: std::sync::OnceLock<Option<Vec<u8>>> = std::sync::OnceLock::new();
    FONT.get_or_init(|| {
        const CANDIDATES: &[&str] = &[r"C:\Windows\Fonts\seguisym.ttf", "/System/Library/Fonts/Apple Symbols.ttf", "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf"];
        CANDIDATES.iter().find_map(|p| std::fs::read(p).ok())
    })
    .as_deref()
}

fn draw_text(pm: &mut Pixmap, font: &Fonts, text: &str, x: f32, baseline: f32, size: f32, color: [u8; 4], halo: bool) {
    if halo {
        // approximate a round stroke with rings of offset copies
        let r = (size * 0.12).max(1.0);
        for ring in [r, r * 0.5] {
            for k in 0..16 {
                let a = k as f32 * std::f32::consts::TAU / 16.0;
                // (as see-through as the text it is behind)
                let h = map_core::doc::halo_for(color);
                draw_glyphs(pm, font, text, x + ring * a.cos(), baseline + ring * a.sin(), size, [h[0], h[1], h[2], color[3]]);
            }
        }
    }
    draw_glyphs(pm, font, text, x, baseline, size, color);
}

fn draw_glyphs(pm: &mut Pixmap, fonts: &Fonts, text: &str, x: f32, baseline: f32, size: f32, color: [u8; 4]) {
    let scale = PxScale::from(size);
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let data = pm.data_mut();
    let mut pen = x;
    for ch in text.chars() {
        // the main font, or a fallback for a character it lacks
        let font = fonts.for_char(ch);
        let gid = font.glyph_id(ch);
        let glyph = gid.with_scale_and_position(scale, ab_glyph::point(pen, baseline));
        pen += font.as_scaled(scale).h_advance(gid);
        let Some(outline) = font.outline_glyph(glyph) else { continue };
        let bb = outline.px_bounds();
        outline.draw(|gx, gy, cov| {
            let px = bb.min.x as i32 + gx as i32;
            let py = bb.min.y as i32 + gy as i32;
            if px < 0 || py < 0 || px >= w || py >= h {
                return;
            }
            let a = cov.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
            let i = ((py * w + px) * 4) as usize;
            for k in 0..3 {
                let src = color[k] as f32 * a;
                data[i + k] = (src + data[i + k] as f32 * (1.0 - a)).round() as u8;
            }
            data[i + 3] = (255.0 * a + data[i + 3] as f32 * (1.0 - a)).round() as u8;
        });
    }
}

pub fn save(s: &Scene, font: Option<&[u8]>, path: &std::path::Path) -> Result<(), String> {
    let pm = render(s, font)?;
    pm.save_png(path).map_err(|e| format!("保存 PNG 失败: {e}"))
}

/// The default font: the one that comes with Atlas Forge (Noto Sans SC,
/// see [`crate::fonts::bundled`]), else a system font with Chinese
/// characters (Windows / macOS / Linux common paths).
pub fn find_system_font() -> Option<Vec<u8>> {
    if let Some((_, bytes)) = crate::fonts::bundled() {
        return Some(bytes);
    }
    const CANDIDATES: &[&str] = &[
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\msyh.ttf",
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
    ];
    CANDIDATES.iter().find_map(|p| std::fs::read(p).ok())
}
