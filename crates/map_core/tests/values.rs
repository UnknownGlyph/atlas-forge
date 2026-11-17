//! Data values through time: undated base values, per-field inheritance
//! policies, and where every value comes from (ADR-7, settled 2026-10-29).

use map_core::doc::{Document, FieldKind, Inherit, Series, Source, Value};
use map_core::EntityKind;
use std::collections::BTreeMap;

fn series(base: Option<f64>, recs: &[(i32, f64)]) -> Series {
    Series { base: base.map(Value::Num), by_year: recs.iter().map(|(y, v)| (*y, Value::Num(*v))).collect::<BTreeMap<_, _>>() }
}

fn num(r: (Option<Value>, Source)) -> (Option<f64>, Source) {
    (r.0.and_then(|v| v.as_num()), r.1)
}

#[test]
fn every_policy_and_source() {
    // base 500 (undated), records 1900 = 120 and 1950 = 0 (a real zero)
    let s = series(Some(500.0), &[(1900, 120.0), (1950, 0.0)]);
    let at = |y: Option<i32>, p: Inherit, by: Option<i32>| num(s.resolve(y, p, by));
    // exact records win under every policy; zero is a value, not unknown
    for p in Inherit::ALL {
        assert_eq!(at(Some(1900), p, None), (Some(120.0), Source::Recorded(1900)));
        assert_eq!(at(Some(1950), p, None), (Some(0.0), Source::Recorded(1950)));
    }
    // no year: the base value as it is (plain maps)
    assert_eq!(at(None, Inherit::Strict, None), (Some(500.0), Source::Base));
    // strict
    assert_eq!(at(Some(1920), Inherit::Strict, None), (None, Source::Unknown));
    // nearest earlier record, never the undated base
    assert_eq!(at(Some(1920), Inherit::Nearest, None), (Some(120.0), Source::Inherited(1900)));
    assert_eq!(at(Some(1800), Inherit::Nearest, None), (None, Source::Unknown));
    assert_eq!(at(Some(2000), Inherit::Nearest, None), (Some(0.0), Source::Inherited(1950)));
    // base fallback
    assert_eq!(at(Some(1920), Inherit::BaseFallback, None), (Some(500.0), Source::Base));
    // interpolation between records, unknown outside them
    assert_eq!(at(Some(1925), Inherit::Interpolate, None), (Some(60.0), Source::Interpolated(1900, 1950)));
    assert_eq!(at(Some(1980), Inherit::Interpolate, None), (None, Source::Unknown));
    // a base value with a year counts as a record of that year only
    let (v, src) = at(Some(2010), Inherit::Interpolate, Some(2020));
    assert_eq!(src, Source::Interpolated(1950, 2020));
    assert!((v.unwrap() - 500.0 * 60.0 / 70.0).abs() < 1e-9);
    assert_eq!(at(Some(2020), Inherit::Strict, Some(2020)), (Some(500.0), Source::Base));
    assert_eq!(at(Some(2030), Inherit::Strict, Some(2020)), (None, Source::Unknown), "not applied to every later year");
    assert_eq!(at(Some(2030), Inherit::Nearest, Some(2020)), (Some(500.0), Source::Inherited(2020)));
    // a real record of the base year wins over the base value
    let s2 = series(Some(500.0), &[(2020, 480.0)]);
    assert_eq!(num(s2.resolve(Some(2020), Inherit::Strict, Some(2020))), (Some(480.0), Source::Recorded(2020)));
    // only dated records, no year selected: the latest one, marked
    let s3 = series(None, &[(1900, 1.0), (1950, 2.0)]);
    assert_eq!(num(s3.resolve(None, Inherit::Strict, None)), (Some(2.0), Source::Inherited(1950)));
}

