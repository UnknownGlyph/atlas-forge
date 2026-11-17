//! Fonts in exports: country names and map text are drawn in the fonts the
//! map names; a font this computer lacks falls back to the default one;
//! the SVG names the font, with the default fonts after it.

use map_core::doc::{Annotation, Document, LineId};
use map_core::LonLat;
use map_io::scene::{build_scene, ExportOptions, Scene};

fn scene(d: &Document) -> Scene {
    build_scene(d, &map_core::theme::evaluate(d), &ExportOptions { width: 900, height: 480, ..Default::default() })
}

fn png(d: &Document) -> Vec<u8> {
    let font = map_io::png::find_system_font();
    map_io::png::render(&scene(d), font.as_deref()).unwrap().data().to_vec()
}

/// The sample world with one piece of map text.
fn world() -> (Document, LineId) {
    let mut d = map_core::sample::demo_world();
    let id = LineId(d.topo.alloc());
    d.annotations.insert(id, Annotation::new("北海", LonLat::new(10.0, 40.0)));
    (d, id)
}

#[test]
fn the_scene_and_svg_name_the_fonts() {
    let (mut d, t) = world();
    d.style.label_font = Some("Some Serif".into());
    d.style.label_bold = true;
    {
        let a = d.annotations.get_mut(&t).unwrap();
        a.font = Some("Another & Font".into());
    }
    let s = scene(&d);
    // country names (whole, or glyph by glyph along a bent line) in their font, bold
    let names: Vec<_> = s.labels.iter().filter(|l| l.font.as_deref() == Some("Some Serif")).collect();
    assert!(names.len() >= d.countries().len() / 2, "{} labels in the names' font", names.len());
    assert!(names.iter().all(|l| l.bold && l.color == d.style.label));
    let text = s.labels.iter().find(|l| l.text.contains('北')).expect("the map text");
    assert_eq!((text.font.as_deref(), text.bold), (Some("Another & Font"), false));
    let svg = map_io::svg::render(&s);
    assert!(svg.contains(r#"font-family="Some Serif, Noto Sans SC, Microsoft YaHei"#) && svg.contains(r#"font-weight="bold""#));
    assert!(svg.contains(r#"font-family="Another &amp; Font, "#), "names are escaped");
    // a map naming no fonts: no font attributes on its labels
    let plain = map_io::svg::render(&scene(&world().0));
    assert_eq!(plain.matches("font-family=").count(), 1, "only the picture's default");
}

#[test]
fn a_missing_font_falls_back_to_the_default() {
    if map_io::png::find_system_font().is_none() {
        return;
    }
    let (d, t) = world();
    let before = png(&d);
    let mut missing = d.clone();
    missing.style.label_font = Some("No Such Font Anywhere".into());
    missing.annotations.get_mut(&t).unwrap().font = Some("No Such Font Either".into());
    assert_eq!(png(&missing), before, "drawn as without a font");
}

#[test]
fn a_named_font_changes_the_picture() {
    // SimSun (宋体) comes with Windows; elsewhere there is nothing to compare
    if map_io::png::find_system_font().is_none() || map_io::fonts::load("SimSun", false).is_none() {
        return;
    }
    let (d, t) = world();
    let before = png(&d);
    let mut serif = d.clone();
    serif.style.label_font = Some("SimSun".into());
    assert_ne!(png(&serif), before, "country names in 宋体");
    let mut text = d.clone();
    text.annotations.get_mut(&t).unwrap().font = Some("SimSun".into());
    assert_ne!(png(&text), before, "the map text in 宋体");
}

#[test]
fn the_bundled_font_is_the_default() {
    // in the source tree the tests find assets/fonts above their program
    let (path, bytes) = map_io::fonts::bundled().expect("assets/fonts/NotoSansSC-Regular.ttf");
    assert_eq!(map_io::png::find_system_font().as_deref(), Some(bytes.as_slice()), "the default font");
    let face = ttf_parser::Face::parse(&bytes, 0).expect("a font");
    for c in ['中', '国', '山', 'A', 'z', '0', '·'] {
        assert!(face.glyph_index(c).is_some(), "{c}");
    }
    // the regular weight (not the thin default of the variable font)
    assert_eq!(face.weight().to_number(), 400);
    // listed with the fonts of the system, and chosen by its name
    let (by_name, _) = map_io::fonts::load("Noto Sans SC", false).expect("found by name");
    assert_eq!(by_name.len(), bytes.len(), "{}", path.display());
    // and the names are drawn with it
    let (d, _) = world();
    let none = map_io::png::render(&scene(&d), None).unwrap().data().to_vec();
    assert_ne!(png(&d), none);
}
