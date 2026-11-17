//! Ownership through time (ADR-7): an edit made in a later year never
//! changes the map of an earlier year.

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::sample::{grid_world, Rng};
use map_core::topo::{FaceId, FaceKind};
use map_core::{EntityId, EntityKind};
use std::collections::HashMap;

/// Area per entity in a given year, rounded to whole km² so maps rebuilt
/// with new face ids still compare equal.
fn map_in(d: &Document, year: Option<i32>) -> HashMap<EntityId, i64> {
    let mut v = d.clone();
    v.set_year(year);
    v.areas_by_entity().into_iter().map(|(e, a)| (e, a.round() as i64)).collect()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<LonLat> {
    vec![LonLat::new(x0, y0), LonLat::new(x1, y0), LonLat::new(x1, y1), LonLat::new(x0, y1)]
}

fn one_face() -> (Document, FaceId, [EntityId; 3]) {
    let mut d = Document::default();
    let f = d.add_ring_face(&rect(0.0, 0.0, 10.0, 10.0), FaceKind::Land).unwrap();
    let e = [d.add_entity(EntityKind::Country, "A"), d.add_entity(EntityKind::Country, "B"), d.add_entity(EntityKind::Country, "C")];
    d.assign(f, Some(e[0])).unwrap();
    (d, f, e)
}

#[test]
fn change_points_apply_from_their_year_until_the_next() {
    let (mut d, f, [a, b, c]) = one_face();
    d.set_year(Some(1900));
    d.assign(f, Some(b)).unwrap();
    d.set_year(Some(1950));
    d.assign(f, Some(c)).unwrap();
    d.set_year(Some(1920));
    d.assign(f, Some(a)).unwrap();
    let at = |y| d.owner_at(f, Some(y));
    assert_eq!((at(1899), at(1900), at(1919), at(1920), at(1949), at(1950)), (Some(a), Some(b), Some(b), Some(a), Some(a), Some(c)));
    assert_eq!(d.owner_at(f, None), Some(c), "no year = latest state");
    assert_eq!(d.change_years().into_iter().collect::<Vec<_>>(), vec![1900, 1920, 1950]);
    // undoing a change by assigning the owner already in force removes the point
    d.assign(f, Some(b)).unwrap();
    assert_eq!(d.face_history(f).1, vec![(1900, Some(b)), (1950, Some(c))]);
    // the derived view follows the selected year
    d.set_year(Some(1960));
    assert_eq!(d.owner.get(&f), Some(&c));
    d.set_year(Some(1800));
    assert_eq!(d.owner.get(&f), Some(&a));
    assert_eq!(d.entity_change_years(a).into_iter().collect::<Vec<_>>(), vec![1900]);
}

#[test]
fn editing_all_years_is_refused_once_a_face_has_history() {
    let (mut d, f, [_, b, c]) = one_face();
    // no history yet: an all-years edit is fine
    d.assign(f, Some(c)).unwrap();
    d.set_year(Some(1900));
    d.assign(f, Some(b)).unwrap();
    d.set_year(None);
    let before = d.clone();
    assert!(d.assign(f, None).is_err());
    assert_eq!(d, before);
}

#[test]
fn split_shares_history_and_merge_needs_equal_histories() {
    let (mut d, f, [a, b, _]) = one_face();
    d.set_year(Some(1900));
    d.assign(f, Some(b)).unwrap();
    let (x, y) = d.split_face(&[LonLat::new(5.0, 0.0), LonLat::new(5.0, 10.0)]).unwrap();
    assert!(d.same_history(x, y));
    // the cession: only y changes hands, from 1950
    d.set_year(Some(1950));
    d.assign(y, Some(a)).unwrap();
    assert!(!d.same_history(x, y));
    let before = d.clone();
    assert!(d.merge_faces(x, y).is_err(), "merging would rewrite 1950+");
    assert_eq!(d, before);
    d.assign(y, Some(b)).unwrap();
    d.merge_faces(x, y).unwrap();
    d.topo.validate().unwrap();
}

#[test]
fn entities_merged_or_deleted_in_every_year() {
    let (mut d, f, [a, b, c]) = one_face();
    d.set_year(Some(1900));
    d.assign(f, Some(b)).unwrap();
    d.set_year(Some(1950));
    d.assign(f, Some(c)).unwrap();
    d.merge_entities(b, a).unwrap();
    assert_eq!(d.face_history(f), (Some(a), vec![(1950, Some(c))]), "B's years became A's and the redundant point went away");
    d.delete_entity(c);
    assert_eq!(d.face_history(f), (Some(a), vec![(1950, None)]));
    assert_eq!(d.owner.get(&f), None, "selected year 1950: unclaimed now");
}

/// Random edits at random later years (brush assignments, splits, drawn
/// takes and carves through both overlay paths); after each, every year
/// before the edit year has exactly the map it had.
#[test]
fn later_edits_never_change_earlier_years() {
    let mut d = grid_world(12, 8, 3, 7);
    let countries = d.countries();
    let mut rng = Rng::new(5);
    let probe_years = [1700, 1850, 1900, 1925, 1950, 1975, 2000];
    let mut ok = 0;
    for step in 0..60 {
        let year = 1800 + (rng.next_f64() * 200.0) as i32;
        let before: Vec<_> = probe_years.iter().filter(|y| **y < year).map(|y| (*y, map_in(&d, Some(*y)))).collect();
        d.set_year(Some(year));
        let x = countries[(rng.next_f64() * countries.len() as f64) as usize];
        let result = match step % 3 {
            0 => {
                let faces: Vec<FaceId> = d.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| *id).collect();
                let f = faces[(rng.next_f64() * faces.len() as f64) as usize];
                d.assign(f, Some(x)).map(|_| d.clone())
            }
            _ => {
                let (cx, cy) = (rng.range(-150.0, 140.0), rng.range(-50.0, 50.0));
                let ring = rect(cx, cy, cx + rng.range(3.0, 20.0), cy + rng.range(3.0, 15.0));
                let intent = if step % 3 == 1 { DrawIntent::Take(x) } else { DrawIntent::Expand(x) };
                overlay::apply(&d, &ring, intent).map(|r| r.0)
            }
        };
        let Ok(n) = result else { continue };
        n.topo.validate().unwrap_or_else(|e| panic!("step {step}: {e}"));
        for (y, m) in &before {
            assert_eq!(&map_in(&n, Some(*y)), m, "step {step}: editing {year} changed {y}");
        }
        d = n;
        ok += 1;
    }
    assert!(ok >= 30, "only {ok} edits succeeded");
    assert!(d.change_years().len() >= 10);
}

