use map_core::{
    doc::LineId,
    pixel_art::PixelArt,
    symbol::{MapSymbol, SymbolKind},
    theme, Document, LonLat,
};
use std::sync::Arc;

#[test]
fn pixel_marker_roundtrips_and_exports_with_transparent_cells() {
    let mut doc = Document::default();
    let id = LineId(doc.topo.alloc());
    let mut symbol = MapSymbol::new(SymbolKind::Site, LonLat::new(0.0, 0.0));
    symbol.size = 64.0;
    symbol.legend = "彩旗".into();
    symbol.from = Some(2000);
    let red = [235, 30, 60, 255];
    let blue = [32, 91, 220, 255];
    let mut art = PixelArt::new(4, 2).unwrap();
    art.stroke([0, 0], [1, 0], 1, red);
    art.stroke([2, 1], [3, 1], 1, blue);
    symbol.pixels = Some(Arc::new(art));
    doc.symbols.insert(id, symbol.clone());
    let repeated = LineId(doc.topo.alloc());
    let mut copy = symbol.clone();
    copy.at = LonLat::new(55.0, 30.0);
    doc.symbols.insert(repeated, copy);
    doc.set_year(Some(2005));

    let encoded = map_io::project::to_bytes(&doc).unwrap();
    assert_eq!(u32::from_le_bytes(encoded[8..12].try_into().unwrap()), 11);
    let loaded = map_io::project::from_bytes(&encoded).unwrap();
    assert_eq!(loaded.symbols[&id], symbol);
    assert!(Arc::ptr_eq(
        loaded.symbols[&id].pixels.as_ref().unwrap(),
        loaded.symbols[&repeated].pixels.as_ref().unwrap(),
    ));
    let export = map_io::scene::ExportOptions {
        width: 1600,
        height: 800,
        legend: true,
        ..Default::default()
    };
    let scene = map_io::scene::build_scene(&loaded, &theme::evaluate(&loaded), &export);
    assert_eq!(scene.pixels.len(), 2);
    let [x, y, w, h] = scene.pixels[0].rect;
    assert_eq!(w, 2.0 * h);
    let sample = |image: &tiny_skia::Pixmap, dx: f32, dy: f32| {
        let point = image
            .pixel((x + dx * w) as u32, (y + dy * h) as u32)
            .unwrap();
        [point.red(), point.green(), point.blue(), point.alpha()]
    };
    let image = map_io::png::render(&scene, None).unwrap();
    assert_eq!(sample(&image, 0.125, 0.25), red);
    assert_eq!(sample(&image, 0.875, 0.75), blue);
    assert_eq!(sample(&image, 0.875, 0.25), loaded.style.ocean);
    assert!(map_io::svg::render(&scene).contains("shape-rendering=\"crispEdges\""));
    assert!(map_io::pdf::render(&scene, None).starts_with(b"%PDF"));
    let legend = &scene.legend.as_ref().unwrap().1;
    assert!(legend
        .iter()
        .any(|(_, key)| matches!(key, map_io::scene::LegendKey::Pixels(_))));

    let mut hidden = loaded;
    hidden.set_year(Some(1999));
    let scene = map_io::scene::build_scene(&hidden, &theme::evaluate(&hidden), &export);
    assert!(scene.pixels.is_empty());
}

