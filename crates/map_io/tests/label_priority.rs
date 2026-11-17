//! Which names win where they meet in the export: free text, symbols and
//! country names come before the names of rivers and routes, which slide
//! along their lines to keep clear (or are left out). A world made by a
//! model, with a named river through the middle of most countries, once
//! showed two of its eight country names.

use map_core::doc::{Document, LineFeature, LineId};
use map_io::scene::{build_scene, ExportOptions, Scene};

fn scene(d: &Document) -> Scene {
    build_scene(d, &map_core::theme::evaluate(d), &ExportOptions { width: 1600, height: 850, legend: false, ..Default::default() })
}

fn texts(s: &Scene) -> Vec<(String, i64, i64)> {
    let mut v: Vec<(String, i64, i64)> = s.labels.iter().map(|l| (l.text.clone(), l.pos[0].round() as i64, l.pos[1].round() as i64)).collect();
    v.sort();
    v
}

#[test]
fn river_names_do_not_push_out_country_names() {
    let mut d = map_core::sample::demo_world();
    d.style.show_cities = false;
    let base = scene(&d);
    assert!(!base.labels.is_empty());
    // a named river along the very line each country's name follows
    let mut lines = 0;
    for id in d.countries() {
        let Some(path) = d.label_path(id) else { continue };
        let rid = LineId(d.topo.alloc());
        d.rivers.insert(rid, LineFeature { name: "大河".into(), pts: path.line(32), route: None, legend: String::new(), show_name: true, hidden: false });
        lines += 1;
    }
    assert!(lines >= 3, "{lines}");
    let with = scene(&d);
    // every name shown before is still shown, where it was
    let (a, b) = (texts(&base), texts(&with));
    for t in &a {
        assert!(b.contains(t), "{t:?} went missing");
    }
    // and some river names found room further along their rivers
    let river_glyphs = b.len() - a.len();
    assert!(river_glyphs > 0 && river_glyphs % 2 == 0, "{river_glyphs}");
}

#[test]
fn a_country_name_moves_off_a_symbol_rather_than_vanish() {
    // a symbol (which always shows) in the middle of a country: the name
    // slides along its line or off it, or is set smaller
    let mut d = map_core::sample::demo_world();
    d.style.show_cities = false;
    let base = scene(&d);
    let shown = |s: &Scene, name: &str| {
        let all: String = s.labels.iter().map(|l| l.text.split_whitespace().collect::<String>()).collect();
        all.contains(name)
    };
    let mut tried = 0;
    for id in d.countries() {
        let name = d.entities[&id].name.trim().to_string();
        let Some(path) = d.label_path(id) else { continue };
        // (not on an island smaller than the battle's symbol)
        if !shown(&base, &name) || path.length() < 15.0 {
            continue;
        }
        // a battle at the very middle of the name's line
        let mut e = d.clone();
        let sid = LineId(e.topo.alloc());
        let mid = path.point(path.length() * 0.5, 0.0);
        e.symbols.insert(sid, map_core::symbol::MapSymbol::new(map_core::symbol::SymbolKind::Battle, mid));
        assert!(shown(&scene(&e), &name), "{name} went missing beside a battle");
        tried += 1;
    }
    assert!(tried >= 5, "{tried}");
}
