//! Entity lifespans: with a year selected an entity that does not exist
//! owns nothing and leads no one; the records are never changed by it.

use map_core::doc::{Document, Entity, EntityKind};
use map_core::lifespan::Presence;
use map_core::FaceKind;

fn world() -> (Document, map_core::EntityId, map_core::EntityId) {
    let d = map_core::sample::demo_world();
    let c = d.countries();
    (d, c[0], c[1])
}

#[test]
fn lifespans_are_inclusive_and_open_by_default() {
    let (mut d, e, _) = world();
    let x = &d.entities[&e];
    assert!(x.exists_in(None) && x.exists_in(Some(-3000)) && x.lifespan_text().is_empty());
    let x = d.entities.get_mut(&e).unwrap();
    x.founded = Some(1871);
    x.ended = Some(1918);
    let x = &d.entities[&e];
    assert!(x.exists_in(Some(1871)) && x.exists_in(Some(1918)) && x.exists_in(None));
    assert_eq!((x.presence(Some(1870)), x.presence(Some(1919)), x.presence(Some(1900))), (Presence::NotYet, Presence::Ended, Presence::Exists));
    assert_eq!(x.lifespan_text(), "1871–1918 年");
    // saved before lifespans existed: open at both ends, no successors
    let v = serde_json::to_value(&d.entities[&e]).unwrap();
    let mut o = v.as_object().unwrap().clone();
    for k in ["founded", "ended", "successors"] {
        o.remove(k);
    }
    let back: Entity = serde_json::from_value(serde_json::Value::Object(o)).unwrap();
    assert!(back.founded.is_none() && back.ended.is_none() && back.successors.is_empty());
}

/// After its end an entity owns nothing in the year's view: its land is
/// listed for confirmation, the records still name it, and without a year
/// (or before the end) it owns its land as before. Land cannot be given to
/// it in a year it does not exist. Handing the land to a successor from
/// the year after the end leaves earlier years alone.
#[test]
fn an_ended_country_owns_nothing_until_its_land_is_handed_on() {
    let (mut d, e, other) = world();
    let faces = d.faces_of(e);
    assert!(!faces.is_empty());
    d.entities.get_mut(&e).unwrap().ended = Some(1920);
    d.successors_set(e, vec![other]);
    d.set_year(Some(1910));
    assert_eq!(d.faces_of(e).len(), faces.len());
    d.set_year(Some(1930));
    assert!(d.faces_of(e).is_empty() && !d.areas_by_entity().contains_key(&e));
    assert!(faces.iter().all(|f| d.owner_at(*f, Some(1930)) == Some(e)), "records unchanged");
    let c = d.lifespan_conflicts(1930);
    assert_eq!(c.len(), 1);
    assert_eq!((c[0].entity, c[0].presence, c[0].faces.len()), (e, Presence::Ended, faces.len()));
    let theirs = d.faces_of(other)[0];
    assert!(d.record_owner(theirs, Some(e)).is_err(), "no land for an entity that does not exist");
    assert_eq!(d.owner.get(&theirs), Some(&other));
    d.set_year(None);
    assert_eq!(d.faces_of(e).len(), faces.len(), "no year: as before");
    assert!(d.change_years().contains(&1921));

    d.set_year(Some(1930));
    assert_eq!(d.reassign_after_end(e, Some(other)).unwrap(), faces.len());
    assert!(d.lifespan_conflicts(1930).is_empty());
    assert!(faces.iter().all(|f| d.owner_at(*f, Some(1921)) == Some(other) && d.owner_at(*f, Some(1920)) == Some(e)));
    d.set_year(Some(1915));
    assert_eq!(d.faces_of(e).len(), faces.len(), "earlier years untouched");
}

/// Land that rises from the sea again goes back to its recorded owner if
/// that owner still exists, and is left unassigned and listed for
/// confirmation if the owner has ended.
#[test]
fn land_rising_again_is_not_handed_to_an_ended_owner() {
    let (mut d, e, _) = world();
    let f = d.faces_of(e)[0];
    d.set_year(Some(1900));
    d.record_kind(f, FaceKind::Water).unwrap();
    d.set_year(Some(1950));
    d.record_kind(f, FaceKind::Land).unwrap();
    assert_eq!(d.owner.get(&f), Some(&e), "owner still exists: it comes back to it");
    d.entities.get_mut(&e).unwrap().ended = Some(1920);
    d.refresh_time_view();
    assert!(d.owner.get(&f).is_none(), "owner ended: unassigned");
    let c = d.lifespan_conflicts(1950);
    assert!(c.iter().any(|x| x.entity == e && x.faces.contains(&f)));
    // in 1925 the face is water: nothing to confirm for it then
    assert!(!d.lifespan_conflicts(1925).iter().any(|x| x.faces.contains(&f)));
    d.set_year(Some(1890));
    assert_eq!(d.owner.get(&f), Some(&e), "before it sank, still the old owner");
    // left unassigned from the year after the end
    d.reassign_after_end(e, None).unwrap();
    d.set_year(Some(1950));
    assert!(d.owner.get(&f).is_none() && d.lifespan_conflicts(1950).is_empty());
}

/// A province of a country that has ended has no parent in the year's
/// view (and is listed); before a country is founded it has no members
/// either.
#[test]
fn members_of_an_entity_that_does_not_exist() {
    let (mut d, e, _) = world();
    let r = d.add_entity(EntityKind::Region, "省");
    d.entities.get_mut(&r).unwrap().parent_base = Some(e);
    d.entities.get_mut(&e).unwrap().founded = Some(1871);
    d.set_year(Some(1900));
    assert_eq!(d.entities[&r].parent, Some(e));
    d.set_year(Some(1850));
    assert_eq!(d.entities[&r].parent, None);
    let c = d.lifespan_conflicts(1850);
    let row = c.iter().find(|x| x.entity == e).unwrap();
    assert_eq!(row.presence, Presence::NotYet);
    assert!(row.members.contains(&r) && !row.faces.is_empty());
}

/// Successor links follow merges and go with deletions; predecessors are
/// the reverse links.
#[test]
fn successor_links_follow_merges_and_deletions() {
    let (mut d, a, b) = world();
    let c = d.countries()[2];
    d.successors_set(a, vec![b]);
    assert_eq!(d.predecessors(b), vec![a]);
    d.merge_entities(b, c).unwrap();
    assert_eq!(d.entities[&a].successors, vec![c]);
    d.delete_entity(c);
    assert!(d.entities[&a].successors.is_empty());
}
