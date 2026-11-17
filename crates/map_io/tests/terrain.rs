//! A generated world's terrain in files: kept when saved and read back,
//! and its hill shading over the land in PNG, SVG and PDF exports (none
//! when turned off, none for worlds without terrain).

use map_core::worldgen::{generate, WorldSpec};
use map_io::scene::{build_scene, ExportOptions};

fn world() -> map_core::Document {
    generate(&WorldSpec { seed: 2, countries: 8, rivers: 4, ..WorldSpec::default() }).unwrap().0
}

fn opts() -> ExportOptions {
    ExportOptions { width: 640, height: 340, ..Default::default() }
}

#[test]
fn terrain_is_saved_and_read_back() {
    let d = world();
    let back = map_io::project::from_bytes(&map_io::project::to_bytes(&d).unwrap()).unwrap();
    assert_eq!(back.terrain.as_deref(), d.terrain.as_deref());
    assert!(back.terrain.is_some());
    // a world without terrain stays without
    let demo = map_core::sample::demo_world();
    assert!(map_io::project::from_bytes(&map_io::project::to_bytes(&demo).unwrap()).unwrap().terrain.is_none());
}

#[test]
fn hill_shading_in_every_picture() {
    let d = world();
    let s = build_scene(&d, &map_core::theme::evaluate(&d), &opts());
    let shade = s.shade.as_ref().expect("hill shading");
    assert_eq!(shade.len(), 640 * 340 * 4);
    assert!(shade.as_chunks::<4>().0.iter().filter(|p| p[3] > 0).count() > 1000, "shading on the land");
    // turned off: none, and the PNG differs
    let mut flat = d.clone();
    flat.style.hillshade = 0.0;
    let f = build_scene(&flat, &map_core::theme::evaluate(&flat), &opts());
    assert!(f.shade.is_none());
    assert_ne!(map_io::png::render(&s, None).unwrap().data(), map_io::png::render(&f, None).unwrap().data());
    // SVG: an embedded picture
    let svg = map_io::svg::render(&s);
    assert!(svg.contains(r#"<image id="hill-shading""#) && svg.contains("data:image/png;base64,iVBORw0KGgo"));
    assert!(!map_io::svg::render(&f).contains("hill-shading"));
    // PDF: an image with its own alpha
    let pdf = map_io::pdf::render(&s, None);
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/XObject << /Shade 6 0 R >>") && text.contains("/Subtype /Image") && text.contains("/SMask 7 0 R"));
    // no terrain: nothing of it anywhere
    let demo = map_core::sample::demo_world();
    assert!(build_scene(&demo, &map_core::theme::evaluate(&demo), &opts()).shade.is_none());
}