#[test]
fn defaults_by_kind_and_editing_only_touches_the_selected_year() {
    let mut d = Document::default();
    let e = d.add_entity(EntityKind::Country, "甲");
    let pop = d.add_field("pop", "人口", FieldKind::Number, None).unwrap();
    let motto = d.add_field("motto", "格言", FieldKind::Text, None).unwrap();
    assert_eq!(d.fields[&pop].policy(), Inherit::Strict, "statistics are strict");
    assert_eq!(d.fields[&motto].policy(), Inherit::BaseFallback, "static attributes inherit the base");
    d.set_value(e, pop, None, Some(Value::Num(500.0)));
    d.set_value(e, motto, None, Some(Value::Text("统一".into())));
    d.set_value(e, pop, Some(1900), Some(Value::Num(120.0)));
    d.year = Some(1920);
    assert_eq!(d.value_with_source(e, pop), (None, Source::Unknown));
    assert_eq!(d.value_with_source(e, motto), (Some(Value::Text("统一".into())), Source::Base));
    // what editing sees and changes: only the selected year's own record
    assert_eq!(d.stored_value(e, pop), None);
    d.set_value(e, pop, Some(1920), Some(Value::Num(200.0)));
    assert_eq!(d.value_with_source(e, pop), (Some(Value::Num(200.0)), Source::Recorded(1920)));
    d.year = Some(1900);
    assert_eq!(d.raw_value(e, pop), Some(Value::Num(120.0)), "earlier records untouched");
    d.year = None;
    assert_eq!(d.stored_value(e, pop), Some(Value::Num(500.0)));
    assert_eq!(d.source_of(e, "pop"), Source::Base);
    assert_eq!(d.source_of(e, "area_km2"), Source::Derived);
}

#[test]
fn the_demo_world_interpolates_population_to_its_2020_base() {
    let mut d = map_core::sample::demo_world();
    let pop = d.field_by_key("population").unwrap();
    let c = d.countries().into_iter().find(|c| d.stored_value(*c, pop).is_some()).expect("a country with population");
    d.year = Some(2020);
    let (now, s) = d.value_with_source(c, pop);
    assert_eq!(s, Source::Base);
    d.year = Some(1900);
    let then = d.raw_value(c, pop).unwrap().as_num().unwrap();
    d.year = Some(1960);
    let (mid, s) = d.value_with_source(c, pop);
    assert_eq!(s, Source::Interpolated(1900, 2020));
    let now = now.unwrap().as_num().unwrap();
    assert!((mid.unwrap().as_num().unwrap() - (then + (now - then) * 0.5)).abs() < 1e-6);
    // the legend says how values were obtained
    d.theme.mode = map_core::theme::ThemeMode::Choropleth;
    d.theme.field_key = "population".into();
    let t = map_core::theme::evaluate(&d);
    assert!(t.title.contains("1960 年") && t.title.contains("插值"), "{}", t.title);
}

/// X owns two squares; the eastern one passes to Y in 1930. Population is
/// recorded for X in 1900 and 1960.
fn bordered() -> (Document, [map_core::EntityId; 2], [map_core::FaceId; 2], f64, f64) {
    use map_core::geom::LonLat;
    let mut d = Document::default();
    let sq = |x: f64| vec![LonLat::new(x, 0.0), LonLat::new(x + 10.0, 0.0), LonLat::new(x + 10.0, 10.0), LonLat::new(x, 10.0)];
    let a = d.add_ring_face(&sq(0.0), map_core::FaceKind::Land).unwrap();
    let b = d.add_ring_face(&sq(20.0), map_core::FaceKind::Land).unwrap();
    let (x, y) = (d.add_entity(EntityKind::Country, "X"), d.add_entity(EntityKind::Country, "Y"));
    d.assign(a, Some(x)).unwrap();
    d.assign(b, Some(x)).unwrap();
    d.set_year(Some(1930));
    d.assign(b, Some(y)).unwrap();
    d.set_year(None);
    let pop = d.add_field("pop", "人口", FieldKind::Number, None).unwrap();
    d.add_field("density", "密度", FieldKind::Number, Some("pop / area_km2".into())).unwrap();
    d.set_value(x, pop, Some(1900), Some(Value::Num(1000.0)));
    d.set_value(x, pop, Some(1960), Some(Value::Num(3000.0)));
    let (aa, ab) = (d.topo.face_area(a, d.radius()), d.topo.face_area(b, d.radius()));
    (d, [x, y], [a, b], aa, ab)
}

