//! Name styles tied to units: a culture given a style lends it to the
//! cities that stand in it and the countries mostly covered by it; a unit's
//! own style comes first; a style goes with its unit.

use map_core::doc::{LayerKind, FULL_SHARE, POLITICAL};
use map_core::names::NameStyle;
use map_core::sample::grid_world;
use map_core::{EntityKind, FaceKind};

#[test]
fn a_culture_lends_its_style() {
    let mut d = grid_world(6, 4, 2, 3);
    // a country of two faces (another country's face given to it)
    let cs = d.countries();
    let country = cs[0];
    let other = d.faces_of(cs[1])[0];
    d.assign(other, Some(country)).unwrap();
    let faces = d.faces_of(country);
    assert_eq!(faces.len(), 2);
    // a culture over all of the country but one face, another over that one
    let culture = d.add_layer_of("文化", LayerKind::Shares);
    d.set_active_layer(culture).unwrap();
    let (east, west) = (d.add_entity(EntityKind::Country, "东方文化"), d.add_entity(EntityKind::Country, "西方文化"));
    for (k, f) in faces.iter().enumerate() {
        d.set_share(*f, Some(if k == 0 { west } else { east }), FULL_SHARE).unwrap();
    }
    d.set_active_layer(POLITICAL).unwrap();
    assert_eq!(d.name_style_for(country), None, "no culture has a style yet");
    d.set_name_style(east, Some(NameStyle::Huaxia)).unwrap();
    d.set_name_style(west, Some(NameStyle::Latin)).unwrap();
    // the country: the culture over most of its land
    let area = |f| d.topo.face_area(f, d.radius());
    let west_area = area(faces[0]);
    let east_area: f64 = faces[1..].iter().map(|f| area(*f)).sum();
    let want = if east_area > west_area { NameStyle::Huaxia } else { NameStyle::Latin };
    assert_eq!(d.name_style_for(country), Some(want));
    // a city: the culture where it stands
    let p = |f| {
        let (outer, holes) = d.topo.face_planar(f);
        let (q, _) = map_core::geom::interior_point(&outer, &holes).unwrap();
        map_core::LonLat::new(q[0], q[1])
    };
    let (pw, pe) = (p(faces[0]), p(faces[1]));
    let (cw, ce) = (d.add_city(pw, "西城"), d.add_city(pe, "东城"));
    assert_eq!(d.topo.faces[&d.topo.face_at(pw)].kind, FaceKind::Land);
    assert_eq!((d.name_style_for(cw), d.name_style_for(ce)), (Some(NameStyle::Latin), Some(NameStyle::Huaxia)));
    // a style of its own comes first
    d.set_name_style(cw, Some(NameStyle::Nordic)).unwrap();
    assert_eq!(d.name_style_for(cw), Some(NameStyle::Nordic));
    // gone with the unit (saving: map_io/tests/name_styles.rs)
    d.delete_entity(west);
    assert_eq!(d.name_style_for(ce), Some(NameStyle::Huaxia));
    assert!(!d.name_styles.contains_key(&west));
}
