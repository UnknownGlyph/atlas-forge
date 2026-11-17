//! Layers of drawn features in the exported picture: a document without
//! layers is drawn as it always was; a layer's features are laid over
//! that in the layer's turn, as see-through as the layer; a hidden layer
//! leaves nothing, in the picture or the legend.

use map_core::doc::{Annotation, Document, LineFeature, LineId, RouteStyle};
use map_core::features::FeatureRef;
use map_core::symbol::{MapSymbol, SymbolKind};
use map_core::LonLat;
use map_io::scene::{build_scene, ExportOptions, Scene};

/// The sample world with a route, a river, a symbol and a text added.
fn world() -> (Document, [FeatureRef; 4]) {
    let mut d = map_core::sample::demo_world();
    let mut id = || LineId(d.topo.alloc());
    let (route, river, sym, text) = (id(), id(), id(), id());
    d.rivers.insert(route, LineFeature { name: "商路".into(), pts: vec![LonLat::new(-70.0, 10.0), LonLat::new(-50.0, 25.0)], route: Some(RouteStyle::default()), legend: "商路".into(), show_name: false, hidden: false });
    d.rivers.insert(river, LineFeature { name: "大河".into(), pts: vec![LonLat::new(50.0, 0.0), LonLat::new(60.0, 10.0)], route: None, legend: String::new(), show_name: true, hidden: false });
    let mut s = MapSymbol::new(SymbolKind::Battle, LonLat::new(-60.0, 20.0));
    s.name = "某战".into();
    s.legend = "战役".into();
    d.symbols.insert(sym, s);
    d.annotations.insert(text, Annotation::new("东海", LonLat::new(100.0, 10.0)));
    (d, [FeatureRef::Route(route), FeatureRef::River(river), FeatureRef::Symbol(sym), FeatureRef::Text(text)])
}

fn scene(d: &Document) -> Scene {
    build_scene(d, &map_core::theme::evaluate(d), &ExportOptions { width: 1200, height: 650, ..Default::default() })
}

fn label<'a>(s: &'a Scene, text: &str) -> &'a map_io::scene::SceneLabel {
    // (map text may be written with space between its characters)
    s.labels.iter().find(|l| l.text.split_whitespace().collect::<String>() == text).unwrap_or_else(|| panic!("no label {text} among {:?}", s.labels.iter().map(|l| &l.text).collect::<Vec<_>>()))
}

#[test]
fn without_layers_everything_is_where_it_was() {
    let (d, _) = world();
    let s = scene(&d);
    assert!(s.layers.is_empty());
    // an empty layer changes nothing
    let mut e = d.clone();
    e.add_group("空");
    let t = scene(&e);
    assert_eq!((t.polys.len(), t.lines.len(), t.fills.len(), t.labels.len()), (s.polys.len(), s.lines.len(), s.fills.len(), s.labels.len()));
    assert!(t.layers.len() == 1 && t.layers[0].lines.is_empty() && t.layers[0].fills.is_empty());
    assert_eq!(map_io::png::render(&s, None).unwrap().data(), map_io::png::render(&t, None).unwrap().data(), "the same picture");
}

#[test]
fn a_layer_is_laid_over_the_rest_as_see_through_as_it_is() {
    let (d, fs) = world();
    let base = scene(&d);
    let mut e = d.clone();
    let g = e.add_group("战争");
    e.set_group(&fs, Some(g)).unwrap();
    e.set_group_opacity(g, 0.5).unwrap();
    let s = scene(&e);
    // the shapes moved from the scene into the layer, none lost or doubled
    assert_eq!(s.layers.len(), 1);
    let l = &s.layers[0];
    assert_eq!(l.name, "战争");
    assert!(!l.lines.is_empty() && !l.fills.is_empty());
    assert_eq!((s.lines.len() + l.lines.len(), s.fills.len() + l.fills.len()), (base.lines.len(), base.fills.len()));
    assert_eq!(s.labels.len(), base.labels.len());
    // half as opaque: shapes and the features' labels, not other labels
    assert!(l.lines.iter().all(|x| x.color[3] <= 128) && l.fills.iter().all(|x| x.1[3] <= 128));
    for text in ["东海", "某战"] {
        assert_eq!((label(&base, text).color[3], label(&s, text).color[3]), (255, 128), "{text}");
    }
    let other = base.labels.iter().find(|l| !["东海", "某战", "大", "河"].contains(&l.text.split_whitespace().collect::<String>().as_str())).unwrap();
    assert_eq!(label(&s, &other.text).color[3], other.color[3]);
    // both pictures can be made, and differ
    assert_ne!(map_io::png::render(&s, None).unwrap().data(), map_io::png::render(&base, None).unwrap().data());
    let svg = map_io::svg::render(&s);
    assert!(svg.contains(r#"<g class="layer" data-name="战争">"#) && svg.contains(r#"opacity="0.502""#));
    // at full opacity the picture is the same as without the layer when
    // nothing overlaps (here the route, the symbol and the river are apart)
    e.set_group_opacity(g, 1.0).unwrap();
    let full = scene(&e);
    assert!(full.layers[0].lines.iter().any(|x| x.color[3] == 255));
}

#[test]
fn a_hidden_layer_leaves_nothing() {
    let (d, fs) = world();
    let base = scene(&d);
    let legend = |s: &Scene| s.legend.as_ref().map(|l| l.1.iter().map(|r| r.0.clone()).collect::<Vec<_>>()).unwrap_or_default();
    assert!(legend(&base).contains(&"商路".to_string()) && legend(&base).contains(&"战役".to_string()));
    let mut e = d.clone();
    let g = e.add_group("战争");
    e.set_group(&fs, Some(g)).unwrap();
    e.set_group_hidden(g, true).unwrap();
    let s = scene(&e);
    assert!(s.layers.is_empty());
    assert!(s.lines.len() < base.lines.len() && s.fills.len() < base.fills.len());
    for text in ["东海", "某战"] {
        assert!(!s.labels.iter().any(|l| l.text.split_whitespace().collect::<String>() == text), "{text}");
    }
    assert!(!legend(&s).contains(&"商路".to_string()) && !legend(&s).contains(&"战役".to_string()));
    // shown again, it is all back
    e.set_group_hidden(g, false).unwrap();
    let back = scene(&e);
    assert_eq!(back.labels.len(), base.labels.len());
    assert_eq!(legend(&back), legend(&base));
    // layers are drawn in the order of the stack
    let h = e.add_group("上层");
    e.set_group(&fs[2..3], Some(h)).unwrap();
    let names = |s: &Scene| s.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&scene(&e)), ["战争", "上层"]);
    e.move_group(h, 0).unwrap();
    assert_eq!(names(&scene(&e)), ["上层", "战争"]);
}
