//! The list of drawn features.

use map_core::area::AreaMark;
use map_core::doc::{Annotation, Document, LineFeature, LineId};
use map_core::features::{matches, FeatureRef};
use map_core::geom::LonLat;
use map_core::symbol::{MapSymbol, SymbolKind};

fn ll(lon: f64, lat: f64) -> LonLat {
    LonLat::new(lon, lat)
}

/// Every kind is listed, in kind order then by name, with its years and
/// a point on or in it; the filter matches names, sub-kinds, kinds and
/// years.
#[test]
fn drawn_features_are_listed() {
    let mut d = Document::default();
    let mut id = || LineId(d.topo.alloc());
    let (t, s1, s2, r, v, a) = (id(), id(), id(), id(), id(), id());
    d.annotations.insert(t, Annotation::new("南方大洋", ll(0.0, -40.0)));
    let mut b = MapSymbol::new(SymbolKind::Battle, ll(10.0, 10.0));
    b.name = "赤壁".into();
    b.from = Some(208);
    b.to = Some(208);
    d.symbols.insert(s2, b);
    d.symbols.insert(s1, MapSymbol::new(SymbolKind::Volcano, ll(20.0, 20.0)));
    d.rivers.insert(r, LineFeature { name: "北伐路线".into(), pts: vec![ll(0.0, 0.0), ll(10.0, 0.0), ll(20.0, 0.0)], route: Some(Default::default()), legend: String::new(), show_name: false, hidden: false });
    d.rivers.insert(v, LineFeature { name: "长河".into(), pts: vec![ll(0.0, 5.0), ll(10.0, 5.0)], route: None, legend: String::new(), show_name: false, hidden: false });
    let mut area = AreaMark::new(vec![ll(-10.0, -10.0), ll(10.0, -10.0), ll(10.0, 10.0), ll(-10.0, 10.0)]);
    area.name = "争议区".into();
    area.to = Some(1900);
    d.areas.insert(a, area);

    let f = d.features();
    let order: Vec<FeatureRef> = f.iter().map(|x| x.what).collect();
    assert_eq!(order, vec![FeatureRef::Text(t), FeatureRef::Symbol(s1), FeatureRef::Symbol(s2), FeatureRef::Route(r), FeatureRef::River(v), FeatureRef::Area(a)]);
    // the unnamed volcano sorts before the named battle; its kind is the detail
    assert_eq!((f[1].name.as_str(), f[1].detail.as_str()), ("", "火山"));
    assert_eq!(f[2].years, "208 年");
    assert_eq!(f[5].years, "至 1900 年");
    // a line's point is its middle, an area's its centre
    assert!((f[3].at.lon - 10.0).abs() < 1e-6 && f[3].at.lat.abs() < 1e-6);
    assert!(f[5].at.lon.abs() < 1e-6 && f[5].at.lat.abs() < 1e-6);
    let hits = |q: &str| f.iter().filter(|x| matches(x, q)).count();
    assert_eq!((hits(""), hits("北伐"), hits("火山"), hits("符号"), hits("1900"), hits("无此")), (6, 1, 1, 2, 1, 0));
}
