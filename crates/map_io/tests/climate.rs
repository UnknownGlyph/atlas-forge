use map_core::climate::*;
use map_core::{Document, FaceKind, LonLat};
use std::sync::Arc;

fn blank() -> Vec<Ground> {
    vec![Ground::default(); W * H]
}

#[test]
fn displayed_scenario_is_exported_and_climate_files_require_a_capable_reader() {
    let mut d = Document::default();
    let bytes = map_io::project::to_bytes(&d).unwrap();
    assert_eq!(&bytes[8..12], &1u32.to_le_bytes());
    let p = Project {
        current: Some(Arc::new(run(preset(0), &blank()))),
        saved: vec![("冰原".into(), Arc::new(run(preset(4), &blank())))],
        ..Default::default()
    };
    d.climate = Some(Arc::new(p));
    let bytes = map_io::project::to_bytes(&d).unwrap();
    assert_eq!(&bytes[8..12], &2u32.to_le_bytes());
    let export = |d: &Document| {
        map_io::scene::build_scene(
            d,
            &map_core::theme::evaluate(d),
            &map_io::scene::ExportOptions {
                width: 240,
                height: 120,
                ..Default::default()
            },
        )
    };
    let natural = export(&d);
    Arc::make_mut(d.climate.as_mut().unwrap()).selected = Some(0);
    let frozen = export(&d);
    assert!(frozen.shade.is_some());
    assert_ne!(natural.shade, frozen.shade);
    Arc::make_mut(d.climate.as_mut().unwrap()).visible = false;
    let hidden = export(&d);
    assert!(hidden.shade.is_none());
    assert_ne!(
        map_io::png::render(&frozen, None).unwrap().data(),
        map_io::png::render(&hidden, None).unwrap().data()
    );
    assert!(map_io::svg::render(&frozen).contains("data:image/png;base64,"));
    assert!(
        String::from_utf8_lossy(&map_io::pdf::render(&frozen, None)).contains("/Subtype /Image")
    );
}
fn run(s: Settings, g: &[Ground]) -> Scenario {
    Scenario::new(s, None, Region::default(), g).unwrap()
}
fn at(lon: f64, lat: f64) -> usize {
    index(LonLat::new(lon, lat))
}
#[test]
fn blank_is_zonal_and_the_two_hemispheres_have_opposite_seasons() {
    let g = sample(&Document::default(), &Default::default(), Surface::Unknown);
    assert!(g.iter().all(|v| v.surface == Surface::Unknown));
    let s = run(preset(0), &g);
    for f in Field::ALL {
        assert_eq!(
            s.value(at(-90., 15.), f, Period::Annual),
            s.value(at(45., 15.), f, Period::Annual)
        );
    }
    assert!(
        s.value(at(0., 0.), Field::Temperature, Period::Annual)
            > s.value(at(0., 80.), Field::Temperature, Period::Annual) + 15.
    );
    assert!(s.cells[at(0., 45.)].months[7].temp > s.cells[at(0., 45.)].months[1].temp);
    assert!(s.cells[at(0., -45.)].months[7].temp < s.cells[at(0., -45.)].months[1].temp);
    let summary = s.summary(s.region);
    println!("Earth-like reference: {summary:?}");
    assert!((5.0..22.).contains(&summary[0]));
}
#[test]
fn alien_presets_respond_to_rotation_radiation_and_water() {
    let g = blank();
    let locked = run(preset(7), &g);
    assert!(
        locked.value(at(0., 0.), Field::Temperature, Period::Annual)
            > locked.value(at(175., 0.), Field::Temperature, Period::Annual) + 50.
    );
    let arid = run(preset(5), &g);
    let ocean = run(preset(6), &g);
    assert!(ocean.summary(ocean.region)[1] > arid.summary(arid.region)[1] * 2.);
    let mut retro = preset(0);
    retro.planet.retrograde = true;
    assert!(
        run(retro, &g).cells[at(0., 0.)].months[0].east
            * run(preset(0), &g).cells[at(0., 0.)].months[0].east
            < 0.
    );
    let ice = run(preset(4), &g);
    assert!(ice.summary(ice.region)[2] > 0.6);
}
#[test]
fn fixed_gravity_flux_and_conflicts() {
    use Param::*;
    let mut p = preset(1).planet;
    p[Radius] = 1.5;
    let out = p.resolve().unwrap();
    assert!((out[Mass] - 2.25).abs() < 1e-8);
    assert_eq!(out[Gravity], p[Gravity]);
    p.controls[Flux as usize] = Control::Fixed;
    p.controls[Lum as usize] = Control::Manual;
    p[Lum] = 4.;
    let out = p.resolve().unwrap();
    assert!((out[Distance] - 2.).abs() < 1e-8);
    assert_eq!(out[Flux], 1361.);
    p.controls[Distance as usize] = Control::Fixed;
    assert!(p.resolve().is_err());
    assert_eq!(p[Distance], 1.);
    p.linked = false;
    assert_eq!(p.resolve().unwrap(), p);
}
#[test]
fn mapped_features_coasts_through_time_and_dateline() {
    let mut d = Document::default();
    d.set_year(Some(1500));
    let ring = [
        LonLat::new(165., -12.),
        LonLat::new(-165., -12.),
        LonLat::new(-165., 12.),
        LonLat::new(165., 12.),
    ];
    d.add_ring_face(&ring, FaceKind::Land).unwrap();
    for (y, expected) in [(1499, Surface::Water), (1500, Surface::Land)] {
        d.set_year(Some(y));
        let g = sample(&d, &Default::default(), Surface::Unknown);
        for x in [-175., 175.] {
            assert_eq!(g[at(x, 0.)].surface, expected);
        }
        assert_eq!(g[at(0., 0.)].surface, Surface::Unknown);
    }
}
#[test]
fn mountains_cool_and_suppress_leeward_rain() {
    let mut g = vec![
        Ground {
            surface: Surface::Land,
            ..Ground::default()
        };
        W * H
    ];
    let s = run(preset(0), &g);
    let x = at(0., 2.) % W;
    for row in 0..H {
        g[row * W + x].height = 3000.;
    }
    let mountain = run(preset(0), &g);
    let ridge = at(0., 2.);
    assert!(mountain.cells[ridge].months[0].temp < s.cells[ridge].months[0].temp - 15.);
    // Equatorial easterlies: the western cell is downwind.
    assert!(
        mountain.cells[ridge - 1].months[0].rain < mountain.cells[ridge + 1].months[0].rain * 0.65
    );
}

