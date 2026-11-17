//! Make reference images with known georeferencing from a project, for
//! checking underlay alignment by eye:
//! `underlay_images <project.atlas> <out dir>` writes
//! `world_equirect.png` (whole world, equirectangular, 2400×1200) and
//! `world_mercator_150.png` (Mercator centred on 150°E, a "foreign" scan)
//! and `underlay_test.atlas` (the project with both as underlays).

use map_core::projection::Projection;
use map_io::scene::{build_scene, ExportOptions};

fn main() {
    let mut a = std::env::args().skip(1);
    let (Some(src), Some(out)) = (a.next(), a.next()) else {
        eprintln!("usage: underlay_images <project.atlas> <out dir>");
        std::process::exit(2);
    };
    let doc = map_io::project::load(std::path::Path::new(&src)).expect("load");
    let theme = map_core::theme::evaluate(&doc);
    let out = std::path::PathBuf::from(out);
    for (name, proj, lon0, w) in [("world_equirect.png", Projection::Equirectangular, 0.0, 2400u32), ("world_mercator_150.png", Projection::Mercator, 150.0, 1600)] {
        let b = proj.bounds();
        let h = (w as f64 * (b[3] - b[1]) / (b[2] - b[0])).round() as u32;
        let o = ExportOptions { width: w, height: h, projection: proj, lon0, labels: false, legend: false, graticule: true, background: [200, 225, 245, 255], ..Default::default() };
        let s = build_scene(&doc, &theme, &o);
        map_io::png::save(&s, None, &out.join(name)).expect("png");
        println!("{name}: {w}x{h}");
    }
    // the project with both images: the equirectangular one aligns as
    // imported; the Mercator one needs aligning (hidden at first)
    let mut doc = doc;
    doc.underlays.push(map_io::underlay::import(&out.join("world_equirect.png")).unwrap());
    let mut m = map_io::underlay::import(&out.join("world_mercator_150.png")).unwrap();
    m.visible = false;
    doc.underlays.push(m);
    map_io::project::save(&out.join("underlay_test.atlas"), &doc).unwrap();
    println!("underlay_test.atlas");
}