#[test]
fn area_statistics_use_the_borders_of_the_values_year() {
    use map_core::stats::{Borders, Stats};
    let (mut d, [x, _], _, aa, ab) = bordered();
    let pop = d.field_by_key("pop").unwrap();
    d.fields.get_mut(&pop).unwrap().inherit = Some(Inherit::Nearest);
    // 1950: population carried over from 1900, divided by X's 1900 area (both squares)
    d.set_year(Some(1950));
    let st = Stats::new(&d).stat(x, "density");
    assert_eq!(st.borders, Borders::Year(1900));
    assert!((st.value.unwrap() - 1000.0 / (aa + ab)).abs() < 1e-12);
    assert_eq!(st.tag(), "derived:borders-1900");
    // the plain area is still the selected year's
    assert!((Stats::new(&d).value(x, "area_km2").unwrap() - aa).abs() < 1e-6);
    // 1960: recorded that year, that year's borders (one square)
    d.set_year(Some(1960));
    let st = Stats::new(&d).stat(x, "density");
    assert_eq!(st.borders, Borders::Selected);
    assert!((st.value.unwrap() - 3000.0 / aa).abs() < 1e-12);
    // interpolated values use the selected year's borders
    d.fields.get_mut(&pop).unwrap().inherit = Some(Inherit::Interpolate);
    d.set_year(Some(1930));
    let st = Stats::new(&d).stat(x, "density");
    assert_eq!(st.borders, Borders::Selected);
    assert!((st.value.unwrap() - 2000.0 / aa).abs() < 1e-9);
    // num_value agrees with the shared context
    let areas = d.areas_by_entity();
    assert_eq!(d.num_value(x, "density", &areas), st.value);
}

#[test]
fn inputs_from_different_years_are_mixed_and_base_years_count() {
    use map_core::stats::{Borders, Stats};
    let (mut d, [x, _], _, aa, ab) = bordered();
    let pop = d.field_by_key("pop").unwrap();
    d.fields.get_mut(&pop).unwrap().inherit = Some(Inherit::Nearest);
    let gdp = d.add_field("gdp", "GDP", FieldKind::Number, None).unwrap();
    d.fields.get_mut(&gdp).unwrap().inherit = Some(Inherit::Nearest);
    d.set_value(x, gdp, Some(1920), Some(Value::Num(50.0)));
    d.add_field("both", "合计密度", FieldKind::Number, Some("(pop + gdp) / area_km2".into())).unwrap();
    d.set_year(Some(1950));
    let st = Stats::new(&d).stat(x, "both");
    assert_eq!(st.borders, Borders::Mixed, "1900 and 1920 inputs");
    assert!((st.value.unwrap() - 1050.0 / aa).abs() < 1e-9, "mixed: the selected year's area");
    assert!(st.tag().ends_with(":mixed"));
    // a base value that is the 2020 figure: 2020 borders
    let area2 = d.add_field("land", "土地", FieldKind::Number, None).unwrap();
    {
        let f = d.fields.get_mut(&area2).unwrap();
        f.base_year = Some(1925);
        f.inherit = Some(Inherit::BaseFallback);
    }
    d.set_value(x, area2, None, Some(Value::Num(7.0)));
    d.add_field("land_share", "土地占比", FieldKind::Number, Some("land / area_km2".into())).unwrap();
    let st = Stats::new(&d).stat(x, "land_share");
    assert_eq!(st.borders, Borders::Year(1925));
    assert!((st.value.unwrap() - 7.0 / (aa + ab)).abs() < 1e-12);
    // the map legend says so
    d.theme.mode = map_core::theme::ThemeMode::Choropleth;
    d.theme.field_key = "land_share".into();
    let t = map_core::theme::evaluate(&d);
    assert!(t.title.contains("按数据所属年份的边界"), "{}", t.title);
}
