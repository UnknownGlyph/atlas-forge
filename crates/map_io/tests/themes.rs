//! Map themes in exports: a theme's coast bands lie over the sea and under
//! the land, widest first; its tone changes the country colours (and the
//! legend's); a dark theme's light names get a dark halo; the default look
//! is untouched (the appearance test keeps it).

use map_core::basemap::THEMES;
use map_core::doc::Document;
use map_io::scene::{build_scene, ExportOptions, Scene};

fn scene(d: &Document) -> Scene {
    build_scene(d, &map_core::theme::evaluate(d), &ExportOptions { width: 900, height: 480, ..Default::default() })
}

fn themed(i: usize) -> Document {
    let mut d = map_core::sample::demo_world();
    THEMES[i].apply(&mut d.style, None);
    d
}

#[test]
fn the_default_theme_draws_no_bands() {
    let d = map_core::sample::demo_world();
    let s = scene(&d);
    assert!(s.glow.is_empty());
    assert!(!map_io::svg::render(&s).contains("coast-bands"));
    assert_eq!(map_core::basemap::current(&d.style).map(|t| t.name), Some("现代"));
}

#[test]
fn bands_along_the_coasts_widest_first() {
    let d = themed(1);
    let s = scene(&d);
    assert!(!s.glow.is_empty(), "古典图集 has coast bands");
    assert!(s.glow.windows(2).all(|w| w[0].width >= w[1].width), "widest first");
    let widths: std::collections::BTreeSet<u32> = s.glow.iter().map(|l| (l.width * 100.0) as u32).collect();
    assert_eq!(widths.len(), 3, "three bands");
    let svg = map_io::svg::render(&s);
    // over the sea (the first area), before the land
    let (sea, bands) = (svg.find("<path").unwrap(), svg.find("coast-bands").unwrap());
    let land = svg[bands..].find("</g>").unwrap() + bands;
    assert!(sea < bands && svg[land..].contains("<path"));
    // the picture differs from the default
    let plain = map_io::png::render(&scene(&map_core::sample::demo_world()), None).unwrap();
    let with = map_io::png::render(&s, None).unwrap();
    assert_ne!(plain.data(), with.data());
}

#[test]
fn a_tone_changes_country_colours_and_the_legend() {
    let plain = map_core::sample::demo_world();
    let d = themed(2);
    let (a, b) = (map_core::theme::evaluate(&plain), map_core::theme::evaluate(&d));
    let e = plain.countries()[0];
    assert_ne!(a.entity_colors[&e], b.entity_colors[&e]);
    assert_eq!(b.entity_colors[&e], d.style.toned(plain.entities[&e].color));
    assert_eq!(b.legend[0].color, d.style.toned(a.legend[0].color));
}

#[test]
fn a_dark_theme_writes_light_names_on_dark_halos() {
    let d = themed(3);
    let svg = map_io::svg::render(&scene(&d));
    let label = d.style.label;
    assert!(svg.contains(&format!("fill=\"rgb({},{},{})\"", label[0], label[1], label[2])));
    assert!(svg.contains("stroke=\"rgb(20,22,28)\""), "dark halos behind the light names");
}
