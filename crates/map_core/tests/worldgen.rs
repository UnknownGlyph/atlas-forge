//! The new-world wizard: a valid map with about the land asked for,
//! countries that cover it with different colours from their neighbours,
//! rivers that run downhill to the sea, capitals inside their countries,
//! the same world from the same seed — and unclaimed land when no
//! countries are asked for.

use map_core::worldgen::{generate, WorldSpec};
use map_core::{EntityKind, FaceKind, BACKGROUND};
use std::collections::HashSet;

fn spec(seed: u64) -> WorldSpec {
    WorldSpec { seed, countries: 14, rivers: 8, ..WorldSpec::default() }
}

#[test]
fn a_world_with_countries_rivers_and_capitals() {
    let (d, rep) = generate(&spec(3)).unwrap();
    eprintln!("{rep:?}");
    d.topo.validate().expect("a valid topology");
    assert!((rep.land - 0.32).abs() < 0.07, "land {:.3}", rep.land);
    assert!(rep.landmasses >= 1 && rep.faces >= rep.countries);
    // countries hold all the land; names are different
    let countries: Vec<_> = d.countries();
    assert_eq!(countries.len(), rep.countries);
    assert!(rep.countries >= 8 && rep.countries <= 14, "{}", rep.countries);
    let names: HashSet<&str> = countries.iter().map(|e| d.entities[e].name.as_str()).collect();
    assert_eq!(names.len(), countries.len());
    for (f, x) in d.topo.faces.iter() {
        if *f != BACKGROUND && x.kind == FaceKind::Land {
            assert!(d.owner.contains_key(f), "land with no country");
        }
    }
    assert!(countries.iter().all(|e| !d.faces_of(*e).is_empty()));
    // neighbours in different colours
    for (a, ns) in d.country_neighbours() {
        for b in ns {
            assert_ne!(d.entities[&a].color, d.entities[&b].color);
        }
    }
    // capitals inside their countries
    let cities: Vec<_> = d.entities.iter().filter(|(_, x)| x.kind == EntityKind::City).collect();
    assert_eq!(cities.len(), rep.cities);
    assert!(rep.cities >= rep.countries / 2);
    for (c, x) in &cities {
        let at = x.pos.unwrap();
        let owner = d.owner.get(&d.topo.face_at(at)).copied();
        assert!(owner.is_some_and(|o| d.entities[&o].capital == Some(**c)), "{} is not in its country", x.name);
    }
    // rivers: named, running downhill, ending by the sea
    let t = d.terrain.as_ref().expect("the elevation is kept");
    assert_eq!((t.w, t.h), (720, 360));
    assert!(rep.rivers >= 3, "{} rivers", rep.rivers);
    for r in d.rivers.values() {
        assert!(!r.name.is_empty() && r.pts.len() >= 4);
        let (src, mouth) = (r.pts[0], *r.pts.last().unwrap());
        assert!(t.at(src.lon, src.lat) > t.at(mouth.lon, mouth.lat), "{} runs uphill", r.name);
        assert!(t.at(mouth.lon, mouth.lat) < 400.0, "{} ends far from the sea", r.name);
    }
    // no land at the map's edge or round a pole
    for e in d.topo.edges.values() {
        for p in d.topo.edge_full(e) {
            assert!(p.lon.abs() < 179.0 && p.lat.abs() < 85.0, "{p:?}");
        }
    }
}

#[test]
fn the_same_seed_the_same_world() {
    // face revisions are cache keys unique within the process (two worlds
    // made one after the other never share them), not content: set aside
    let without_revs = |mut d: map_core::Document| {
        let ids: Vec<_> = d.topo.faces.keys().copied().collect();
        for f in ids {
            let mut face = d.topo.faces[&f].clone();
            face.rev = 0;
            d.topo.faces.insert(f, face);
        }
        d
    };
    let a = without_revs(generate(&spec(9)).unwrap().0);
    let b = without_revs(generate(&spec(9)).unwrap().0);
    assert_eq!(a.topo.edges.len(), b.topo.edges.len());
    // the very same: every point of every edge, every face, name and colour
    assert!(a == b, "the same seed made two different worlds");
    let names = |d: &map_core::Document| d.entities.values().map(|e| e.name.clone()).collect::<Vec<_>>();
    assert_eq!(names(&a), names(&b));
    let c = generate(&spec(10)).unwrap().0;
    assert_ne!(names(&a), names(&c));
}

#[test]
fn land_without_countries() {
    let (d, rep) = generate(&WorldSpec { countries: 0, rivers: 0, land: 0.25, continents: 2, seed: 4, ..WorldSpec::default() }).unwrap();
    d.topo.validate().unwrap();
    assert_eq!(rep.countries, 0);
    assert!(d.countries().is_empty() && d.owner.is_empty() && d.rivers.is_empty());
    assert!(d.topo.faces.values().filter(|f| f.kind == FaceKind::Land).count() >= rep.landmasses);
}

#[test]
fn a_world_with_a_history() {
    let s = WorldSpec { history: Some((1000, 1400, 90)), ..spec(5) };
    let (mut d, rep) = generate(&s).unwrap();
    d.topo.validate().unwrap();
    assert!(rep.changes >= 30, "{} changes", rep.changes);
    let years = d.change_years();
    assert!(years.iter().all(|y| (1000..=1401).contains(y)) && years.len() >= 20, "{years:?}");
    // the map before the history has the countries asked for
    d.set_year(Some(999));
    let before = d.countries().into_iter().filter(|e| d.exists_in(*e, Some(999)) && !d.faces_of(*e).is_empty()).count();
    assert_eq!(before, rep.countries);
    // neighbours in any year differ in colour, names stay different
    d.set_year(None);
    for (a, ns) in d.country_neighbours() {
        for b in ns {
            assert_ne!(d.entities[&a].color, d.entities[&b].color);
        }
    }
    let names: Vec<&str> = d.countries().iter().map(|e| d.entities[e].name.as_str()).collect();
    let uniq: HashSet<&str> = names.iter().copied().collect();
    assert_eq!(uniq.len(), names.len(), "{names:?}");
}
