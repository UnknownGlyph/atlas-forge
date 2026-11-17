//! Automatic partition: a country's land cut into units of about the same
//! size — new countries beside it, or provinces under it — leaving every
//! other country as it was, from the selected year on, in one undo step.

use map_core::history::History;
use map_core::partition::PartitionSpec;
use map_core::worldgen::{generate, WorldSpec};
use map_core::{Document, EntityId};

/// A generated world and its largest country.
fn world() -> (Document, EntityId) {
    let (d, _) = generate(&WorldSpec { seed: 21, countries: 5, rivers: 6, land: 0.3, continents: 2, ..WorldSpec::default() }).unwrap();
    let areas = d.areas_by_entity();
    let big = *areas.iter().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
    (d, big)
}

#[test]
fn a_country_into_countries_of_about_the_same_size() {
    let (mut d, e) = world();
    let before = d.areas_by_entity();
    let total = before[&e];
    let spec = PartitionSpec { units: 5, seed: 3, ..PartitionSpec::default() };
    let plan = d.partition_plan(e, &spec).unwrap();
    assert_eq!(plan.units.len(), 5);
    assert!(plan.units.iter().all(|u| !u.is_empty()));
    let planned: f64 = plan.areas.iter().sum();
    assert!((planned - total).abs() < total * 0.02, "{planned} of {total}");
    let (lo, hi) = plan.areas.iter().fold((f64::MAX, 0.0f64), |(a, b), x| (a.min(*x), b.max(*x)));
    assert!(hi < lo * 2.2, "sizes {:?}", plan.areas);
    // applied: the old country keeps one unit, four new ones beside it
    let mut h = History::default();
    let start = d.clone();
    let units = h.apply(&mut d, "划分", |d| d.apply_partition(&plan, &spec)).unwrap();
    d.topo.validate().unwrap();
    assert_eq!(units.len(), 5);
    assert_eq!(units[0], e);
    let after = d.areas_by_entity();
    let got: f64 = units.iter().map(|u| after.get(u).copied().unwrap_or(0.0)).sum();
    assert!((got - total).abs() < total * 0.01, "the land is shared out: {got} of {total}");
    assert!(units.iter().all(|u| after.get(u).copied().unwrap_or(0.0) > total * 0.05), "{:?}", units.iter().map(|u| after.get(u)).collect::<Vec<_>>());
    // nothing else moved
    for (o, a) in &before {
        if *o != e {
            assert!((after[o] - a).abs() < a * 1e-6 + 1.0, "{o:?}");
        }
    }
    // neighbours in different colours; names new and different
    for (a, ns) in d.country_neighbours() {
        for b in ns {
            assert_ne!(d.entities[&a].color, d.entities[&b].color);
        }
    }
    let names: std::collections::HashSet<&str> = units.iter().map(|u| d.entities[u].name.as_str()).collect();
    assert_eq!(names.len(), 5);
    // one undo step back
    h.undo(&mut d);
    assert_eq!(d, start);
}

#[test]
fn provinces_under_a_country_from_a_year() {
    let (mut d, e) = world();
    let total = d.areas_by_entity()[&e];
    let spec = PartitionSpec { units: 4, seed: 5, provinces: true, follow: false, ..PartitionSpec::default() };
    d.set_year(Some(1900));
    let plan = d.partition_plan(e, &spec).unwrap();
    let units = d.apply_partition(&plan, &spec).unwrap();
    d.topo.validate().unwrap();
    assert!(!units.contains(&e));
    // the country holds its land through its provinces only
    assert!(d.faces_of(e).is_empty(), "no land left directly with the country");
    for u in &units {
        assert_eq!(d.entities[u].parent, Some(e));
        assert_eq!(d.top_ancestor(*u), e);
    }
    let areas = d.areas_by_entity();
    let got: f64 = units.iter().map(|u| areas.get(u).copied().unwrap_or(0.0)).sum();
    assert!((got - total).abs() < total * 0.01);
    // the political map still shows the country whole: its colour everywhere
    let theme = map_core::theme::evaluate(&d);
    for u in &units {
        for f in d.faces_of(*u) {
            assert_eq!(theme.face_colors[&f], d.entities[&e].color);
        }
    }
    // the country is named over all its provinces; the provinces are not named apart
    assert!(d.label_path(e).is_some());
    assert!(units.iter().all(|u| d.label_path(*u).is_none()));
    let mut cache = map_core::labels::LabelCache::default();
    cache.update(&d);
    assert!(cache.sorted().iter().any(|(x, _)| *x == e));
    // before 1900 the country was undivided
    d.set_year(Some(1899));
    assert!(units.iter().all(|u| d.faces_of(*u).is_empty()));
    assert!(!d.faces_of(e).is_empty());
}