#[test]
fn transparent_png_preserves_partial_alpha_and_pixels() {
    let path = std::env::temp_dir().join(format!("atlas-pixel-symbol-{}.png", std::process::id()));
    let art = PixelArt::from_pixels(
        2,
        2,
        vec![
            [0, 0, 0, 0],
            [95, 160, 0, 128],
            [200, 90, 20, 255],
            [0, 0, 0, 0],
        ],
    )
    .unwrap();
    map_io::png::save_pixel_art(&art, &path).unwrap();
    assert_eq!(map_io::png::load_pixel_art(&path).unwrap(), art);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn artboard_marks_keep_group_opacity_when_the_project_reopens() {
    use map_core::{features::FeatureRef, paper::Paper, world_book::WorldBook};

    let mut root = Document::default();
    let paper = Paper {
        width: 320,
        height: 240,
    };
    let board = WorldBook::edit(&mut root).add_map(Document::new_paper("制图符号画板", paper));
    WorldBook::switch(&mut root, board, None).unwrap();

    let mut symbol = MapSymbol::new(SymbolKind::Site, LonLat::new(0.0, 0.0));
    symbol.size = 100.0;
    symbol.pixels = Some(Arc::new(
        PixelArt::from_pixels(1, 1, vec![[255, 0, 0, 255]]).unwrap(),
    ));
    let id = LineId(root.topo.alloc());
    root.symbols.insert(id, symbol);
    let layer = root.add_group("标志");
    root.set_group(&[FeatureRef::Symbol(id)], Some(layer))
        .unwrap();
    root.set_group_opacity(layer, 0.5).unwrap();

    let bytes = map_io::project::to_bytes(&root).unwrap();
    assert_eq!(u32::from_le_bytes(bytes[8..12].try_into().unwrap()), 11);
    let mut reopened = map_io::project::from_bytes(&bytes).unwrap();
    assert!(reopened.symbols.is_empty());
    WorldBook::switch(&mut reopened, board, None).unwrap();
    let export = map_io::scene::ExportOptions {
        width: paper.width,
        height: paper.height,
        ..Default::default()
    };
    let scene = map_io::scene::build_scene(&reopened, &theme::evaluate(&reopened), &export);
    assert_eq!(scene.layers[0].pixels[0].opacity, 0.5);
    let pm = map_io::png::render(&scene, None).unwrap();
    let [x, y, w, h] = scene.layers[0].pixels[0].rect;
    let pixel = pm
        .pixel((x + w * 0.5) as u32, (y + h * 0.5) as u32)
        .unwrap();
    assert_eq!(pixel.red(), 255);
    assert!((126..=129).contains(&pixel.green()));
}

#[test]
fn pixel_marks_survive_the_clipboard_and_share_their_art() {
    use map_core::features::FeatureRef;

    // two marks drawn with one picture, a third with another
    let mut flag = PixelArt::new(16, 8).unwrap();
    flag.stroke([0, 0], [15, 7], 2, [235, 30, 60, 255]);
    let mut star = PixelArt::new(8, 8).unwrap();
    star.stroke([0, 4], [7, 4], 1, [250, 200, 40, 255]);
    let (flag, star) = (Arc::new(flag), Arc::new(star));
    let mut doc = Document::default();
    let mut marks = Vec::new();
    for (lon, art) in [(0.0, &flag), (20.0, &flag), (40.0, &star)] {
        let id = LineId(doc.topo.alloc());
        let mut symbol = MapSymbol::new(SymbolKind::Site, LonLat::new(lon, 10.0));
        symbol.pixels = Some(Arc::clone(art));
        doc.symbols.insert(id, symbol);
        marks.push(FeatureRef::Symbol(id));
    }
    // through the clipboard as text, as between two windows
    let text = map_io::clip::to_text(&doc.copy_features(&marks));
    let clip = map_io::clip::from_text(&text).expect("our own clipboard text");
    let art = |d: &Document, f: &FeatureRef| d.symbols[&f.id()].pixels.clone().expect("pixel art");

    // into another map: the same pictures, the repeated one held once
    let mut other = Document::default();
    let first = other.paste_features(&clip, None).unwrap();
    assert_eq!(*art(&other, &first[0]), *flag);
    assert_eq!(*art(&other, &first[2]), *star);
    assert!(Arc::ptr_eq(&art(&other, &first[0]), &art(&other, &first[1])));
    assert!(!Arc::ptr_eq(&art(&other, &first[0]), &art(&other, &first[2])));
    // pasted again there: the pictures that map already holds
    let again = other.paste_features(&clip, Some(LonLat::new(-60.0, -20.0))).unwrap();
    for (a, b) in first.iter().zip(&again) {
        assert!(Arc::ptr_eq(&art(&other, a), &art(&other, b)));
    }
    // back into the map they came from: its own pictures
    let back = doc.paste_features(&clip, None).unwrap();
    assert!(Arc::ptr_eq(&art(&doc, &back[0]), &flag) && Arc::ptr_eq(&art(&doc, &back[1]), &flag));
    assert!(Arc::ptr_eq(&art(&doc, &back[2]), &star));
    // and the copies export like the originals
    let scene = map_io::scene::build_scene(&other, &theme::evaluate(&other), &map_io::scene::ExportOptions { width: 800, height: 400, ..Default::default() });
    assert_eq!(scene.pixels.len(), 6);
}
