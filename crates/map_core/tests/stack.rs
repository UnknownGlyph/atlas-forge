//! Layers of drawn features: membership, order, and what hiding, locking
//! and deleting do.

use map_core::doc::{Annotation, Document, LineFeature, LineId};
use map_core::features::FeatureRef;
use map_core::stack::DrawPass;
use map_core::symbol::{MapSymbol, SymbolKind};
use map_core::LonLat;

/// Three symbols, a text and a river.
fn world() -> (Document, Vec<FeatureRef>) {
    let mut d = Document::default();
    let mut fs = Vec::new();
    for i in 0..3 {
        let id = LineId(d.topo.alloc());
        d.symbols.insert(id, MapSymbol::new(SymbolKind::Port, LonLat::new(10.0 * i as f64, 5.0)));
        fs.push(FeatureRef::Symbol(id));
    }
    let t = LineId(d.topo.alloc());
    d.annotations.insert(t, Annotation::new("海", LonLat::new(40.0, 0.0)));
    fs.push(FeatureRef::Text(t));
    let r = LineId(d.topo.alloc());
    d.rivers.insert(r, LineFeature { name: "河".into(), pts: vec![LonLat::new(0.0, 20.0), LonLat::new(5.0, 22.0)], route: None, legend: String::new(), show_name: false, hidden: false });
    fs.push(FeatureRef::River(r));
    (d, fs)
}

#[test]
fn a_document_without_groups_draws_as_before() {
    let (d, fs) = world();
    assert_eq!(d.draw_passes(), [DrawPass { group: None, opacity: 1.0 }]);
    assert_eq!(d.group_members(None).len(), 5);
    assert!(fs.iter().all(|f| d.pickable(*f) && d.group_opacity(f.id()) == 1.0));
    // and an older file, without the fields, reads as one
    let mut v = serde_json::to_value(&d).unwrap();
    let o = v.as_object_mut().unwrap();
    assert!(o.remove("groups").is_some() && o.remove("group_of").is_some());
    let old: Document = serde_json::from_value(v).unwrap();
    assert!(old.groups.is_empty() && old.group_of.is_empty());
}

#[test]
fn membership_order_and_removal() {
    let (mut d, fs) = world();
    let a = d.add_group("战役");
    let b = d.add_group("");
    assert_eq!(d.group(b).unwrap().name, "图层 2");
    d.set_group(&fs[..2], Some(a)).unwrap();
    d.set_group(&fs[2..4], Some(b)).unwrap();
    assert_eq!((d.group_members(Some(a)), d.group_members(Some(b)).len(), d.group_members(None)), (fs[..2].to_vec(), 2, vec![fs[4]]));
    // a feature is in one group: moving it takes it out of the other
    d.set_group(&fs[1..2], Some(b)).unwrap();
    assert_eq!((d.group_members(Some(a)).len(), d.group_members(Some(b)).len()), (1, 3));
    // the stack: no group first, then bottom to top
    let order = |d: &Document| d.draw_passes().iter().map(|p| p.group).collect::<Vec<_>>();
    assert_eq!(order(&d), [None, Some(a), Some(b)]);
    d.move_group(a, 1).unwrap();
    assert_eq!(order(&d), [None, Some(b), Some(a)]);
    d.move_group(a, 99).unwrap();
    d.move_group(b, 0).unwrap();
    assert_eq!(order(&d), [None, Some(b), Some(a)]);
    d.rename_group(a, " 旧战役 ").unwrap();
    assert_eq!(d.group(a).unwrap().name, "旧战役");
    assert!(d.rename_group(a, "  ").is_err());
    // a copy stays in its original's layer; a deleted feature leaves its layer
    let dup = d.duplicate_features(&fs[..1], 1.0, 1.0).unwrap();
    assert_eq!(d.group_of_feature(dup[0]), Some(a));
    assert_eq!(d.delete_features(&dup), 1);
    assert!(d.group_of_feature(dup[0]).is_none() && d.group_members(Some(a)).len() == 1);
    // removing a group keeps its features, in no group
    assert_eq!(d.remove_group(b).unwrap(), 3);
    assert_eq!((order(&d), d.group_members(None).len(), d.features().len()), (vec![None, Some(a)], 4, 5));
    assert!(d.remove_group(b).is_err() && d.set_group(&fs[..1], Some(b)).is_err() && d.move_group(b, 0).is_err());
    assert!(d.set_group(&dup, Some(a)).is_err(), "a feature that is gone");
    // stale entries are forgotten
    d.symbols.remove(&fs[0].id());
    d.prune_groups();
    assert!(d.group_of.is_empty());
}

#[test]
fn hiding_locking_and_opacity() {
    let (mut d, fs) = world();
    let g = d.add_group("河流与港口");
    d.set_group(&[fs[0], fs[4]], Some(g)).unwrap();
    let FeatureRef::River(river) = fs[4] else { unreachable!() };
    assert!(d.river_at(LonLat::new(2.5, 21.0), 0.5).is_some());
    d.set_group_opacity(g, 0.4).unwrap();
    assert_eq!((d.group_opacity(fs[0].id()), d.group_opacity(fs[1].id())), (0.4, 1.0));
    d.set_group_opacity(g, 7.0).unwrap();
    assert_eq!(d.group(g).unwrap().opacity, 1.0);
    assert!(d.set_group_opacity(g, f32::NAN).is_err());
    // locked: drawn, not picked
    d.set_group_locked(g, true).unwrap();
    assert!(d.locked_in_effect(fs[0]) && !d.is_locked(fs[0]) && !d.pickable(fs[0]) && d.pickable(fs[1]));
    assert!(!d.hidden_in_effect(fs[0]) && d.draw_passes().len() == 2);
    d.set_group_locked(g, false).unwrap();
    // hidden: not drawn, not picked, not found under the pointer
    d.set_group_hidden(g, true).unwrap();
    assert!(d.hidden_in_effect(fs[0]) && d.group_hides(river) && !d.pickable(fs[4]) && d.pickable(fs[1]));
    assert_eq!(d.draw_passes(), [DrawPass { group: None, opacity: 1.0 }]);
    assert!(d.river_at(LonLat::new(2.5, 21.0), 0.5).is_none(), "a river of a hidden layer is not under the pointer");
    assert!(!d.rivers[&river].hidden, "its own switch is untouched");
    d.set_group_hidden(g, false).unwrap();
    assert!(d.pickable(fs[4]) && d.river_at(LonLat::new(2.5, 21.0), 0.5).is_some());
    // a feature hidden by itself stays hidden in a shown layer
    d.set_hidden(fs[0], true).unwrap();
    assert!(d.hidden_in_effect(fs[0]) && !d.group_hides(fs[0].id()));
}