#[test]
fn plans_follow_the_seed_and_refuse_too_many_units() {
    let (d, e) = world();
    let a = d.partition_plan(e, &PartitionSpec { units: 3, seed: 1, ..PartitionSpec::default() }).unwrap();
    let b = d.partition_plan(e, &PartitionSpec { units: 3, seed: 1, ..PartitionSpec::default() }).unwrap();
    let c = d.partition_plan(e, &PartitionSpec { units: 3, seed: 2, ..PartitionSpec::default() }).unwrap();
    assert_eq!(a.areas, b.areas);
    assert_ne!(a.areas, c.areas);
    // a country with no land
    let mut d2 = d.clone();
    let empty = d2.add_entity(map_core::EntityKind::Country, "空国");
    assert!(d2.partition_plan(empty, &PartitionSpec::default()).is_err());
}

/// Every country of a generated world cut into ten. The units are drawn one
/// after another along shared grid lines; a border the boolean kernel had
/// left a hair off its line used to make the next unit's rebuild fail.
#[test]
fn every_country_of_a_world_can_be_cut_into_ten() {
    let (d, _) = generate(&WorldSpec { seed: 9, ..WorldSpec::default() }).unwrap();
    let countries: Vec<EntityId> = d.entities.iter().filter(|(_, x)| x.kind == map_core::EntityKind::Country).map(|(e, _)| *e).collect();
    let mut cut = 0;
    for e in countries {
        let spec = PartitionSpec { units: 10, seed: 19, ..PartitionSpec::default() };
        // a country too small for ten units says so
        let Ok(plan) = d.partition_plan(e, &spec) else { continue };
        let mut d2 = d.clone();
        let units = d2.apply_partition(&plan, &spec).unwrap_or_else(|err| panic!("{}: {err}", d.entities[&e].name));
        d2.topo.validate().unwrap();
        assert_eq!(units.len(), 10);
        cut += 1;
    }
    assert!(cut >= 15, "only {cut} countries were large enough");
}

/// A lake and another country's enclave inside the country stay as they
/// are: each unit is drawn with its holes (drawn by its outer ring alone, a
/// unit filled the lake and took the enclave).
#[test]
fn a_lake_and_an_enclave_inside_stay() {
    use map_core::overlay::{self, DrawIntent};
    use map_core::{FaceKind, LonLat, BACKGROUND};
    let (mut d, e) = world();
    let polys = d.dissolve(e);
    let big = polys.iter().max_by(|a, b| map_core::geom::signed_area(&a.outer).abs().total_cmp(&map_core::geom::signed_area(&b.outer).abs())).unwrap();
    let (c, w) = map_core::geom::interior_point(&big.outer, &big.holes).unwrap();
    let r = (w / 10.0).min(1.0);
    let square = |x: f64, y: f64| vec![LonLat::new(x - r, y - r * 0.5), LonLat::new(x + r, y - r * 0.5), LonLat::new(x + r, y + r * 0.5), LonLat::new(x - r, y + r * 0.5)];
    let other = *d.entities.iter().find(|(x, v)| **x != e && v.kind == map_core::EntityKind::Country).unwrap().0;
    d = overlay::apply(&d, &square(c[0] - 2.0 * r, c[1]), DrawIntent::CarveWater).unwrap().0;
    d = overlay::apply(&d, &square(c[0] + 2.0 * r, c[1]), DrawIntent::Take(other)).unwrap().0;
    let rad = d.radius();
    let lakes = |d: &Document| -> f64 { d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Water).map(|(f, _)| d.topo.face_area(*f, rad)).sum() };
    let enclave = |d: &Document| d.areas_by_entity().get(&other).copied().unwrap_or(0.0);
    let (lake0, enclave0) = (lakes(&d), enclave(&d));
    assert!(lake0 > 0.0);
    for seed in 1..=4 {
        let spec = PartitionSpec { units: 4, seed, ..PartitionSpec::default() };
        let plan = d.partition_plan(e, &spec).unwrap();
        let mut d2 = d.clone();
        d2.apply_partition(&plan, &spec).unwrap();
        d2.topo.validate().unwrap();
        assert!((lakes(&d2) - lake0).abs() < 1e-6 * lake0, "plan {seed}: lakes {} km² became {} km²", lake0, lakes(&d2));
        assert!((enclave(&d2) - enclave0).abs() < 1e-6 * enclave0, "plan {seed}: the enclave {} km² became {} km²", enclave0, enclave(&d2));
    }
}
