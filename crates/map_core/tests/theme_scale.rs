//! Thematic classes shared by all years: a colour means the same value in
//! every year, so years can be compared and per-year exports match.

use map_core::doc::{Document, FieldKind, Value};
use map_core::geom::LonLat;
use map_core::theme::{self, ClassMethod, ClassScale, ThemeMode};
use map_core::{EntityKind, FaceKind};

/// X and Y each own a square; population is recorded for both in 1900
/// (X 100, Y 10) and 1950 (X 1000, Y 500). The eastern square passes from
/// Y to X in 1930.
fn world() -> (Document, [map_core::EntityId; 2]) {
    let mut d = Document::default();
    let sq = |x: f64| vec![LonLat::new(x, 0.0), LonLat::new(x + 10.0, 0.0), LonLat::new(x + 10.0, 10.0), LonLat::new(x, 10.0)];
    let a = d.add_ring_face(&sq(0.0), FaceKind::Land).unwrap();
    let b = d.add_ring_face(&sq(20.0), FaceKind::Land).unwrap();
    let c = d.add_ring_face(&sq(40.0), FaceKind::Land).unwrap();
    let (x, y) = (d.add_entity(EntityKind::Country, "X"), d.add_entity(EntityKind::Country, "Y"));
    d.assign(a, Some(x)).unwrap();
    d.assign(b, Some(y)).unwrap();
    d.assign(c, Some(y)).unwrap();
    d.set_year(Some(1930));
    d.assign(c, Some(x)).unwrap();
    d.set_year(None);
    let pop = d.add_field("pop", "人口", FieldKind::Number, None).unwrap();
    for (e, y0, y1) in [(x, 100.0, 1000.0), (y, 10.0, 500.0)] {
        d.set_value(e, pop, Some(1900), Some(Value::Num(y0)));
        d.set_value(e, pop, Some(1950), Some(Value::Num(y1)));
    }
    d.theme.mode = ThemeMode::Choropleth;
    d.theme.field_key = "pop".into();
    d.theme.method = ClassMethod::EqualInterval;
    d.theme.classes = 2;
    (d, [x, y])
}

#[test]
fn per_year_classes_stretch_each_year() {
    let (mut d, [x, y]) = world();
    assert!(theme::fixed_scale(&d).is_none(), "per year: no shared scale");
    for yr in [1900, 1950] {
        d.set_year(Some(yr));
        let t = theme::evaluate(&d);
        assert_ne!(t.entity_colors[&x], t.entity_colors[&y], "{yr}: X high, Y low");
    }
}

#[test]
fn shared_classes_mean_the_same_value_in_every_year() {
    let (mut d, [x, y]) = world();
    d.theme.scale = ClassScale::AllYears;
    // the year before the first change, the change and the record years
    assert_eq!(theme::scale_years(&d), vec![1900, 1929, 1930, 1950]);
    let f = theme::fixed_scale(&d).unwrap();
    // equal interval over 10..1000 in two classes
    assert_eq!((f.lo, f.hi, f.breaks.clone()), (10.0, 1000.0, vec![505.0]));
    d.set_year(Some(1900));
    let t1900 = theme::evaluate(&d);
    // 100 and 10 are both in the low class in 1900
    assert_eq!(t1900.entity_colors[&x], t1900.entity_colors[&y]);
    assert!(t1900.title.contains("1900 年") && t1900.title.contains("各年统一分级 1900–1950 年"), "{}", t1900.title);
    d.set_year(Some(1950));
    let t1950 = theme::evaluate(&d);
    // 500 stays low, 1000 is high: Y keeps the colour it had in 1900
    assert_eq!(t1950.entity_colors[&y], t1900.entity_colors[&y]);
    assert_ne!(t1950.entity_colors[&x], t1950.entity_colors[&y]);
    // the legend is the same in both years: it covers every year's values
    assert_eq!(t1900.legend, t1950.legend);
    assert_eq!(t1950.legend[0].label, "10 – 505");
    // the same as with the scale worked out once (per-year exports)
    assert_eq!(theme::evaluate_with(&d, Some(&f)).entity_colors, t1950.entity_colors);
}

/// A derived field's scale follows the years of the records it is made of.
#[test]
fn a_derived_fields_years_come_from_its_inputs() {
    let (mut d, _) = world();
    d.add_field("density", "密度", FieldKind::Number, Some("pop / area_km2".into())).unwrap();
    d.theme.field_key = "density".into();
    d.theme.scale = ClassScale::AllYears;
    assert!(theme::scale_years(&d).contains(&1950));
    assert!(theme::fixed_scale(&d).is_some());
}

/// A map without years has nothing to share across: the classes are the
/// usual ones, and old projects (no `scale` saved) read as per year.
#[test]
fn maps_without_years_and_old_projects() {
    let (mut d, [x, _]) = world();
    let pop = d.field_by_key("pop").unwrap();
    d.values.clear();
    d.owner_changes.clear();
    d.refresh_time_view();
    d.set_value(x, pop, None, Some(Value::Num(5.0)));
    d.theme.scale = ClassScale::AllYears;
    assert!(theme::scale_years(&d).is_empty() && theme::fixed_scale(&d).is_none());
    assert!(!theme::evaluate(&d).title.contains("统一"));
    let mut v = serde_json::to_value(&d.theme).unwrap();
    v.as_object_mut().unwrap().remove("scale");
    let back: theme::ThemeSpec = serde_json::from_value(v).unwrap();
    assert_eq!(back.scale, ClassScale::PerYear);
}