#[test]
fn history_survives_save_and_old_files_load() {
    let (mut d, f, [_, b, _]) = one_face();
    d.set_year(Some(1900));
    d.assign(f, Some(b)).unwrap();
    d.set_year(Some(1850));
    let json = serde_json::to_string(&d).unwrap();
    let mut back: Document = serde_json::from_str(&json).unwrap();
    back.topo.rebuild_index();
    back.refresh_owner();
    assert_eq!(back.face_history(f), d.face_history(f));
    assert_eq!(back.owner, d.owner);
    // a file from before M3 has only "owner" (the base)
    let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
    v.as_object_mut().unwrap().remove("owner_changes");
    let mut old: Document = serde_json::from_value(v).unwrap();
    old.refresh_owner();
    assert_eq!(old.face_history(f).1, vec![]);
    assert!(old.owner.contains_key(&f));
}

/// Top-level country of every face in a year: what the political map shows.
fn top_map(d: &Document, year: Option<i32>) -> Vec<(FaceId, Option<EntityId>)> {
    let mut v = d.clone();
    v.set_year(year);
    v.topo.faces.keys().map(|f| (*f, v.owner.get(f).map(|e| v.top_ancestor(*e)))).collect()
}

fn region_world() -> (Document, FaceId, [EntityId; 3]) {
    let (mut d, f, [a, b, _]) = one_face();
    let r = d.add_entity(EntityKind::Region, "R");
    d.assign(f, Some(r)).unwrap();
    d.record_parent(r, Some(a)).unwrap();
    (d, f, [a, b, r])
}

#[test]
fn a_region_changes_country_from_a_year() {
    let (mut d, f, [a, b, r]) = region_world();
    d.set_year(Some(1900));
    d.record_parent(r, Some(b)).unwrap();
    let top = |d: &Document, y| top_map(d, Some(y)).into_iter().find(|x| x.0 == f).unwrap().1;
    assert_eq!((top(&d, 1899), top(&d, 1900), top(&d, 2000)), (Some(a), Some(b), Some(b)));
    assert_eq!(d.owner.get(&f), Some(&r), "the face itself stays with the region");
    assert_eq!(d.parent_history(r), (Some(a), vec![(1900, Some(b))]));
    assert!(d.entity_change_years(a).contains(&1900) && d.entity_change_years(b).contains(&1900));
    assert_eq!(d.change_years().into_iter().collect::<Vec<_>>(), vec![1900]);
    d.set_year(None);
    let before = d.clone();
    assert!(d.record_parent(r, None).is_err(), "all-years edit of a history");
    assert_eq!(d, before);
}

