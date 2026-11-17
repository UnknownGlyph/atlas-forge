//! Roads and travel: a road network between the cities of a world, over
//! the land only, written as routes in one undo step; travel times; the
//! shortest way between two cities along the routes.

use map_core::geom::LonLat;
use map_core::history::History;
use map_core::roads::{days, format_days, RoadSpec, MODES};
use map_core::topo::FaceKind;
use map_core::worldgen::{generate, WorldSpec};
use map_core::Document;

fn world() -> Document {
    generate(&WorldSpec { seed: 21, countries: 8, rivers: 4, land: 0.3, continents: 2, ..WorldSpec::default() }).unwrap().0
}

fn on_land(d: &Document, p: LonLat) -> bool {
    let f = d.topo.face_at(p);
    d.topo.faces.get(&f).is_some_and(|x| x.kind == FaceKind::Land)
}

#[test]
fn a_road_network_over_the_land() {
    let mut d = world();
    let cities = d.entities.values().filter(|x| x.kind == map_core::EntityKind::City).count();
    let plan = d.road_plan(&RoadSpec::default()).unwrap();
    assert_eq!(plan.cities, cities);
    assert!(plan.roads.len() >= cities / 2, "{} roads for {cities} cities", plan.roads.len());
    for road in &plan.roads {
        // from one city's place to the other's
        let pos = |e| d.entities[&e].pos.unwrap();
        assert_eq!((road.pts[0], *road.pts.last().unwrap()), (pos(road.a), pos(road.b)));
        // over the land: points between the ends on land (a smoothed bend
        // may cut a coast's corner by a little)
        let inner = &road.pts[1..road.pts.len() - 1];
        let wet = inner.iter().filter(|p| !on_land(&d, **p)).count();
        assert!(wet * 10 <= inner.len().max(1), "{} of {} points of a road in the water", wet, inner.len());
        // never shorter than the straight way
        let straight = map_core::measure::great_circle(pos(road.a), pos(road.b), d.radius());
        assert!(road.km >= straight * 0.999, "{} km against {straight} km", road.km);
    }
    // written as routes, one undo step
    let before = d.clone();
    let mut h = History::default();
    let mut ids = vec![];
    h.apply(&mut d, "道路", |d| {
        ids = d.apply_roads(&plan)?;
        Ok(())
    })
    .unwrap();
    assert_eq!(ids.len(), plan.roads.len());
    assert!(ids.iter().all(|id| d.rivers[id].route.is_some() && d.rivers[id].legend == "道路"));
    // the way between the two ends of the longest road, along the routes
    let long = plan.roads.iter().max_by(|a, b| a.km.total_cmp(&b.km)).unwrap();
    let (way, km) = d.travel_way(long.a, long.b).unwrap();
    assert!(!way.is_empty() && km <= long.km + 1e-6, "{km} km against the road's {}", long.km);
    h.undo(&mut d);
    assert!(d == before);
}

/// Two islands: no road crosses the sea between them.
#[test]
fn no_road_over_the_sea() {
    let mut d = Document::default();
    let sq = |x: f64| vec![LonLat::new(x, 0.0), LonLat::new(x + 10.0, 0.0), LonLat::new(x + 10.0, 10.0), LonLat::new(x, 10.0)];
    d.add_ring_face(&sq(0.0), FaceKind::Land).unwrap();
    d.add_ring_face(&sq(20.0), FaceKind::Land).unwrap();
    let a = d.add_city(LonLat::new(3.0, 5.0), "甲");
    let b = d.add_city(LonLat::new(7.0, 5.0), "乙");
    let c = d.add_city(LonLat::new(25.0, 5.0), "丙");
    let plan = d.road_plan(&RoadSpec::default()).unwrap();
    assert_eq!(plan.roads.len(), 1);
    let r = &plan.roads[0];
    assert_eq!((r.a.min(r.b), r.a.max(r.b)), (a.min(b), a.max(b)));
    assert_eq!(plan.alone, vec![c]);
    // a lake between two cities: the road goes round it
    let mut d = Document::default();
    d.add_ring_face(&[LonLat::new(0.0, 0.0), LonLat::new(30.0, 0.0), LonLat::new(30.0, 20.0), LonLat::new(0.0, 20.0)], FaceKind::Land).unwrap();
    d = map_core::overlay::apply(&d, &[LonLat::new(12.0, 3.0), LonLat::new(18.0, 3.0), LonLat::new(18.0, 17.0), LonLat::new(12.0, 17.0)], map_core::overlay::DrawIntent::CarveWater).unwrap().0;
    let (a, b) = (d.add_city(LonLat::new(5.0, 10.0), "东"), d.add_city(LonLat::new(25.0, 10.0), "西"));
    let plan = d.road_plan(&RoadSpec::default()).unwrap();
    assert_eq!(plan.roads.len(), 1);
    let r = &plan.roads[0];
    assert!(r.pts.iter().all(|p| !(p.lon > 12.2 && p.lon < 17.8 && p.lat > 3.2 && p.lat < 16.8)), "the road crosses the lake");
    let straight = map_core::measure::great_circle(d.entities[&a].pos.unwrap(), d.entities[&b].pos.unwrap(), d.radius());
    assert!(r.km > straight * 1.1, "the way round the lake is longer: {} against {straight}", r.km);
}

#[test]
fn travel_times() {
    assert_eq!(MODES[0].name, "步行");
    assert_eq!(format_days(days(300.0, &MODES[0])), "约 10 天");
    assert!(days(600.0, &MODES[2]) < days(600.0, &MODES[0]));
}
