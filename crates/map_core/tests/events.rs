//! The event timeline: events in order, their changes in their region or
//! on the whole map, the chronicle in the world's own calendar, and links
//! to local views that go away with the view.

use map_core::doc::Document;
use map_core::geom;
use map_core::history::History;
use map_core::sample::grid_world;

#[test]
fn events_and_the_chronicle() {
    let mut d = grid_world(8, 5, 4, 9);
    let cs = d.countries();
    let (a, b) = (cs[0], cs[1]);
    let fa = d.faces_of(a)[0];
    for (y, who) in [(1914, b), (1918, a)] {
        d.set_year(Some(y));
        d.record_owner(fa, Some(who)).unwrap();
    }
    d.set_year(None);
    let bb = geom::bbox(&d.topo.face_planar(fa).0);
    let region = d.add_focus("北境", [bb[0] - 0.5, bb[1] - 0.5, bb[2] + 0.5, bb[3] + 0.5], Some((1910, 1920))).unwrap();
    // added out of order, kept in order of years
    let peace = d.add_event("北境和约", 1918, None);
    let war = d.add_event("北境战争", 1914, Some(1917));
    let quiet = d.add_event("", 1700, None);
    assert_eq!(d.event(quiet).unwrap().name, "事件 3");
    let names: Vec<&str> = d.events_in_order().iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, ["事件 3", "北境战争", "北境和约"]);
    // the war took the land; its region's report says so
    let mut w = d.event(war).unwrap().clone();
    w.focus = Some(region);
    w.note = "边境冲突".into();
    d.update_event(w).unwrap();
    let ch = d.chronicle();
    let war_entry = ch.iter().find(|c| c.event.id == war).unwrap();
    assert_eq!(war_entry.report.transfers.len(), 1);
    assert_eq!((war_entry.report.transfers[0].0, war_entry.report.transfers[0].1), (Some(a), Some(b)));
    // the peace gave it back (on the whole map: no region)
    let peace_entry = ch.iter().find(|c| c.event.id == peace).unwrap();
    assert!(peace_entry.report.transfers.iter().any(|t| t.0 == Some(b) && t.1 == Some(a)));
    // a quiet year: nothing changed
    assert!(ch[0].report.transfers.is_empty());
    // the chronicle, in the world's calendar
    d.calendar.on = true;
    d.calendar.epoch = 1800;
    let md = d.chronicle_markdown();
    assert!(md.contains("新历115年—新历118年　北境战争"), "{md}");
    assert!(md.contains("地区：北境") && md.contains("边境冲突"));
    assert!(md.contains(&format!("{} → {}", d.entities[&a].name, d.entities[&b].name)));
    assert!(md.contains("（期间领土没有变化）"));
    // the view removed: the war keeps its years, without the region
    d.remove_focus(region).unwrap();
    assert_eq!(d.event(war).unwrap().focus, None);
    // a link to a view that is not there is refused; names are needed
    let mut bad = d.event(war).unwrap().clone();
    bad.focus = Some(99);
    assert!(d.update_event(bad).is_err());
    let mut unnamed = d.event(war).unwrap().clone();
    unnamed.name = " ".into();
    assert!(d.update_event(unnamed).is_err());
}

#[test]
fn undone_saved_and_older_files() {
    let mut d = Document::default();
    let mut h = History::default();
    let before = d.clone();
    h.apply(&mut d, "新建事件", |d| Ok(d.add_event("加冕", 1500, Some(1500)))).unwrap();
    assert_eq!(d.events[0].end, None, "an end no later than the start is dropped");
    let back: Document = serde_json::from_str(&serde_json::to_string(&d).unwrap()).unwrap();
    assert_eq!(back.events, d.events);
    let mut v = serde_json::to_value(&d).unwrap();
    v.as_object_mut().unwrap().remove("events");
    let old: Document = serde_json::from_value(v).unwrap();
    assert!(old.events.is_empty());
    h.undo(&mut d);
    assert_eq!(d, before);
    assert!(d.remove_event(1).is_err());
}