#[test]
fn inland_lakes_and_open_seas_have_aquatic_ecosystems() {
    let mut d = Document::default();
    let square = |extent: f64| {
        [
            LonLat::new(-extent, -extent),
            LonLat::new(extent, -extent),
            LonLat::new(extent, extent),
            LonLat::new(-extent, extent),
        ]
    };
    d.add_ring_face(&square(30.), FaceKind::Land).unwrap();
    d.add_ring_face(&square(9.), FaceKind::Water).unwrap();
    let g = sample(&d, &Default::default(), Surface::Water);
    let i = at(2., 2.);
    assert_eq!(g[i].surface, Surface::Lake);
    let s = run(preset(0), &g);
    assert_eq!(s.value(i, Field::Vegetation, Period::Annual), 8.);
    assert_eq!(s.value(at(70., 2.), Field::Vegetation, Period::Annual), 7.);
}
#[test]
fn authored_and_frozen_fields_survive_reruns_and_project_io() {
    let mut old = run(preset(0), &blank());
    let i = at(1., 1.);
    old.paint(&[i], Field::Temperature, Period::Month(1), Some(35.), true);
    old.paint(&[i], Field::Rain, Period::Annual, Some(300.), false);
    let natural_ice = old.value(i, Field::Ice, Period::Annual);
    old.paint(&[i], Field::Ice, Period::Annual, None, true);
    let mut next = run(preset(4), &blank());
    next.retain_locks(&old);
    assert_eq!(next.value(i, Field::Temperature, Period::Month(1)), 35.);
    assert_eq!(next.value(i, Field::Ice, Period::Annual), natural_ice);
    assert!(!next.edits[&(i, Field::Ice)].authored);
    assert!(!next.edits.contains_key(&(i, Field::Rain)));
    let p = Project {
        current: Some(Arc::new(next)),
        saved: vec![("初稿".into(), Arc::new(old))],
        ..Project::default()
    };
    let d = Document {
        climate: Some(Arc::new(p.clone())),
        ..Document::default()
    };
    let bytes = map_io::project::to_bytes(&d).unwrap();
    let back = map_io::project::from_bytes(&bytes).unwrap();
    assert_eq!(back.climate.as_deref(), Some(&p));
    let original = include_bytes!("fixtures/v1_world.atlas");
    assert!(map_io::project::from_bytes(original)
        .unwrap()
        .climate
        .is_none());
}
#[test]
fn regional_raster_masks_the_rest_and_csv_uses_orbital_months() {
    let region = Region {
        west: 170.,
        east: -170.,
        south: -5.,
        north: 5.,
    };
    let s = Scenario::new(preset(0), None, region, &blank()).unwrap();
    let (w, h, raster) = s.raster(Field::Wind, Period::Annual);
    assert_eq!(raster.len(), w * h * 4);
    let pixel = |i: usize| raster[4 * ((i / W * 8 + 4) * w + (i % W) * 8 + 4) + 3];
    assert_eq!(pixel(at(0., 0.)), 0);
    assert_eq!(pixel(at(175., 0.)), 255);
    assert_eq!(
        s.csv().lines().count(),
        1 + region_cells(region).count() * 12
    );
    let i = at(175., 0.);
    let rain = s.value(i, Field::Rain, Period::Annual);
    let seasons: f32 = (0..4)
        .map(|k| s.value(i, Field::Rain, Period::Season(k)))
        .sum();
    assert!((seasons - rain).abs() < 0.01);
}
#[test]
fn target_search_improves_temperature_and_preserves_manual_values() {
    let mut settings = preset(0);
    settings.planet.controls.fill(Control::Manual);
    settings.planet.controls[Param::Greenhouse as usize] = Control::Auto;
    settings.goals.temperature = Some(23.);
    let g = blank();
    let baseline = run(settings.clone(), &g).summary(Region::default())[0];
    let (found, metrics) = fit(&settings, &g, Region::default(), None).unwrap();
    assert!((metrics[0] - 23.).abs() < (baseline - 23.).abs());
    for &p in PARAMS.iter().filter(|&&p| p != Param::Greenhouse) {
        assert_eq!(settings.planet[p], found.planet[p]);
    }
}
