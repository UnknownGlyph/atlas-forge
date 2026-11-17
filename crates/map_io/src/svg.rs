//! SVG export: real vector paths (even-odd fill for holes), text labels, legend.

use crate::scene::{legend_layout, text_width, title_box, LegendPart, Scene, SceneLine};
use std::fmt::Write;

/// The fonts the picture asks for when a label names none (or the one it
/// names is missing on the viewer's computer).
const FALLBACK_FONTS: &str = "Noto Sans SC, Microsoft YaHei, PingFang SC, Noto Sans CJK SC, sans-serif";

fn col(c: [u8; 4]) -> String {
    format!("rgb({},{},{})", c[0], c[1], c[2])
}
fn op(c: [u8; 4]) -> String {
    if c[3] == 255 {
        String::new()
    } else {
        format!(" fill-opacity=\"{:.3}\"", c[3] as f32 / 255.0)
    }
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A line as a path (round caps and joins, dashed when it says so).
fn line_path(l: &SceneLine) -> String {
    let mut d = String::new();
    for (i, q) in l.pts.iter().enumerate() {
        let _ = write!(d, "{}{:.2},{:.2}", if i == 0 { "M" } else { "L" }, q[0], q[1]);
    }
    format!(
        r#"<path d="{d}" fill="none" stroke="{}" stroke-opacity="{:.3}" stroke-width="{:.2}" stroke-linecap="round" stroke-linejoin="round"{}/>"#,
        col(l.color),
        l.color[3] as f32 / 255.0,
        l.width,
        l.dash.map_or(String::new(), |[a, b]| format!(r#" stroke-dasharray="{a:.2} {b:.2}""#))
    )
}

/// A filled polygon as a path.
fn fill_path(poly: &[[f64; 2]], c: [u8; 4]) -> String {
    let mut d = String::new();
    for (i, q) in poly.iter().enumerate() {
        let _ = write!(d, "{}{:.2},{:.2}", if i == 0 { "M" } else { "L" }, q[0], q[1]);
    }
    format!(r#"<path d="{d}Z" fill="{}" fill-opacity="{:.3}" stroke="none"/>"#, col(c), c[3] as f32 / 255.0)
}

pub fn render(s: &Scene) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{FALLBACK_FONTS}">"#,
        w = s.width,
        h = s.height
    );
    if s.background[3] > 0 {
        let _ = writeln!(o, r#"<rect width="100%" height="100%" fill="{}"/>"#, col(s.background));
    }
    o.push_str("<g id=\"areas\" stroke=\"none\">\n");
    for (i, p) in s.polys.iter().enumerate() {
        // the bands along the coasts: over the sea, under the land
        if i == 1 && !s.glow.is_empty() {
            // (kept to the sea's outline: none spill past the map's edge)
            let mut d = String::new();
            for r in &s.polys[0].rings {
                for (k, q) in r.iter().enumerate() {
                    let _ = write!(d, "{}{:.2},{:.2}", if k == 0 { "M" } else { "L" }, q[0], q[1]);
                }
                d.push('Z');
            }
            let _ = writeln!(o, r#"<clipPath id="sea"><path d="{d}"/></clipPath>"#);
            o.push_str("<g id=\"coast-bands\" clip-path=\"url(#sea)\" fill=\"none\" stroke-linejoin=\"round\" stroke-linecap=\"round\">\n");
            for l in &s.glow {
                let _ = writeln!(o, "{}", line_path(l));
            }
            o.push_str("</g>\n");
        }
        let mut d = String::new();
        for r in &p.rings {
            for (i, q) in r.iter().enumerate() {
                let _ = write!(d, "{}{:.2},{:.2}", if i == 0 { "M" } else { "L" }, q[0], q[1]);
            }
            d.push('Z');
        }
        let _ = writeln!(o, r#"<path d="{d}" fill="{}"{} fill-rule="evenodd"/>"#, col(p.fill), op(p.fill));
    }
    // the hill shading over the land, as a picture
    if let Some(px) = &s.shade {
        if let Some(png) = png_bytes(s.width, s.height, px) {
            let _ = writeln!(o, r#"<image id="hill-shading" x="0" y="0" width="{}" height="{}" href="data:image/png;base64,{}"/>"#, s.width, s.height, base64(&png));
        }
    }
    o.push_str("</g>\n<g id=\"lines\" fill=\"none\" stroke-linejoin=\"round\" stroke-linecap=\"round\">\n");
    for l in &s.lines {
        let _ = writeln!(o, "{}", line_path(l));
    }
    for (poly, c) in &s.fills {
        let _ = writeln!(o, "{}", fill_path(poly, *c));
    }
    // the layers of drawn features, each a group of its own, the top one last
    for layer in &s.layers {
        let _ = writeln!(o, r#"<g class="layer" data-name="{}">"#, esc(&layer.name));
        for p in &layer.polys {
            let mut d = String::new();
            for r in &p.rings {
                for (i, q) in r.iter().enumerate() {
                    let _ = write!(d, "{}{:.2},{:.2}", if i == 0 { "M" } else { "L" }, q[0], q[1]);
                }
                d.push('Z');
            }
            let _ = writeln!(o, r#"<path d="{d}" fill="{}"{} stroke="none" fill-rule="evenodd"/>"#, col(p.fill), op(p.fill));
        }
        for l in &layer.lines {
            let _ = writeln!(o, "{}", line_path(l));
        }
        for (poly, c) in &layer.fills {
            let _ = writeln!(o, "{}", fill_path(poly, *c));
        }
        o.push_str("</g>\n");
    }
    o.push_str("</g>\n<g id=\"cities\">\n");
    for (p, r, c) in &s.dots {
        let _ = writeln!(o, r#"<circle cx="{:.2}" cy="{:.2}" r="{r:.2}" fill="{}" stroke="white" stroke-width="1"/>"#, p[0], p[1], col(*c));
    }
    o.push_str("</g>\n<g id=\"labels\" text-anchor=\"middle\" paint-order=\"stroke\" stroke=\"white\" stroke-linejoin=\"round\">\n");
    for l in &s.labels {
        let _ = writeln!(
            o,
            r#"<text x="{:.2}" y="{:.2}" font-size="{:.1}" fill="{}"{} stroke-width="{:.1}"{}{}{}{}>{}</text>"#,
            l.pos[0],
            l.pos[1],
            l.size,
            col(l.color),
            // text in a see-through layer, halo and all
            if l.color[3] == 255 { String::new() } else { format!(r#" opacity="{:.3}""#, l.color[3] as f32 / 255.0) },
            l.size * 0.22,
            // a dark halo behind light text (the group's is white)
            if map_core::doc::halo_for(l.color) == [255, 255, 255, 255] { String::new() } else { format!(r#" stroke="{}""#, col(map_core::doc::halo_for(l.color))) },
            // turned about the label's centre (counter-clockwise on screen)
            if l.angle.abs() < 0.01 { String::new() } else { format!(r#" transform="rotate({:.2} {:.2} {:.2})""#, -l.angle, l.pos[0], l.pos[1] - l.size as f64 * 0.35) },
            if l.italic { r#" font-style="italic""# } else { "" },
            // its own font first, the document's defaults after it
            match &l.font {
                Some(f) => format!(r#" font-family="{}, {FALLBACK_FONTS}"{}"#, esc(f), if l.bold { r#" font-weight="bold""# } else { "" }),
                None if l.bold => r#" font-weight="bold""#.to_string(),
                None => String::new(),
            },
            esc(&l.text)
        );
    }
    o.push_str("</g>\n");
    // inset maps: each a picture of its own nested in its frame (its ids
    // prefixed, so its clip paths do not stand for the main map's)
    for (i, inset) in s.insets.iter().enumerate() {
        let r = inset.rect;
        let inner = render(&inset.scene);
        let body = inner.split_once('\n').map_or("", |(_, rest)| rest).trim_end().trim_end_matches("</svg>");
        let pre = format!("inset{i}-");
        let body = body.replace("id=\"", &format!("id=\"{pre}")).replace("url(#", &format!("url(#{pre}")).replace("href=\"#", &format!("href=\"#{pre}"));
        let (w, h) = (inset.scene.width, inset.scene.height);
        let _ = writeln!(o, r#"<svg id="{pre}map" x="{:.1}" y="{:.1}" width="{w}" height="{h}" viewBox="0 0 {w} {h}" overflow="hidden">"#, r[0], r[1]);
        o.push_str(&body);
        o.push_str("</svg>\n");
        let _ = writeln!(o, r##"<rect x="{:.1}" y="{:.1}" width="{w}" height="{h}" fill="none" stroke="#3c3c3c" stroke-width="1.5"/>"##, r[0], r[1]);
        if !inset.title.is_empty() {
            let _ = writeln!(o, r#"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="20.8" fill="white" fill-opacity="0.9"/>"#, r[0] + 1.0, r[1] + 1.0, text_width(&inset.title, 13.0) + 10.0);
            let _ = writeln!(o, r##"<text x="{:.1}" y="{:.1}" font-size="13" fill="#1e1e1e">{}</text>"##, r[0] + 6.0, r[1] + 16.25, esc(&inset.title));
        }
    }
    // the scale bar and the north arrow, over the insets
    if let Some(t) = &s.top {
        let inner = render(t);
        let body = inner.split_once('\n').map_or("", |(_, rest)| rest).trim_end().trim_end_matches("</svg>");
        let body = body.replace("id=\"", "id=\"top-").replace("url(#", "url(#top-").replace("href=\"#", "href=\"#top-");
        o.push_str("<g id=\"furniture\">\n");
        o.push_str(&body);
        o.push_str("</g>\n");
    }
    if let Some((bx, size, rows)) = legend_layout(s) {
        let title = &s.legend.as_ref().unwrap().0;
        let _ = writeln!(
            o,
            r##"<g id="legend"><rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="white" fill-opacity="0.9" stroke="#888"/>"##,
            bx[0], bx[1], bx[2], bx[3]
        );
        let _ = writeln!(o, r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}" font-weight="bold">{}</text>"#, bx[0] + size * 0.6, bx[1] + size * 1.5, size * 1.1, esc(title));
        for row in rows {
            for part in &row.parts {
                let _ = match part {
                    LegendPart::Rect(r, c) => writeln!(
                        o,
                        r##"<rect x="{:.1}" y="{:.1}" width="{:.1}" height="{:.1}" fill="{}"{} stroke="#555" stroke-width="0.5"/>"##,
                        r[0],
                        r[1],
                        r[2],
                        r[3],
                        col(*c),
                        op(*c)
                    ),
                    LegendPart::Line(l) => writeln!(o, "{}", line_path(l)),
                    LegendPart::Fill(poly, c) => writeln!(o, "{}", fill_path(poly, *c)),
                    LegendPart::Text(t, at, size, c) => writeln!(o, r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}" fill="{}">{}</text>"#, at[0], at[1], size, col(*c), esc(t)),
                };
            }
            let _ = writeln!(o, r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}">{}</text>"#, row.text[0], row.text[1], size, esc(&row.label));
        }
        o.push_str("</g>\n");
    }
    match title_box(s) {
        // moved, or larger on a big page
        Some((b, at, size)) if s.title_at.is_some() || size != 28.0 => {
            let _ = writeln!(o, r#"<text x="{:.1}" y="{:.1}" font-size="{:.1}" text-anchor="middle" font-weight="bold">{}</text>"#, b[0] + b[2] * 0.5, at[1], size, esc(&s.title));
        }
        _ if !s.title.is_empty() => {
            let _ = writeln!(o, r#"<text x="{:.1}" y="40" font-size="28" text-anchor="middle" font-weight="bold">{}</text>"#, s.width as f32 / 2.0, esc(&s.title));
        }
        _ => {}
    }
    o.push_str("</svg>\n");
    o
}

/// A straight-alpha RGBA picture as PNG bytes.
fn png_bytes(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let img = image::RgbaImage::from_raw(w, h, rgba.to_vec())?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// Standard base64 (with padding).
fn base64(b: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for k in 0..4 {
            if k <= c.len() {
                o.push(T[(n >> (18 - 6 * k) & 63) as usize] as char);
            } else {
                o.push('=');
            }
        }
    }
    o
}

#[cfg(test)]
mod tests {
    #[test]
    fn base64_as_everyone_writes_it() {
        assert_eq!(super::base64(b""), "");
        assert_eq!(super::base64(b"f"), "Zg==");
        assert_eq!(super::base64(b"fo"), "Zm8=");
        assert_eq!(super::base64(b"foo"), "Zm9v");
        assert_eq!(super::base64(b"foobar"), "Zm9vYmFy");
    }
}
