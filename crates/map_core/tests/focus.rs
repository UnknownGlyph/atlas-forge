//! Local views: a region and a stretch of years, kept in the project, to
//! look at a war or another local change on its own. What changed there
//! is worked out from the map's history; changes elsewhere do not count.

use map_core::doc::Document;
use map_core::focus::Focus;
use map_core::geom::{self, LonLat};
use map_core::history::History;
use map_core::sample::grid_world;
use map_core::{EntityId, FaceId};

/// The region around one face (its outline's box, a little larger).
fn around(d: &Document, f: FaceId) -> [f64; 4] {
    let (outer, _) = d.topo.face_planar(f);
    let b = geom::bbox(&outer);
    [b[0] - 0.5, b[1] - 0.5, b[2] + 0.5, b[3] + 0.5]
}

/// A grid world and two neighbouring countries' faces, far from a third.
fn world() -> (Document, FaceId, EntityId, EntityId, FaceId, EntityId) {
    let d = grid_world(8, 5, 4, 9);
    let cs = d.countries();
    let (a, b) = (cs[0], cs[1]);
    let fa = d.faces_of(a)[0];
    let far = *cs.last().unwrap();
    let ff = d.faces_of(far)[0];
    (d, fa, a, b, ff, far)
}

#[test]
fn a_war_in_its_region_and_years() {
    let (mut d, fa, a, b, ff, far) = world();
    // B takes A's land in 1914 and gives it back in 1918; elsewhere, in
    // 1916, a third country loses land to A
    for (y, f, who) in [(1914, fa, b), (1918, fa, a), (1916, ff, a)] {
        d.set_year(Some(y));
        d.record_owner(f, Some(who)).unwrap();
    }
    let _ = far;
    d.set_year(None);
    let id = d.add_focus("北境战争", around(&d, fa), Some((1910, 1920))).unwrap();
    let war = d.focus(id).unwrap().clone();
    let faces = d.faces_in(&war);
    assert!(faces.contains(&fa) && !faces.contains(&ff), "the region holds the war, not the far land");
    // over the whole war: two change years in the region, and in the end nothing changed hands
    let rep = d.focus_report(&war).unwrap();
    assert_eq!(rep.years, vec![1914, 1918]);
    assert!(rep.transfers.is_empty() && rep.areas.is_empty(), "{rep:?}");
    // in the middle of it: A's land is B's
    assert_eq!(d.changed_between(&faces, 1910, 1915), vec![fa]);
    assert!(d.changed_between(&faces, 1910, 1919).is_empty());
    let mut mid = war.clone();
    mid.years = Some((1910, 1915));
    let rep = d.focus_report(&mid).unwrap();
    assert_eq!(rep.years, vec![1914]);
    let km2 = d.topo.face_area(fa, d.radius());
    assert_eq!(rep.transfers.len(), 1);
    let (from, to, area) = rep.transfers[0];
    assert_eq!((from, to), (Some(a), Some(b)));
    assert!((area - km2).abs() < 1e-6);
    // A lost what B gained
    let change = |e: EntityId| rep.areas.iter().find(|x| x.0 == Some(e)).map(|x| x.2 - x.1).unwrap();
    assert!((change(a) + km2).abs() < 1e-6 && (change(b) - km2).abs() < 1e-6);
    // in its first year nothing has changed yet; a stretch of one year, or
    // one given backwards, has no change years
    let start = d.change_report(&faces, 1910, 1910);
    assert!(start.years.is_empty() && start.transfers.is_empty() && start.areas.is_empty());
    assert!(d.change_years_of(&faces, 1914, 1914).is_empty() && d.change_years_of(&faces, 1920, 1910).is_empty());
    // a place without years has no report
    let place = d.add_focus("", around(&d, fa), None).unwrap();
    assert!(d.focus_report(d.focus(place).unwrap()).is_none());
}

