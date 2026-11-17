//! Named styles: features linked to a style look like it, a change of the
//! style changes them all, an edit of one linked feature changes its style,
//! and features without a style keep their own look as before.

use map_core::area::{AreaMark, Pattern};
use map_core::doc::{Annotation, Dash, Document, LineFeature, LineId, RouteStyle};
use map_core::features::FeatureRef;
use map_core::history::History;
use map_core::styles::{Look, StyleKind, TextLook};
use map_core::symbol::{MapSymbol, SymbolKind};
use map_core::LonLat;

fn p(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

/// Three routes, a river, two areas, a text and a symbol.
fn world() -> (Document, Vec<FeatureRef>) {
    let mut d = Document::default();
    let mut fs = Vec::new();
    for i in 0..3 {
        let id = LineId(d.topo.alloc());
        d.rivers.insert(id, LineFeature { name: format!("路{i}"), pts: vec![p(i as f64 * 5.0, 0.0), p(i as f64 * 5.0 + 3.0, 2.0)], route: Some(RouteStyle::default()), legend: String::new(), show_name: false, hidden: false });
        fs.push(FeatureRef::Route(id));
    }
    let river = LineId(d.topo.alloc());
    d.rivers.insert(river, LineFeature { name: "河".into(), pts: vec![p(0.0, 10.0), p(5.0, 12.0)], route: None, legend: String::new(), show_name: false, hidden: false });
    fs.push(FeatureRef::River(river));
    for i in 0..2 {
        let id = LineId(d.topo.alloc());
        let x = 20.0 + i as f64 * 10.0;
        d.areas.insert(id, AreaMark::new(vec![p(x, 0.0), p(x + 5.0, 0.0), p(x + 5.0, 5.0)]));
        fs.push(FeatureRef::Area(id));
    }
    let t = LineId(d.topo.alloc());
    d.annotations.insert(t, Annotation::new("海", p(0.0, -10.0)));
    fs.push(FeatureRef::Text(t));
    let s = LineId(d.topo.alloc());
    d.symbols.insert(s, MapSymbol::new(SymbolKind::Port, p(10.0, -10.0)));
    fs.push(FeatureRef::Symbol(s));
    (d, fs)
}

/// Every linked feature looks like its style.
fn consistent(d: &Document) {
    for (id, s) in &d.style_of {
        let st = d.style(*s).expect("a link to a style that is gone");
        let f = d.features().into_iter().map(|f| f.what).find(|f| f.id() == *id).expect("a link from a feature that is gone");
        assert_eq!(d.feature_look(f), Some(st.look.clone()), "{f:?} does not look like {}", st.name);
    }
}

#[test]
fn a_style_changes_every_feature_linked_to_it() {
    let (mut d, fs) = world();
    let red = RouteStyle { color: [200, 0, 0, 255], dash: Dash::Solid, ..RouteStyle::default() };
    let s = d.add_style("进军路线", Look::Route(red));
    // linked: the routes take the look; other kinds are left alone
    assert_eq!(d.link_style(&fs, s).unwrap(), 3);
    assert!(fs[..3].iter().all(|f| d.feature_look(*f) == Some(Look::Route(red)) && d.style_of_feature(*f) == Some(s)));
    assert!(d.style_of_feature(fs[3]).is_none() && d.style_of_feature(fs[4]).is_none());
    assert!(d.link_style(&fs[3..4], s).is_err(), "a river takes no route style");
    // the style changed: all three change
    let blue = RouteStyle { color: [0, 0, 200, 255], width: 4.0, ..red };
    assert_eq!(d.update_style(s, Look::Route(blue)).unwrap(), 3);
    assert!(fs[..3].iter().all(|f| d.feature_look(*f) == Some(Look::Route(blue))));
    assert!(d.update_style(s, Look::Text(TextLook { size: 1.0, color: [0; 4], spacing: 0.0, italic: false, font: None, bold: false })).is_err());
    // one unlinked keeps its look and no longer follows
    d.unlink_style(&fs[2..3]);
    d.update_style(s, Look::Route(red)).unwrap();
    assert_eq!((d.feature_look(fs[2]), d.feature_look(fs[0])), (Some(Look::Route(blue)), Some(Look::Route(red))));
    assert_eq!(d.style_users(s), fs[..2].to_vec());
    consistent(&d);
    // removed: the features keep the look, unlinked
    assert_eq!(d.remove_style(s).unwrap(), 2);
    assert!(d.style_of.is_empty() && d.feature_look(fs[0]) == Some(Look::Route(red)));
}

#[test]
fn styles_from_features_and_the_eyedropper() {
    let (mut d, fs) = world();
    let (a, b, t, sym) = (fs[4], fs[5], fs[6], fs[7]);
    // an area restyled by hand becomes a named style
    {
        let x = d.areas.get_mut(&a.id()).unwrap();
        (x.color, x.pattern, x.tint) = ([0, 120, 0, 255], Pattern::Dots, true);
    }
    let s = d.style_from_feature(a, "势力范围").unwrap();
    assert_eq!(d.style(s).unwrap().look.kind(), StyleKind::Area);
    assert!(d.style_from_feature(fs[3], "").is_err(), "a river has no look of its own");
    // the eyedropper: the other area takes the look and the link; others nothing
    assert_eq!(d.copy_look(a, &[b, t, sym]).unwrap(), 1);
    assert_eq!((d.feature_look(b), d.style_of_feature(b)), (d.feature_look(a), Some(s)));
    // from an unlinked feature: the look, and no link
    let t2 = d.duplicate_annotation(t.id(), 1.0, 1.0).map(FeatureRef::Text).unwrap();
    d.annotations.get_mut(&t.id()).unwrap().size = 40.0;
    assert_eq!(d.copy_look(t, &[t2]).unwrap(), 1);
    assert!(d.style_of_feature(t2).is_none() && d.annotations[&t2.id()].size == 40.0);
    // a copy follows its original's style; a style redefined from a feature
    let ss = d.style_from_feature(sym, "港口").unwrap();
    let copy = d.duplicate_symbol(sym.id(), 1.0, 1.0).map(FeatureRef::Symbol).unwrap();
    assert_eq!(d.style_of_feature(copy), Some(ss));
    d.symbols.get_mut(&copy.id()).unwrap().size = 12.0;
    d.redefine_style(ss, copy).unwrap();
    assert_eq!(d.symbols[&sym.id()].size, 12.0, "the original follows the redefined style");
    consistent(&d);
}

#[test]
fn an_edit_of_a_linked_feature_changes_its_style() {
    let (mut d, fs) = world();
    let s = d.add_style("", Look::Route(RouteStyle::default()));
    assert!(d.style(s).unwrap().name.starts_with("线路样式"));
    d.link_style(&fs[..3], s).unwrap();
    // the second route's colour changed directly (as its panel does)
    let r1 = fs[1].id();
    d.rivers.get_mut(&r1).unwrap().route.as_mut().unwrap().color = [1, 2, 3, 255];
    assert_eq!(d.settle_styles(), vec![s]);
    assert!(fs[..3].iter().all(|f| matches!(d.feature_look(*f), Some(Look::Route(r)) if r.color == [1, 2, 3, 255])));
    consistent(&d);
    assert!(d.settle_styles().is_empty(), "nothing more to carry over");
    // a route made a river leaves its style
    d.rivers.get_mut(&r1).unwrap().route = None;
    d.settle_styles();
    assert_eq!(d.style_users(s).len(), 2);
    // deleted features leave it too
    d.delete_features(&fs[..1]);
    assert_eq!(d.style_users(s).len(), 1);
    consistent(&d);
}

#[test]
fn copies_undo_saving_and_older_files() {
    let (mut d, fs) = world();
    let s = d.add_style("航线", Look::Route(RouteStyle { color: [9, 9, 9, 255], ..RouteStyle::default() }));
    d.link_style(&fs[..3], s).unwrap();
    // copied and pasted in the same document: linked
    let clip = d.copy_features(&fs[..2]);
    let new = d.paste_features(&clip, Some(p(60.0, 30.0))).unwrap();
    assert!(new.iter().all(|f| d.style_of_feature(*f) == Some(s)));
    // pasted into another document: the look, no link (it has no such style)
    let mut other = Document::default();
    let there = other.paste_features(&clip, None).unwrap();
    assert!(other.style_of.is_empty() && other.feature_look(there[0]) == d.feature_look(fs[0]));
    // nor when the style there looks different
    let mut changed = d.clone();
    changed.update_style(s, Look::Route(RouteStyle::default())).unwrap();
    let pasted = changed.paste_features(&clip, None).unwrap();
    assert!(changed.style_of_feature(pasted[0]).is_none());
    consistent(&changed);
    // undo puts the style and the looks back
    let mut h = History::default();
    let before = d.clone();
    h.apply(&mut d, "style", |d| d.update_style(s, Look::Route(RouteStyle::default())).map(|_| ())).unwrap();
    h.undo(&mut d);
    assert_eq!(d, before);
    // saved and read back
    d.add_swatch("海军蓝", [20, 40, 90, 255]);
    let back: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!((back.styles.len(), back.style_of.len(), back.swatches.len()), (d.styles.len(), d.style_of.len(), 1));
    // an older file: no styles, no links, every feature its own look
    let mut v = serde_json::to_value(&d).unwrap();
    for k in ["styles", "style_of", "swatches"] {
        v.as_object_mut().unwrap().remove(k);
    }
    let old: Document = serde_json::from_value(v).unwrap();
    assert!(old.styles.is_empty() && old.style_of.is_empty() && old.feature_look(fs[0]) == d.feature_look(fs[0]));
}

#[test]
fn presets_swatches_and_order() {
    let (mut d, _) = world();
    let n = d.add_preset_styles();
    assert!(n >= 10);
    assert_eq!(d.add_preset_styles(), 0, "not added twice");
    for k in [StyleKind::Route, StyleKind::Area, StyleKind::Text] {
        assert!(d.styles.iter().any(|s| s.look.kind() == k), "{}", k.name());
    }
    let first = d.styles[0].id;
    d.move_style(first, 99).unwrap();
    assert_eq!(d.styles.last().unwrap().id, first);
    let copy = d.duplicate_style(first).unwrap();
    assert!(d.style(copy).unwrap().name.ends_with("副本") && d.style_users(copy).is_empty());
    d.rename_style(copy, " 新名字 ").unwrap();
    assert_eq!(d.style(copy).unwrap().name, "新名字");
    assert!(d.rename_style(copy, "").is_err());
    // swatches: a colour once, removable
    assert_eq!(d.add_swatch("", [1, 2, 3, 255]), 0);
    assert_eq!(d.add_swatch("又一次", [1, 2, 3, 255]), 0);
    assert_eq!(d.swatches[0].name, "颜色 1");
    d.remove_swatch(0).unwrap();
    assert!(d.remove_swatch(0).is_err());
    // colours the map uses, most used first
    let used = d.colors_in_use();
    assert!(!used.is_empty() && used.windows(2).all(|w| w[0].1 >= w[1].1));
}