#[test]
fn cycles_are_refused_in_every_later_year() {
    let (mut d, _, [a, b, _]) = region_world();
    d.set_year(Some(1900));
    d.record_parent(b, Some(a)).unwrap();
    // A under B from 1950: B is still under A then
    d.set_year(Some(1950));
    let before = d.clone();
    assert!(d.record_parent(a, Some(b)).is_err());
    assert_eq!(d, before);
    // A under B from 1850 is fine in 1850 but cycles once B joins A in 1900
    d.set_year(Some(1850));
    assert!(d.record_parent(a, Some(b)).is_err());
    // after B leaves A in 1950, A may join B from 1960
    d.set_year(Some(1950));
    d.record_parent(b, None).unwrap();
    d.set_year(Some(1960));
    d.record_parent(a, Some(b)).unwrap();
    assert_eq!(d.top_ancestor(a), b);
}

#[test]
fn merging_and_deleting_rewrite_parents_in_every_year() {
    let (mut d, _, [a, b, r]) = region_world();
    d.set_year(Some(1900));
    d.record_parent(r, Some(b)).unwrap();
    d.merge_entities(b, a).unwrap();
    assert_eq!(d.parent_history(r), (Some(a), vec![]), "B's years became A's, the change went away");
    d.delete_entity(a);
    assert_eq!(d.parent_history(r), (None, vec![]));
    assert_eq!(d.entities[&r].parent, None);
}

/// Merging a unit's parent into it, or a parent further up reached through
/// another unit, leaves no unit its own ancestor: the hierarchy stays
/// usable (a merge used to make a unit its own parent, after which nothing
/// could be put under it - "这会使一个实体成为自己的上级").
#[test]
fn merging_never_makes_a_unit_its_own_ancestor() {
    // direct: R under A from the start, A under B from 1900; B merged into A
    let (mut d, _, [a, b, _]) = region_world();
    d.set_year(Some(1900));
    d.record_parent(a, Some(b)).unwrap();
    d.merge_entities(b, a).unwrap();
    assert_eq!(d.parent_history(a), (None, vec![]), "A is not its own parent");
    let n = d.add_entity(EntityKind::Region, "N");
    d.record_parent(n, Some(a)).unwrap();
    // through another unit: A under C, C under B; B merged into A
    let (mut d, _, [a, b, _]) = region_world();
    let c = d.add_entity(EntityKind::Country, "C");
    d.record_parent(c, Some(b)).unwrap();
    d.record_parent(a, Some(c)).unwrap();
    d.merge_entities(b, a).unwrap();
    assert_eq!(d.parent_history(c), (Some(a), vec![]), "C is now under A");
    assert_eq!(d.parent_history(a), (None, vec![]), "A no longer under C");
    let n = d.add_entity(EntityKind::Region, "N");
    d.record_parent(n, Some(c)).unwrap();
    assert_eq!(d.top_ancestor(n), a);
}

