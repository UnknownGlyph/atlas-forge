//! Partition layers: each layer is an independent exclusive partition of
//! the same faces, with its own units and its own history. Geometry edits
//! keep every layer consistent; editing one layer never changes another.

use map_core::doc::{Document, POLITICAL};
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::sample::grid_world;
use map_core::topo::FaceKind;
use map_core::{EntityId, EntityKind};
use std::collections::HashMap;

fn areas(d: &Document) -> HashMap<EntityId, i64> {
    d.areas_by_entity().into_iter().map(|(e, a)| (e, a.round() as i64)).collect()
}

fn political_areas(d: &Document, year: Option<i32>) -> HashMap<EntityId, i64> {
    let mut v = d.clone();
    v.set_active_layer(POLITICAL).unwrap();
    v.set_year(year);
    areas(&v)
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<LonLat> {
    vec![LonLat::new(x0, y0), LonLat::new(x1, y0), LonLat::new(x1, y1), LonLat::new(x0, y1)]
}

/// A grid world plus a "culture" layer: two cultures splitting the land
/// at longitude 0.
fn world() -> (Document, map_core::doc::LayerId, [EntityId; 2]) {
    let mut d = grid_world(8, 5, 2, 4);
    let culture = d.add_layer("文化区");
    d.set_active_layer(culture).unwrap();
    let (east, west) = (d.add_entity(EntityKind::Country, "东方文化"), d.add_entity(EntityKind::Country, "西方文化"));
    let land: Vec<_> = d.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Land).map(|(id, _)| *id).collect();
    for f in land {
        let (outer, _) = d.topo.face_planar(f);
        let c = outer.iter().map(|p| p[0]).sum::<f64>() / outer.len() as f64;
        d.assign(f, Some(if c > 0.0 { east } else { west })).unwrap();
    }
    (d, culture, [east, west])
}

#[test]
fn layers_are_independent_views_of_the_same_faces() {
    let (mut d, culture, [east, west]) = world();
    let before = political_areas(&d, None);
    assert_eq!(d.countries(), vec![east, west], "units of the active layer");
    let a = areas(&d);
    assert!(a[&east] > 0 && a[&west] > 0);
    // switching layers swaps what the map shows, nothing is lost
    d.set_active_layer(POLITICAL).unwrap();
    assert_eq!(d.countries().len(), 40);
    assert!(!d.owner.values().any(|e| *e == east || *e == west));
    assert_eq!(areas(&d), before);
    d.set_active_layer(culture).unwrap();
    assert_eq!(areas(&d), a);
    assert_eq!(d.layer_ids(), vec![POLITICAL, culture]);
    assert_eq!(d.layer_name(culture), "文化区");
    // units cannot be merged or nested across layers
    let country = political_areas(&d, None).keys().next().copied().unwrap();
    assert!(d.merge_entities(east, country).is_err());
    assert!(d.record_parent(east, Some(country)).is_err());
}

#[test]
fn geometry_edits_keep_every_layer() {
    let (mut d, culture, [east, _]) = world();
    // split a face while the culture layer is active: both halves keep
    // their political owner too
    // an island in open sea, owned in both layers
    let spot = (0..36).map(|i| -175.0 + 10.0 * i as f64).find(|x| (0..5).all(|k| d.topo.face_at(LonLat::new(x + k as f64, -78.0)) == map_core::topo::BACKGROUND)).expect("open sea");
    let island = d.add_ring_face(&rect(spot, -79.0, spot + 4.0, -77.0), FaceKind::Land).unwrap();
    d.assign(island, Some(east)).unwrap();
    d.set_active_layer(POLITICAL).unwrap();
    let country = d.countries()[0];
    d.assign(island, Some(country)).unwrap();
    d.set_active_layer(culture).unwrap();
    let pol = political_areas(&d, None);
    let (a, b) = d.split_face(&[LonLat::new(spot + 2.0, -79.0), LonLat::new(spot + 2.0, -77.0)]).unwrap();
    assert_eq!(political_areas(&d, None), pol, "a split changes no political area");
    let mut p = d.clone();
    p.set_active_layer(POLITICAL).unwrap();
    assert_eq!(p.owner.get(&a), p.owner.get(&b));
    // the halves now differ in the culture layer only: merging is refused
    d.assign(b, None).unwrap();
    assert!(d.merge_faces(a, b).is_err());
    d.assign(b, Some(east)).unwrap();
    d.merge_faces(a, b).unwrap();
    assert_eq!(political_areas(&d, None), pol);
    // drawing in the culture layer: political ownership carried over exactly
    let (n, _) = overlay::apply(&d, &rect(-20.0, -15.0, 25.0, 15.0), DrawIntent::Take(east)).unwrap();
    n.topo.validate().unwrap();
    let after = political_areas(&n, None);
    for (e, v) in &pol {
        assert!((after.get(e).copied().unwrap_or(0) - v).abs() <= 1, "political area of {e:?} changed: {v} -> {:?}", after.get(e));
    }
    assert_eq!(n.active_layer, culture);
}

