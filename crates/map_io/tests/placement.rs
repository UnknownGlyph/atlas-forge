//! The layout view's furniture placement and inset maps: every piece of
//! furniture reports the box it is drawn in; moved pieces are drawn where
//! they were put, in every format; insets are the map of another region in
//! a frame; placements are kept with saved layouts.

use map_core::layout::{Inset, Placement};
use map_io::scene::{build_scene, legend_layout, ExportOptions, Furniture};

fn opts() -> ExportOptions {
    ExportOptions { width: 1200, height: 650, title: "测试地图".into(), scale_bar: true, north_arrow: true, ..Default::default() }
}

fn boxes(o: &ExportOptions) -> Vec<(Furniture, [f32; 4])> {
    let d = map_core::sample::demo_world();
    let th = map_core::theme::evaluate(&d);
    build_scene(&d, &th, o).furniture
}

fn find(b: &[(Furniture, [f32; 4])], f: Furniture) -> [f32; 4] {
    b.iter().find(|x| x.0 == f).unwrap_or_else(|| panic!("{f:?} not drawn")).1
}

#[test]
fn furniture_sits_where_it_is_put() {
    let o = opts();
    let b = boxes(&o);
    let (w, h) = (o.width as f32, o.height as f32);
    for f in [Furniture::Title, Furniture::Legend, Furniture::ScaleBar, Furniture::NorthArrow] {
        let r = find(&b, f);
        assert!(r[0] >= 0.0 && r[1] >= 0.0 && r[0] + r[2] <= w + 1.0 && r[1] + r[3] <= h + 1.0, "{f:?} {r:?} off the page");
    }
    // the usual places: title on top, legend lower left, scale bar lower right
    assert!(find(&b, Furniture::Title)[1] < h * 0.1);
    assert!(find(&b, Furniture::Legend)[0] < w * 0.1);
    assert!(find(&b, Furniture::ScaleBar)[0] > w * 0.6);
    // moved: each box's top left is where it was put
    let moved = ExportOptions {
        placement: Placement { title: Some([0.05, 0.85]), legend: Some([0.7, 0.05]), scale_bar: Some([0.1, 0.1]), north_arrow: Some([0.5, 0.5]), insets: vec![] },
        ..opts()
    };
    let m = boxes(&moved);
    for (f, at) in [(Furniture::Title, [0.05, 0.85]), (Furniture::Legend, [0.7, 0.05]), (Furniture::ScaleBar, [0.1, 0.1]), (Furniture::NorthArrow, [0.5, 0.5])] {
        let r = find(&m, f);
        assert!((r[0] - at[0] * w).abs() < 0.5 && (r[1] - at[1] * h).abs() < 0.5, "{f:?} at {r:?}, wanted {:?}", [at[0] * w, at[1] * h]);
    }
    // the drawings follow: the title's text, the legend's box
    let d = map_core::sample::demo_world();
    let th = map_core::theme::evaluate(&d);
    let s = build_scene(&d, &th, &moved);
    let svg = map_io::svg::render(&s);
    assert!(svg.contains(&format!(r#"y="{:.1}" font-size="28.0""#, 0.85 * h + 30.0)), "the title is written at its new place");
    let (lb, _, _) = legend_layout(&s).unwrap();
    assert!((lb[0] - 0.7 * w).abs() < 0.5 && (lb[1] - 0.05 * h).abs() < 0.5);
    // and nothing else moved: without a placement the usual output
    let plain = build_scene(&d, &th, &opts());
    assert!(map_io::svg::render(&plain).contains(r#"y="40" font-size="28""#));
    // on a big page the title is larger, as the legend is
    let big = build_scene(&d, &th, &ExportOptions { width: 3200, height: 1700, ..opts() });
    assert!(map_io::svg::render(&big).contains(r#"font-size="56.0""#));
}

#[test]
fn an_inset_map_in_every_format() {
    let d = map_core::sample::demo_world();
    let th = map_core::theme::evaluate(&d);
    let world = opts().projection.bounds();
    // a corner of the world, in a frame at the lower right
    let bounds = [world[0], world[1], world[0] + (world[2] - world[0]) * 0.3, world[1] + (world[3] - world[1]) * 0.4];
    let o = ExportOptions { placement: Placement { insets: vec![Inset { rect: [0.6, 0.6, 0.35, 0.35], bounds, title: "西南一角".into() }], ..Default::default() }, ..opts() };
    let s = build_scene(&d, &th, &o);
    assert_eq!(s.insets.len(), 1);
    let inset = &s.insets[0];
    assert_eq!((inset.scene.width, inset.scene.height), ((0.35f32 * 1200.0).round() as u32, (0.35f32 * 650.0).round() as u32));
    assert!(inset.scene.legend.is_none() && inset.scene.title.is_empty(), "an inset has no legend or title of its own");
    assert!(s.furniture.iter().any(|f| f.0 == Furniture::Inset(0)));
    // SVG: nested, its ids its own
    let svg = map_io::svg::render(&s);
    assert!(svg.contains(r#"<svg id="inset0-map""#) && svg.contains(r#"id="inset0-areas""#) && svg.contains("西南一角"));
    assert_eq!(svg.matches(r#"id="areas""#).count(), 1, "no id twice");
    // the scale bar and the north arrow are drawn over the inset, never under it
    let (at_inset, at_furniture) = (svg.find("inset0-map").unwrap(), svg.find(r#"id="furniture""#).expect("furniture drawn"));
    assert!(at_furniture > at_inset);
    // PNG: the frame's inside differs from the page without the inset
    let with = map_io::png::render(&s, None).unwrap();
    let without = map_io::png::render(&build_scene(&d, &th, &opts()), None).unwrap();
    let px = |p: &tiny_skia::Pixmap, x: u32, y: u32| p.pixel(x, y).map(|c| (c.red(), c.green(), c.blue()));
    let differ = (0..40).filter(|i| {
        let (x, y) = (760 + i * 9, 400 + (i % 5) * 40);
        px(&with, x, y) != px(&without, x, y)
    });
    assert!(differ.count() > 10, "the inset is drawn in its frame");
    // PDF: drawn cut to its frame
    let pdf = map_io::pdf::render(&s, None);
    assert!(pdf.starts_with(b"%PDF"));
    // a layout keeps its placement; a page with an inset off it is refused
    let mut doc = d.clone();
    doc.save_layout(o.to_layout("带插图")).unwrap();
    assert_eq!(ExportOptions::from_layout(doc.layout("带插图").unwrap()).placement, o.placement);
    let back = map_io::project::from_bytes(&map_io::project::to_bytes(&doc).unwrap()).unwrap();
    assert_eq!(back.layout("带插图").unwrap().placement, o.placement);
    let mut bad = o.to_layout("坏");
    bad.placement.insets[0].rect = [0.8, 0.8, 0.5, 0.5];
    assert!(doc.save_layout(bad).is_err());
}