/// Random parent changes and territory edits at random later years; every
/// earlier year keeps exactly its political map (face -> top country).
#[test]
fn later_hierarchy_edits_never_change_earlier_years() {
    let mut d = grid_world(8, 5, 2, 3);
    let countries = d.countries();
    // every country face goes to a region of that country
    let mut regions = Vec::new();
    for (i, c) in countries.iter().enumerate() {
        let r = d.add_entity(EntityKind::Region, &format!("R{i}"));
        for f in d.faces_of(*c) {
            d.assign(f, Some(r)).unwrap();
        }
        d.record_parent(r, Some(*c)).unwrap();
        regions.push(r);
    }
    let mut rng = Rng::new(11);
    let probe = [1700, 1850, 1900, 1950, 2000];
    let (mut ok, mut refused) = (0, 0);
    for step in 0..80 {
        let year = 1800 + (rng.next_f64() * 200.0) as i32;
        let before: Vec<_> = probe.iter().filter(|y| **y < year).map(|y| (*y, top_map(&d, Some(*y)))).collect();
        d.set_year(Some(year));
        let pick = |rng: &mut Rng, v: &[EntityId]| v[(rng.next_f64() * v.len() as f64) as usize];
        let r = if step % 4 == 3 {
            // occasionally nest countries, which can create cycles
            let (x, y) = (pick(&mut rng, &countries), pick(&mut rng, &countries));
            d.record_parent(x, (x != y).then_some(y))
        } else {
            let (reg, c) = (pick(&mut rng, &regions), pick(&mut rng, &countries));
            d.record_parent(reg, Some(c))
        };
        if r.is_err() {
            refused += 1;
            continue;
        }
        for (y, m) in &before {
            assert_eq!(&top_map(&d, Some(*y)), m, "step {step}: editing {year} changed {y}");
        }
        ok += 1;
    }
    assert!(ok >= 50, "{ok} ok, {refused} refused");
}

#[test]
fn old_files_keep_their_parents() {
    let (d, _, [a, _, r]) = region_world();
    let mut v: serde_json::Value = serde_json::to_value(&d).unwrap();
    // entities serialise as a map keyed by id
    let ents = v["entities"].as_object_mut().unwrap();
    assert!(!ents.is_empty());
    for e in ents.values_mut() {
        e.as_object_mut().unwrap().remove("parent_changes");
    }
    let mut old: Document = serde_json::from_value(v).unwrap();
    old.refresh_time_view();
    assert_eq!(old.entities[&r].parent, Some(a));
    assert_eq!(old.parent_history(r), (Some(a), vec![]));
}

/// An author's change point is an event, not just a state: recording "B
/// from 1920" when "B from 1950" already exists must keep the 1950 event,
/// so that inserting "C from 1930" later still returns to B in 1950.
#[test]
fn recorded_events_survive_later_inserts() {
    let (mut d, f, [_a, b, c]) = one_face();
    d.set_year(Some(1950));
    d.assign(f, Some(b)).unwrap();
    d.set_year(Some(1920));
    d.assign(f, Some(b)).unwrap();
    assert_eq!(d.face_history(f).1, vec![(1920, Some(b)), (1950, Some(b))], "1950 kept although it repeats 1920");
    d.set_year(Some(1930));
    d.assign(f, Some(c)).unwrap();
    for (y, who) in [(1925, b), (1940, c), (1950, b), (2000, b)] {
        assert_eq!(d.owner_at(f, Some(y)), Some(who), "{y}");
    }
    // a no-op at a year is still not recorded
    d.set_year(Some(1960));
    d.assign(f, Some(b)).unwrap();
    assert_eq!(d.face_history(f).1.len(), 3);
}

/// The cycle check only walks up from the edited entity; a brute-force scan
/// of every entity at every year must agree that no accepted edit made a
/// cycle anywhere.
#[test]
fn accepted_parent_edits_never_leave_a_cycle() {
    let mut d = Document::default();
    let ents: Vec<EntityId> = (0..8).map(|i| d.add_entity(EntityKind::Country, &format!("E{i}"))).collect();
    let mut rng = Rng::new(17);
    let (mut ok, mut refused) = (0, 0);
    for _ in 0..400 {
        let year = 1800 + (rng.next_f64() * 200.0) as i32;
        d.set_year(Some(year));
        let e = ents[(rng.next_f64() * 8.0) as usize];
        let p = if rng.next_f64() < 0.2 { None } else { Some(ents[(rng.next_f64() * 8.0) as usize]) };
        if p == Some(e) {
            continue;
        }
        if d.record_parent(e, p).is_err() {
            refused += 1;
            continue;
        }
        ok += 1;
        let years: Vec<Option<i32>> = std::iter::once(None).chain(std::iter::once(Some(i32::MIN))).chain(d.change_years().into_iter().map(Some)).collect();
        for y in years {
            for start in &ents {
                let mut cur = *start;
                for step in 0..=ents.len() {
                    match d.parent_at(cur, y) {
                        Some(p) => {
                            assert!(p != *start, "cycle through {start:?} in {y:?}");
                            cur = p;
                        }
                        None => break,
                    }
                    assert!(step < ents.len(), "chain too long in {y:?}");
                }
            }
        }
    }
    assert!(ok > 100 && refused > 10, "{ok} ok, {refused} refused");
}