#[test]
fn each_layer_has_its_own_timeline() {
    let (mut d, culture, [east, west]) = world();
    let f = d.topo.face_at(LonLat::new(30.0, 10.0));
    d.set_year(Some(1900));
    d.assign(f, Some(west)).unwrap();
    assert_eq!(d.owner_at(f, Some(1899)), Some(east));
    // the political map has no change at all
    assert_eq!(political_areas(&d, Some(1899)), political_areas(&d, Some(1950)));
    let mut p = d.clone();
    p.set_active_layer(POLITICAL).unwrap();
    assert!(p.face_history(f).1.is_empty());
    assert_eq!(p.active_layer, POLITICAL);
    p.set_active_layer(culture).unwrap();
    assert_eq!(p.face_history(f).1, vec![(1900, Some(west))]);
}

#[test]
fn deleting_a_layer_and_saving() {
    let (mut d, culture, [east, _]) = world();
    // saved with the culture layer active, loaded intact
    let bytes = rmp_serde::to_vec_named(&d).unwrap();
    let mut back: Document = rmp_serde::from_slice(&bytes).unwrap();
    back.topo.rebuild_index();
    back.refresh_time_view();
    assert_eq!(back.active_layer, culture);
    assert_eq!(areas(&back), areas(&d));
    assert_eq!(political_areas(&back, None), political_areas(&d, None));
    // deleting it removes its units and returns to the political layer
    assert!(d.delete_layer(POLITICAL).is_err());
    d.delete_layer(culture).unwrap();
    assert_eq!(d.active_layer, POLITICAL);
    assert!(!d.entities.contains_key(&east));
    assert_eq!(d.layer_ids(), vec![POLITICAL]);
    assert_eq!(political_areas(&d, None), political_areas(&back, None));
}

#[test]
fn overlay_boundaries_follow_the_other_layer_and_its_year() {
    let (mut d, culture, [east, west]) = world();
    d.set_active_layer(POLITICAL).unwrap();
    d.style.overlay_layer = Some(culture);
    assert_eq!(d.overlay_layer(), Some(culture));
    let own = d.layer_owner(culture);
    let edges = d.layer_border_edges(culture);
    assert!(!edges.is_empty());
    // exactly the land-land edges whose culture differs
    let expect = d
        .topo
        .edges
        .iter()
        .filter(|(_, e)| {
            let land = |f| d.topo.faces[&f].kind == FaceKind::Land;
            land(e.left) && land(e.right) && own.get(&e.left) != own.get(&e.right)
        })
        .count();
    assert_eq!(edges.len(), expect);
    // the culture layer's own timeline moves the boundary
    let f = edges.iter().flat_map(|id| [d.topo.edges[id].left, d.topo.edges[id].right]).find(|f| own.get(f) == Some(&east)).unwrap();
    d.set_active_layer(culture).unwrap();
    d.set_year(Some(1900));
    d.assign(f, Some(west)).unwrap();
    d.set_active_layer(POLITICAL).unwrap();
    let at = |d: &mut Document, y| {
        d.set_year(Some(y));
        d.layer_border_edges(culture)
    };
    assert_eq!(at(&mut d, 1899), edges);
    assert_ne!(at(&mut d, 1900), edges);
    // the active layer is never overlaid on itself
    d.set_active_layer(culture).unwrap();
    assert_eq!(d.overlay_layer(), None);
}
