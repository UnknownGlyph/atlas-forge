//! Appearance regression: exported pictures of saved worlds must not change
//! when the program changes underneath (a new style system, a refactor of
//! the export). Reference pictures are kept in `tests/fixtures/appearance`.
//!
//! A reference is written once, when it does not exist yet, by:
//!   ATLAS_WRITE_FIXTURES=1 cargo test -p map_io --test appearance
//! Existing references are never rewritten by the test: a deliberate change
//! of the look means deleting the reference by hand, with the reason in the
//! commit, and writing it again.
//!
//! PNG pictures are compared pixel by pixel (a channel may differ by a
//! little, and only on a handful of pixels, so a different processor's
//! rounding does not count); they are drawn without a font, so text is
//! checked through the SVG, which is compared exactly.

use map_core::doc::Document;
use map_io::scene::{build_scene, ExportOptions};
use std::path::PathBuf;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("appearance")
}

fn v1_world() -> Document {
    map_io::project::load(&PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("fixtures").join("v1_world.atlas")).expect("v1 sample")
}

fn options() -> ExportOptions {
    ExportOptions { width: 1000, height: 540, ..Default::default() }
}

/// Write the reference if asked and missing; otherwise it must exist.
fn reference(name: &str, made: &[u8]) -> Option<Vec<u8>> {
    let path = dir().join(name);
    match std::fs::read(&path) {
        Ok(b) => Some(b),
        Err(_) if std::env::var_os("ATLAS_WRITE_FIXTURES").is_some() => {
            std::fs::create_dir_all(dir()).unwrap();
            std::fs::write(&path, made).unwrap();
            eprintln!("wrote {}", path.display());
            None
        }
        Err(e) => panic!("reference {} is missing ({e}); write it with ATLAS_WRITE_FIXTURES=1", path.display()),
    }
}

fn check_png(name: &str, d: &Document) {
    let scene = build_scene(d, &map_core::theme::evaluate(d), &options());
    let pm = map_io::png::render(&scene, None).unwrap();
    let made = pm.encode_png().unwrap();
    let Some(want) = reference(name, &made) else { return };
    let want = image::load_from_memory(&want).unwrap().to_rgba8();
    assert_eq!((want.width(), want.height()), (pm.width(), pm.height()), "{name}: size");
    // tiny-skia stores premultiplied colour; compare what a viewer sees
    let got = image::load_from_memory(&made).unwrap().to_rgba8();
    let (mut off, mut worst) = (0usize, 0u8);
    for (a, b) in want.pixels().zip(got.pixels()) {
        let d = a.0.iter().zip(b.0.iter()).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
        worst = worst.max(d);
        if d > 2 {
            off += 1;
        }
    }
    let total = (want.width() * want.height()) as usize;
    assert!(off * 2000 <= total, "{name}: {off} of {total} pixels differ (by up to {worst}); the picture changed");
}

fn check_svg(name: &str, d: &Document) {
    let scene = build_scene(d, &map_core::theme::evaluate(d), &options());
    let made = map_io::svg::render(&scene);
    let Some(want) = reference(name, made.as_bytes()) else { return };
    let want = String::from_utf8(want).unwrap();
    if want != made {
        let line = want.lines().zip(made.lines()).position(|(a, b)| a != b).unwrap_or(want.lines().count().min(made.lines().count()));
        panic!("{name}: the SVG changed, first at line {} (expected {:?}, got {:?})", line + 1, want.lines().nth(line), made.lines().nth(line));
    }
}

#[test]
fn saved_worlds_look_as_they_did() {
    let d = v1_world();
    check_png("v1_latest.png", &d);
    check_svg("v1_latest.svg", &d);
    let mut y = d.clone();
    y.set_year(Some(1500));
    check_png("v1_1500.png", &y);
    check_png("demo.png", &map_core::sample::demo_world());
}
