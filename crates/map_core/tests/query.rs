//! Select by attribute: kinds, a word in the name or legend, layer, style,
//! year and hidden — each narrows the result, and an empty query matches all.

use map_core::area::AreaMark;
use map_core::doc::{Annotation, Document, LineFeature, LineId, RouteStyle};
use map_core::features::{FeatureQuery, FeatureRef};
use map_core::styles::Look;
use map_core::symbol::{MapSymbol, SymbolKind};
use map_core::LonLat;

fn p(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

#[test]
fn queries_narrow_the_features() {
    let mut d = Document::default();
    let mut id = || LineId(d.topo.alloc());
    let (r1, r2, rv, ar, tx, sy) = (id(), id(), id(), id(), id(), id());
    d.rivers.insert(r1, LineFeature { name: "北伐".into(), pts: vec![p(0.0, 0.0), p(5.0, 5.0)], route: Some(RouteStyle::default()), legend: "进军路线".into(), show_name: false, hidden: false });
    d.rivers.insert(r2, LineFeature { name: "商路".into(), pts: vec![p(0.0, 10.0), p(5.0, 15.0)], route: Some(RouteStyle::default()), legend: String::new(), show_name: false, hidden: true });
    d.rivers.insert(rv, LineFeature { name: "大河".into(), pts: vec![p(10.0, 0.0), p(15.0, 5.0)], route: None, legend: String::new(), show_name: false, hidden: false });
    let mut area = AreaMark::new(vec![p(20.0, 0.0), p(25.0, 0.0), p(25.0, 5.0)]);
    (area.legend, area.from, area.to) = ("占领区".into(), Some(1914), Some(1918));
    d.areas.insert(ar, area);
    d.annotations.insert(tx, Annotation::new("北海", p(0.0, -10.0)));
    let mut sym = MapSymbol::new(SymbolKind::Battle, p(10.0, -10.0));
    sym.name = "北原会战".into();
    d.symbols.insert(sy, sym);
    let all = d.query_features(&FeatureQuery::default());
    assert_eq!(all.len(), 6, "an empty query matches every feature");
    // kinds
    let routes = d.query_features(&FeatureQuery { kinds: [false, false, true, false, false], ..Default::default() });
    assert!(routes.len() == 2 && routes.contains(&FeatureRef::Route(r1)) && routes.contains(&FeatureRef::Route(r2)));
    // a word in the name or the legend
    let north = d.query_features(&FeatureQuery { text: "北".into(), ..Default::default() });
    assert!(north.contains(&FeatureRef::Route(r1)) && north.contains(&FeatureRef::Text(tx)) && north.contains(&FeatureRef::Symbol(sy)) && north.len() == 3);
    assert_eq!(d.query_features(&FeatureQuery { text: "进军".into(), ..Default::default() }), vec![FeatureRef::Route(r1)], "by legend");
    // a year: the area only in its years; features without years always
    let y1900 = d.query_features(&FeatureQuery { year: Some(1900), ..Default::default() });
    assert!(!y1900.contains(&FeatureRef::Area(ar)) && y1900.len() == 5);
    assert!(d.query_features(&FeatureQuery { year: Some(1915), ..Default::default() }).contains(&FeatureRef::Area(ar)));
    // hidden
    assert_eq!(d.query_features(&FeatureQuery { hidden: Some(true), ..Default::default() }), vec![FeatureRef::Route(r2)]);
    // layers and styles
    let g = d.add_group("战争");
    d.set_group(&[FeatureRef::Route(r1), FeatureRef::Symbol(sy)], Some(g)).unwrap();
    let in_war = d.query_features(&FeatureQuery { group: Some(Some(g)), ..Default::default() });
    assert_eq!(in_war.len(), 2);
    assert_eq!(d.query_features(&FeatureQuery { group: Some(None), ..Default::default() }).len(), 4);
    let s = d.add_style("航线", Look::Route(RouteStyle::default()));
    d.link_style(&[FeatureRef::Route(r2)], s).unwrap();
    assert_eq!(d.query_features(&FeatureQuery { style: Some(Some(s)), ..Default::default() }), vec![FeatureRef::Route(r2)]);
    // everything together
    let q = FeatureQuery { kinds: [false, true, true, false, false], text: "北".into(), group: Some(Some(g)), hidden: Some(false), ..Default::default() };
    let both = d.query_features(&q);
    assert!(both.len() == 2 && both.contains(&FeatureRef::Route(r1)) && both.contains(&FeatureRef::Symbol(sy)));
}
