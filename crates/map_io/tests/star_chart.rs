use map_core::{climate, star_chart::StarChart, Document};
use std::sync::Arc;

#[test]
fn star_chart_roundtrip_and_live_home_planet() {
    let mut doc = Document::new("海蓝星", 7400.0);
    let mut chart = StarChart::default();
    chart.galaxy.name = "长夜".into();
    chart.planets[0].orbit.au = 0.36;
    chart.validate().unwrap();
    assert_eq!(chart.planets.len(), 7);
    let (_, home) = chart.resolved(&doc);
    assert_eq!(home.name, "海蓝星");
    assert!((home.radii_earth - 7400.0 / 6371.0088).abs() < 1e-6);
    assert_eq!(home.orbit.au, 1.0);

    let mut project = climate::Project::default();
    project.settings.planet.linked = false;
    project.settings.planet[climate::Param::Distance] = 2.14;
    project.settings.planet[climate::Param::Period] = 421.0;
    project.settings.planet.locked_rotation = true;
    doc.climate = Some(Arc::new(project));
    let (_, home) = chart.resolved(&doc);
    assert_eq!(home.orbit.au, 2.14);
    assert_eq!(home.orbit.days, 421.0);
    assert!(home.synchronous);

    doc.star_chart = Some(Arc::new(chart.clone()));
    let bytes = map_io::project::to_bytes(&doc).unwrap();
    assert_eq!(&bytes[8..12], &3u32.to_le_bytes());
    let read = map_io::project::from_bytes(&bytes).unwrap();
    assert_eq!(read.star_chart.as_deref(), Some(&chart));
    assert_eq!(read.climate, doc.climate);
}

#[test]
fn elliptic_orbit_closes_and_has_expected_apsides() {
    let mut orbit = StarChart::default().home.orbit;
    orbit.phase = 0.0;
    orbit.eccentricity = 0.6;
    orbit.inclination = 24.0;

    let at = orbit.position(0.0);
    let half = orbit.position(orbit.days / 2.0);
    assert!((at[0] - orbit.au * 0.4).abs() < 1e-9);
    assert!((half[0] + orbit.au * 1.6).abs() < 1e-9);
    assert_eq!(at, orbit.position(orbit.days));
    assert!(orbit.position(orbit.days / 4.0)[1] > 0.0);
}

#[test]
fn custom_satellite_roundtrips_and_format_3_gets_the_default_moon() {
    let mut chart = StarChart::default();
    let mut doc = Document {
        star_chart: Some(Arc::new(chart.clone())),
        ..Default::default()
    };
    let version_three = map_io::project::to_bytes(&doc).unwrap();
    let legacy = map_io::project::from_bytes(&version_three).unwrap();
    assert_eq!(&version_three[8..12], &3u32.to_le_bytes());
    assert_eq!(legacy.star_chart.unwrap().moon, chart.moon);

    chart.moon.name = "弦月".into();
    chart.moon.radius_km = 1300.0;
    chart.moon.orbit.days = 31.0;
    chart.moon.orbit.au = 530_000.0 / map_core::star_chart::AU_KM;
    chart.validate().unwrap();
    doc.star_chart = Some(Arc::new(chart.clone()));
    let updated = map_io::project::to_bytes(&doc).unwrap();
    assert_eq!(&updated[8..12], &4u32.to_le_bytes());
    assert_eq!(
        map_io::project::from_bytes(&updated)
            .unwrap()
            .star_chart
            .unwrap()
            .moon,
        chart.moon
    );
}

#[test]
fn simulation_and_three_stars_roundtrip_in_the_extended_project() {
    use map_core::star_chart::{Companion, GalaxyStar, Moon};
    let mut chart = StarChart::default();
    chart.gravity = true;
    chart.companions.push(Companion {
        star: chart.star.clone(),
        orbit: chart.planets[4].orbit.clone(),
    });
    chart.companions.push(Companion {
        star: chart.star.clone(),
        orbit: chart.planets[6].orbit.clone(),
    });
    chart.satellites.push(Moon {
        name: "Observation satellite".into(),
        category: "Artificial".into(),
        mass_kg: Some(21000.0),
        ..Moon::default()
    });
    chart.galaxy.stars.push(GalaxyStar {
        name: "Pillar".into(),
        position_kly: [18.0, 0.2, 4.0],
        temperature: 8700.0,
        category: "Star".into(),
        model: None,
    });
    let doc = Document {
        star_chart: Some(Arc::new(chart.clone())),
        ..Default::default()
    };
    let bytes = map_io::project::to_bytes(&doc).unwrap();
    assert_eq!(&bytes[8..12], &7u32.to_le_bytes());
    assert_eq!(
        map_io::project::from_bytes(&bytes)
            .unwrap()
            .star_chart
            .as_deref(),
        Some(&chart)
    );
}

#[test]
fn a_shared_model_is_saved_with_its_links_in_each_part_of_the_star_chart() {
    use map_core::{
        star_chart::{BodyKey, Companion, GalaxyStar, Moon},
        underlay::Blob,
        world_book::{Attachment, Link, Node, WorldBook, ROOT},
    };
    let mut doc = Document::default();
    let book = WorldBook::edit(&mut doc);
    let asset = book.add_asset(Attachment {
        name: "probe.obj".into(),
        bytes: Blob(Arc::new(b"v 0 0 0\nv 1 0 0\nv 0 1 0\nf 1 2 3\n".to_vec())),
    });
    book.add_link(Link::new(Node::Map(ROOT), Node::Asset(asset), false))
        .unwrap();
    let mut chart = StarChart::default();
    chart.companions.push(Companion {
        star: chart.star.clone(),
        orbit: chart.planets[2].orbit.clone(),
    });
    chart.satellites.push(Moon::default());
    chart.galaxy.stars.push(GalaxyStar {
        name: "Distant system".into(),
        position_kly: [12., 1., 5.],
        temperature: 6000.0,
        category: "Star".into(),
        model: None,
    });
    for key in [
        BodyKey::Star,
        BodyKey::Home,
        BodyKey::Planet(0),
        BodyKey::Companion(0),
        BodyKey::Moon,
        BodyKey::Satellite(0),
        BodyKey::GalaxyStar(0),
    ] {
        *chart.model_slot(key).unwrap() = Some(asset);
    }
    doc.star_chart = Some(Arc::new(chart));
    let stored = map_io::project::from_bytes(&map_io::project::to_bytes(&doc).unwrap()).unwrap();
    assert_eq!(
        stored
            .star_chart
            .as_deref()
            .unwrap()
            .model_ids()
            .collect::<Vec<_>>(),
        vec![asset; 7]
    );
    let book = stored.book.unwrap();
    assert!(book
        .links
        .values()
        .any(|l| l.from == Node::Map(ROOT) && l.to == Node::Asset(asset)));
    assert_eq!(
        map_io::model::decode(&book.assets[&asset])
            .unwrap()
            .triangles
            .len(),
        1
    );
}
