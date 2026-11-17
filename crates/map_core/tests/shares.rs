//! Share layers: a face is divided between units by percentage (culture,
//! religion). Areas are weighted by share, colours mixed, and every other
//! layer and every earlier year stay untouched.

use map_core::doc::{dominant, rebalance, Document, LayerKind, Shares, FULL_SHARE, POLITICAL};
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::sample::grid_world;
use map_core::topo::{FaceId, FaceKind};
use map_core::{EntityId, EntityKind};
use std::collections::HashMap;

fn sh(v: &[(EntityId, u16)]) -> Shares {
    v.iter().copied().collect()
}

fn areas(d: &Document) -> HashMap<EntityId, f64> {
    d.areas_by_entity()
}

fn political(d: &Document) -> HashMap<EntityId, i64> {
    let mut v = d.clone();
    v.set_active_layer(POLITICAL).unwrap();
    v.areas_by_entity().into_iter().map(|(e, a)| (e, a.round() as i64)).collect()
}

#[test]
fn rebalancing_and_dominance() {
    let (a, b, c) = (EntityId(1), EntityId(2), EntityId(3));
    let s = sh(&[(a, 6000), (b, 4000)]);
    assert_eq!(rebalance(&s, c, 5000), sh(&[(a, 3000), (b, 2000), (c, 5000)]));
    assert_eq!(rebalance(&s, a, FULL_SHARE), sh(&[(a, FULL_SHARE)]));
    assert_eq!(rebalance(&s, b, 0), sh(&[(a, 6000)]), "zero removes the unit");
    // room left: nobody is scaled
    assert_eq!(rebalance(&sh(&[(a, 3000)]), b, 5000), sh(&[(a, 3000), (b, 5000)]));
    assert_eq!(dominant(&sh(&[(a, 4000), (b, 4000)])), Some(a), "ties go to the lower id");
    assert_eq!(dominant(&Shares::new()), None);
}

/// Grid world with a share layer "宗教" and two units.
fn world() -> (Document, map_core::doc::LayerId, [EntityId; 2], FaceId) {
    let mut d = grid_world(8, 5, 2, 6);
    let rel = d.add_layer_of("宗教", LayerKind::Shares);
    d.set_active_layer(rel).unwrap();
    let (x, y) = (d.add_entity(EntityKind::Country, "甲教"), d.add_entity(EntityKind::Country, "乙教"));
    let f = *d.topo.faces.iter().find(|(_, f)| f.kind == FaceKind::Land).unwrap().0;
    d.set_share(f, Some(x), 6000).unwrap();
    d.set_share(f, Some(y), 4000).unwrap();
    (d, rel, [x, y], f)
}

#[test]
fn shares_weight_areas_and_mix_colours() {
    let (mut d, _, [x, y], f) = world();
    assert_eq!(d.shares(f), sh(&[(x, 6000), (y, 4000)]));
    assert_eq!(d.owner.get(&f), Some(&x), "the largest share owns the face for labels and lines");
    let a = d.topo.face_area(f, d.radius());
    let ar = areas(&d);
    assert!((ar[&x] - 0.6 * a).abs() < 1e-6 * a && (ar[&y] - 0.4 * a).abs() < 1e-6 * a);
    // raising Y to 70% scales X down to fit
    d.set_share(f, Some(y), 7000).unwrap();
    assert_eq!(d.shares(f), sh(&[(x, 3000), (y, 7000)]));
    assert_eq!(d.owner.get(&f), Some(&y));
    // colours mixed by share
    let th = map_core::theme::evaluate(&d);
    let (cx, cy) = (th.entity_colors[&x], th.entity_colors[&y]);
    let mix: Vec<u8> = (0..3).map(|k| ((cx[k] as f64 * 0.3 + cy[k] as f64 * 0.7).round()) as u8).collect();
    assert_eq!(&th.face_colors[&f][..3], &mix[..]);
    // the brush on a share layer (assign) means 100%; clearing empties it
    d.assign(f, Some(x)).unwrap();
    assert_eq!(d.shares(f), sh(&[(x, FULL_SHARE)]));
    d.assign(f, None).unwrap();
    assert!(d.shares(f).is_empty() && !d.owner.contains_key(&f));
}

