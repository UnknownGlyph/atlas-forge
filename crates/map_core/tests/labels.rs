//! Where countries' names go: along a line through the largest piece of
//! land, as long and as roomy as the land allows — level in a wide country,
//! slanted along a slanted one, bent along a bent one — and always inside.

use map_core::doc::Document;
use map_core::geom::LonLat;
use map_core::labels::{name_em, LabelPath};
use map_core::overlay::{self, DrawIntent};
use map_core::{EntityId, EntityKind};

fn country(outline: &[LonLat], name: &str) -> (Document, EntityId) {
    let d = Document::default();
    let mut d = d;
    let e = d.add_entity(EntityKind::Country, name);
    let (n, _) = overlay::apply(&d, outline, DrawIntent::Take(e)).unwrap();
    (n, e)
}

fn p(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

/// The name, at the size its line allows, lies on the country's land:
/// its middle line and its top and bottom edges, over the whole stretch
/// it is spread along.
fn name_inside(d: &Document, e: EntityId, path: &LabelPath) {
    let size = path.fit();
    assert!(size > 0.0);
    let len = path.length();
    let span = (path.em * size).max(len * 0.72).min(len);
    for i in 0..=20 {
        let s = (len - span) * 0.5 + span * i as f64 / 20.0;
        for t in [-0.5, 0.0, 0.5] {
            let at = path.point(s, t * size);
            assert_eq!(d.owner.get(&d.topo.face_at(at)), Some(&e), "{at:?} ({s:.2} along, {t} across) is outside");
        }
    }
}

/// The turn of a line from east (degrees), end to end.
fn chord_angle(path: &LabelPath) -> f64 {
    let (a, b) = (path.pts[0], path.pts[path.pts.len() - 1]);
    (b[1] - a[1]).atan2(b[0] - a[0]).to_degrees()
}

/// How far the middle of a line stands above the straight line between its ends.
fn sag(path: &LabelPath) -> f64 {
    let (a, b, m) = (path.pts[0], path.pts[path.pts.len() - 1], path.pts[path.pts.len() / 2]);
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    ((m[0] - a[0]) * -dy + (m[1] - a[1]) * dx) / dx.hypot(dy)
}

#[test]
fn a_wide_country_is_named_level_along_its_length() {
    let (d, e) = country(&[p(-20.0, 0.0), p(20.0, 0.0), p(20.0, 10.0), p(-20.0, 10.0)], "平原国");
    let path = d.label_path(e).unwrap();
    assert!(chord_angle(&path).abs() < 2.0 && sag(&path).abs() < 0.3, "level: {:.1}° {:.2}", chord_angle(&path), sag(&path));
    assert!(path.length() > 30.0 && path.height > 7.0, "long and roomy: {:.1} × {:.1}", path.length(), path.height);
    assert!((path.middle().lat - 5.0).abs() < 1.0 && path.middle().lon.abs() < 2.0, "in the middle: {:?}", path.middle());
    name_inside(&d, e, &path);
    // made again: the same line
    assert_eq!(d.label_path(e), Some(path));
}

#[test]
fn a_slanted_country_is_named_along_its_slant() {
    // a band 40° long, 5° across, rising at about 31°
    let (a, b) = ((-20.0f64, -12.0f64), (20.0f64, 12.0f64));
    let ang = (b.1 - a.1).atan2(b.0 - a.0);
    let (nx, ny) = (-ang.sin() * 2.5, ang.cos() * 2.5);
    let (d, e) = country(&[p(a.0 - nx, a.1 - ny), p(b.0 - nx, b.1 - ny), p(b.0 + nx, b.1 + ny), p(a.0 + nx, a.1 + ny)], "长条国");
    let path = d.label_path(e).unwrap();
    let want = ang.to_degrees();
    assert!((chord_angle(&path) - want).abs() < 8.0, "along the band: {:.1}° (the band {want:.1}°)", chord_angle(&path));
    // with nearly all of the band's width across it, measured square to the line
    assert!(path.height > 4.0 && path.height < 5.05, "room {:.2} of 5", path.height);
    assert!(path.length() > 30.0, "{:.1} long", path.length());
    name_inside(&d, e, &path);
}

#[test]
fn a_bent_country_is_named_along_its_bend() {
    // an arch: a band 5° across bent round a point south of it
    let ring: Vec<LonLat> = (0..=24)
        .map(|i| 25.0 + 100.0 * i as f64 / 24.0)
        .map(|a: f64| p(32.0 * a.to_radians().cos(), -20.0 + 32.0 * a.to_radians().sin()))
        .chain((0..=24).rev().map(|i| 25.0 + 100.0 * i as f64 / 24.0).map(|a: f64| p(27.0 * a.to_radians().cos(), -20.0 + 27.0 * a.to_radians().sin())))
        .collect();
    let (d, e) = country(&ring, "弯月国");
    let path = d.label_path(e).unwrap();
    assert!(sag(&path) > 1.0, "bent upwards with the arch: {:.2}", sag(&path));
    assert!(chord_angle(&path).abs() < 10.0, "read across the arch: {:.1}°", chord_angle(&path));
    name_inside(&d, e, &path);
}

#[test]
fn every_country_of_the_sample_world_has_its_name_inside() {
    let d = map_core::sample::demo_world();
    for e in d.countries() {
        let Some(path) = d.label_path(e) else { continue };
        let name = &d.entities[&e].name;
        assert_eq!(path.em, name_em(name));
        assert_eq!(d.owner.get(&d.topo.face_at(path.middle())), Some(&e), "{name}: middle outside");
        assert!(path.pts.first().unwrap()[0] < path.pts.last().unwrap()[0], "{name}: read west to east");
        name_inside(&d, e, &path);
    }
}

/// A ring along a parallel, zigzagging a little.
fn parallel(lat: f64, step: f64, east: bool) -> Vec<LonLat> {
    let n = (360.0 / step) as usize;
    let mut v: Vec<LonLat> = (0..n).map(|i| p(-180.0 + step * i as f64 + 1.0, lat + if i % 2 == 0 { 0.0 } else { 0.7 })).collect();
    if !east {
        v.reverse();
    }
    v
}

#[test]
fn a_country_round_a_pole_is_named_level_by_the_pole() {
    let (mut d, _) = overlay::apply(&Document::default(), &parallel(70.0, 10.0, true), DrawIntent::AddLand).unwrap();
    let e = d.add_entity(EntityKind::Country, "极国");
    let (d, _) = overlay::apply(&d, &parallel(80.0, 20.0, false), DrawIntent::Expand(e)).unwrap();
    let path = d.label_path(e).unwrap();
    assert!(path.middle().lat > 82.0, "{:?}", path.middle());
    assert!(chord_angle(&path).abs() < 1e-6);
    assert_eq!(d.owner.get(&d.topo.face_at(path.middle())), Some(&e));
}
