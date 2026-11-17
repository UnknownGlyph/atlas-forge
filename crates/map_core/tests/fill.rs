//! The fill bucket: the connected land of one owner (or of no one) goes to
//! a country in one step; other owners' land, and land across water, stays.

use map_core::history::History;
use map_core::sample::grid_world;
use map_core::FaceId;

#[test]
fn a_patch_of_one_owner_goes_to_another() {
    let mut d = grid_world(6, 4, 3, 5);
    let cs = d.countries();
    let (a, b) = (cs[0], cs[1]);
    let fa = d.faces_of(a);
    let patch = d.same_owner_patch(fa[0]);
    // the patch is A's land around the clicked face, and nothing else
    assert!(!patch.is_empty() && patch.iter().all(|f| d.owner.get(f) == Some(&a)));
    let before_other: Vec<(FaceId, _)> = d.topo.faces.keys().filter(|f| !patch.contains(f)).map(|f| (*f, d.owner.get(f).copied())).collect();
    let mut h = History::default();
    let start = d.clone();
    let n = h.apply(&mut d, "填充", |d| d.fill(fa[0], Some(b))).unwrap();
    assert_eq!(n, patch.len());
    assert!(patch.iter().all(|f| d.owner.get(f) == Some(&b)));
    assert!(before_other.iter().all(|(f, o)| d.owner.get(f).copied() == *o), "nothing else changed");
    // filling with the owner it already has changes nothing
    assert_eq!(d.fill(fa[0], Some(b)).unwrap(), 0);
    h.undo(&mut d);
    assert_eq!(d, start);
}

#[test]
fn unclaimed_land_is_filled_as_one_patch_and_years_are_kept() {
    let mut d = grid_world(6, 4, 3, 5);
    let cs = d.countries();
    // two neighbouring countries' land made no one's: one unclaimed patch
    let a = cs[0];
    let nb = d.topo.edges.iter().find_map(|(_, e)| {
        let (l, r) = (d.owner.get(&e.left).copied(), d.owner.get(&e.right).copied());
        match (l, r) {
            (Some(x), Some(y)) if x == a && y != a => Some(y),
            (Some(x), Some(y)) if y == a && x != a => Some(x),
            _ => None,
        }
    }).expect("a neighbour");
    let other = *cs.iter().find(|c| **c != a && **c != nb).unwrap();
    let land: Vec<FaceId> = d.faces_of(a).into_iter().chain(d.faces_of(nb)).collect();
    for f in &land {
        d.assign(*f, None).unwrap();
    }
    let patch = d.same_owner_patch(land[0]);
    assert!(land.iter().all(|f| patch.contains(f)), "both countries' former land");
    assert!(patch.iter().all(|f| d.owner.get(f).is_none()));
    // filled from 1900 on: before 1900 it is still no one's
    d.set_year(Some(1900));
    d.fill(land[0], Some(other)).unwrap();
    assert!(land.iter().all(|f| d.owner.get(f) == Some(&other)));
    d.set_year(Some(1899));
    assert!(land.iter().all(|f| d.owner.get(f).is_none()));
    // water is not filled
    assert!(d.same_owner_patch(map_core::BACKGROUND).is_empty());
    assert!(d.fill(map_core::BACKGROUND, Some(other)).is_err());
}