#[test]
fn shares_change_by_year_without_touching_earlier_years() {
    let (mut d, _, [x, y], f) = world();
    d.set_year(Some(1900));
    d.set_share(f, Some(y), FULL_SHARE).unwrap();
    d.set_year(Some(1899));
    assert_eq!(d.shares(f), sh(&[(x, 6000), (y, 4000)]));
    d.set_year(Some(1950));
    assert_eq!(d.shares(f), sh(&[(y, FULL_SHARE)]));
    assert!(d.change_years().contains(&1900));
    assert_eq!(d.share_history(f).1.len(), 1);
    d.set_year(None);
    let before = d.clone();
    assert!(d.set_share(f, Some(x), 5000).is_err(), "all-years edit of a history");
    assert_eq!(d, before);
}

#[test]
fn other_layers_and_geometry_stay_consistent() {
    let (mut d, rel, [x, _], f) = world();
    let pol = political(&d);
    d.set_share(f, Some(x), 2500).unwrap();
    assert_eq!(political(&d), pol, "share edits never touch the political layer");
    let shares_before: HashMap<EntityId, f64> = areas(&d);
    // edit geometry from the political layer: shares follow
    d.set_active_layer(POLITICAL).unwrap();
    let country = d.countries()[0];
    let (n, _) = overlay::apply(&d, &[LonLat::new(-170.0, -40.0), LonLat::new(-60.0, -40.0), LonLat::new(-60.0, 30.0), LonLat::new(-170.0, 30.0)], DrawIntent::Take(country)).unwrap();
    n.topo.validate().unwrap();
    let mut s = n.clone();
    s.set_active_layer(rel).unwrap();
    for (e, v) in &shares_before {
        assert!((areas(&s)[e] - v).abs() < 1e-6 * v.max(1.0), "share area of {e:?} changed");
    }
    // drawing a take on the share layer gives the unit all of the region
    let mut s = d.clone();
    s.set_active_layer(rel).unwrap();
    let (t, _) = overlay::apply(&s, &[LonLat::new(-170.0, -40.0), LonLat::new(-60.0, -40.0), LonLat::new(-60.0, 30.0), LonLat::new(-170.0, 30.0)], DrawIntent::Take(x)).unwrap();
    t.topo.validate().unwrap();
    let taken: Vec<FaceId> = t.owner.iter().filter(|(_, e)| **e == x).map(|(f, _)| *f).collect();
    assert!(taken.len() > 1, "the take produced land for X");
    for f in taken.iter().filter(|f| !d.topo.faces.contains_key(*f)) {
        assert_eq!(t.shares(*f), sh(&[(x, FULL_SHARE)]), "new pieces are wholly X");
    }
    assert_eq!(political(&t), political(&s), "a take on the share layer leaves the political map");
}

#[test]
fn split_and_merge_respect_shares() {
    let mut d = Document::default();
    let f = d.add_ring_face(&[LonLat::new(0.0, 0.0), LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0), LonLat::new(0.0, 10.0)], FaceKind::Land).unwrap();
    let rel = d.add_layer_of("宗教", LayerKind::Shares);
    d.set_active_layer(rel).unwrap();
    let (x, y) = (d.add_entity(EntityKind::Country, "甲教"), d.add_entity(EntityKind::Country, "乙教"));
    d.set_share(f, Some(x), 5000).unwrap();
    d.set_share(f, Some(y), 5000).unwrap();
    d.set_active_layer(POLITICAL).unwrap();
    let (a, b) = d.split_face(&[LonLat::new(5.0, 0.0), LonLat::new(5.0, 10.0)]).unwrap();
    d.set_active_layer(rel).unwrap();
    assert_eq!(d.shares(a), d.shares(b));
    let half = d.topo.face_area(a, d.radius()) + d.topo.face_area(b, d.radius());
    assert!((areas(&d)[&x] - 0.5 * half).abs() < 1e-6 * half);
    d.set_share(b, Some(x), FULL_SHARE).unwrap();
    let before = d.clone();
    assert!(d.merge_faces(a, b).is_err());
    assert_eq!(d, before);
    d.set_share(b, Some(y), 5000).unwrap();
    assert_eq!(d.shares(b), sh(&[(x, 5000), (y, 5000)]));
    d.merge_faces(a, b).unwrap();
    d.topo.validate().unwrap();
    // saved with the share layer active
    let back: Document = rmp_serde::from_slice(&rmp_serde::to_vec_named(&d).unwrap()).unwrap();
    let mut back = back;
    back.topo.rebuild_index();
    back.refresh_time_view();
    assert_eq!(back.shares(a), d.shares(a));
    assert_eq!(back.owner, d.owner);
    // deleting a unit removes its shares everywhere
    d.delete_entity(x);
    assert_eq!(d.shares(a), sh(&[(y, 5000)]));
}
