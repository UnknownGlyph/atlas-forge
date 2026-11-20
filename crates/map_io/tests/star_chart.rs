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