#[test]
fn a_holder_that_ends_is_a_change() {
    let (mut d, fa, a, _, _, _) = world();
    d.entities.get_mut(&a).unwrap().ended = Some(1930);
    let id = d.add_focus("", around(&d, fa), Some((1900, 1950))).unwrap();
    let f = d.focus(id).unwrap().clone();
    let rep = d.focus_report(&f).unwrap();
    assert_eq!(rep.years, vec![1931], "the year after its end");
    assert!(rep.transfers.iter().any(|t| t.0 == Some(a) && t.1.is_none()), "its land is no one's");
    assert_eq!(d.shown_owner(fa, 1931), None);
    assert_eq!(d.shown_owner(fa, 1930), Some(a));
}

#[test]
fn regions_names_and_checks() {
    let mut d = Document::default();
    // across the antimeridian, given either way
    let a = d.add_focus("", [170.0, -10.0, 190.0, 10.0], None).unwrap();
    let b = d.add_focus("", [-190.0, -10.0, -170.0, 10.0], None).unwrap();
    let (fa, fb) = (d.focus(a).unwrap().clone(), d.focus(b).unwrap().clone());
    assert_eq!((fa.name.as_str(), fb.name.as_str()), ("局部 1", "局部 2"));
    assert_eq!(fb.bounds, [170.0, -10.0, 190.0, 10.0]);
    for f in [&fa, &fb] {
        assert!(f.contains(LonLat::new(-175.0, 0.0)) && f.contains(LonLat::new(175.0, 5.0)));
        assert!(!f.contains(LonLat::new(0.0, 0.0)) && !f.contains(LonLat::new(175.0, 20.0)));
        assert!((f.center().lon.abs() - 180.0).abs() < 1e-9);
    }
    let ring = fa.outline(2.0);
    assert_eq!(ring.first(), ring.last());
    assert!(ring.iter().all(|p| (p.lat + 10.0).abs() < 1e-9 || (p.lat - 10.0).abs() < 1e-9 || (p.lon.abs() - 170.0).abs() < 1e-9));
    // bad regions and names are refused
    for bad in [[10.0, 0.0, 10.0, 5.0], [0.0, 5.0, 10.0, 5.0], [0.0, 0.0, 400.0, 5.0], [f64::NAN, 0.0, 1.0, 1.0]] {
        assert!(d.add_focus("x", bad, None).is_err(), "{bad:?}");
    }
    let mut f = fa.clone();
    f.name = "  ".into();
    assert!(d.update_focus(f.clone()).is_err());
    // years are kept in order
    f.name = "海峡之战".into();
    f.years = Some((1920, 1910));
    d.update_focus(f).unwrap();
    assert_eq!(d.focus(a).unwrap().years, Some((1910, 1920)));
    d.remove_focus(b).unwrap();
    assert!(d.remove_focus(b).is_err());
    assert_eq!(d.focuses.len(), 1);
}

#[test]
fn saved_undone_and_older_files() {
    let (mut d, fa, _, _, _, _) = world();
    let mut h = History::default();
    let before = d.clone();
    let bounds = around(&d, fa);
    h.apply(&mut d, "新建局部视图", |d| d.add_focus("战役", bounds, Some((1900, 1905))).map(|_| ())).unwrap();
    assert_eq!(d.focuses.len(), 1);
    // undone
    h.undo(&mut d);
    assert_eq!(d, before);
    // saved and read back (a world JSON can hold)
    let mut e = Document::default();
    e.add_focus("战役", [10.0, 0.0, 30.0, 20.0], Some((1900, 1905))).unwrap();
    let back: Document = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
    assert_eq!(back.focuses, e.focuses);
    // a file from before local views
    let mut v = serde_json::to_value(&back).unwrap();
    v.as_object_mut().unwrap().remove("focuses");
    let old: Document = serde_json::from_value(v).unwrap();
    assert!(old.focuses.is_empty());
    let _: Option<&Focus> = old.focus(1);
}
