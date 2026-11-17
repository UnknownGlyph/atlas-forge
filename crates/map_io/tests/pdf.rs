//! PDF export: a well-formed one-page file (every cross-reference points
//! at its object), the same shapes as the other exports as vector paths,
//! text as glyph outlines (with a font on this computer), coast bands
//! kept to the sea, and a usable file without any font.

use map_io::scene::{build_scene, ExportOptions, Scene};

fn scene(d: &map_core::Document) -> Scene {
    build_scene(d, &map_core::theme::evaluate(d), &ExportOptions { width: 1000, height: 540, ..Default::default() })
}

/// The file's objects are where its cross-reference table says, and the
/// page's drawing, inflated.
fn check(pdf: &[u8]) -> String {
    assert!(pdf.starts_with(b"%PDF-1.4"));
    let find = |hay: &[u8], what: &[u8]| hay.windows(what.len()).rposition(|w| w == what);
    let at_text = |at: usize, n: usize| String::from_utf8_lossy(&pdf[at..(at + n).min(pdf.len())]).to_string();
    assert!(at_text(pdf.len() - 7, 7).trim_end().ends_with("%%EOF"));
    let sx = find(pdf, b"startxref
").unwrap();
    let xref: usize = at_text(sx + 10, 20).lines().next().unwrap().trim().parse().unwrap();
    let table = at_text(xref, 200);
    assert!(table.starts_with("xref"), "startxref points at the table");
    let rows: Vec<&str> = table.lines().skip(3).take_while(|l| l.ends_with(" n ")).collect();
    assert_eq!(rows.len(), 5, "catalogue, pages, page, drawing, information");
    for (i, r) in rows.iter().enumerate() {
        let at: usize = r[..10].parse().unwrap();
        assert!(at_text(at, 12).starts_with(&format!("{} 0 obj", i + 1)), "object {} at {at}", i + 1);
    }
    // the drawing's stream
    let start = pdf.windows(7).position(|w| w == b"stream
").unwrap() + 7;
    let head = at_text(start.saturating_sub(80), 80);
    let len: usize = head[head.find("/Length ").unwrap() + 8..].split_whitespace().next().unwrap().parse().unwrap();
    let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&pdf[start..start + len]).expect("a zlib stream");
    String::from_utf8(raw).unwrap()
}

#[test]
fn shapes_lines_and_text() {
    let d = map_core::sample::demo_world();
    let s = scene(&d);
    let font = map_io::png::find_system_font();
    let pdf = map_io::pdf::render(&s, font.as_deref());
    let ops = check(&pdf);
    // every area is a filled path (the page's own background besides)
    assert!(ops.matches("f*\n").count() > s.polys.len(), "{} fills for {} areas", ops.matches("f*\n").count(), s.polys.len());
    assert!(ops.matches("S\n").count() >= s.lines.len());
    // the y axis turned down, pixels as 0.75 pt
    assert!(ops.starts_with("0.75 0 0 -0.75 0 405.00 cm"));
    if font.is_some() {
        // names as glyph outlines (curves), one fill per label at least
        assert!(ops.contains(" c "), "glyph outlines");
        // (a glyph's path ends with "h", then the fill)
        let fills = ops.matches("h f\n").count();
        assert!(fills >= s.labels.len(), "{fills} text fills for {} labels", s.labels.len());
    }
    // the page is the picture's size in points
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/MediaBox [0 0 750.00 405.00]"));
}

#[test]
fn coast_bands_are_kept_to_the_sea_and_no_font_is_needed() {
    let mut d = map_core::sample::demo_world();
    map_core::basemap::THEMES[1].apply(&mut d.style, None);
    let s = scene(&d);
    let ops = check(&map_io::pdf::render(&s, None));
    assert!(ops.contains("W* n"), "the bands are clipped to the sea");
    // without the default font: still a whole file (checked above)
    // see-through colours use graphics states
    let text = String::from_utf8_lossy(&map_io::pdf::render(&s, None)).to_string();
    assert!(text.contains("/ExtGState << /GS") && text.contains(" /ca "));
}
