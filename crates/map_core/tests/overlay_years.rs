//! Border changes drawn with a year selected are spliced locally; the
//! result must be the same map, in every year and every layer, as
//! rebuilding the whole world.

use map_core::doc::{Document, EntityId, POLITICAL};
use map_core::geom::LonLat;
use map_core::overlay::{self, DrawIntent};
use map_core::sample::Rng;
use map_core::testworld::{history_world, TestWorldSpec};
use std::collections::HashMap;

/// Areas per entity in `year`, in every layer, and all land (under the
/// background's id, which no entity has).
fn areas(d: &Document, year: Option<i32>) -> Vec<HashMap<EntityId, f64>> {
    let mut d = d.clone();
    d.set_year(year);
    let r = d.radius();
    let land: f64 = d.topo.faces.iter().filter(|(f, x)| **f != map_core::BACKGROUND && x.kind == map_core::FaceKind::Land).map(|(f, _)| d.topo.face_area(*f, r)).sum();
    let mut out: Vec<HashMap<EntityId, f64>> = d
        .layer_ids()
        .into_iter()
        .map(|l| {
            d.set_active_layer(l).unwrap();
            d.areas_by_entity()
        })
        .collect();
    out.push(HashMap::from([(EntityId(0), land)]));
    out
}

fn same(a: &[HashMap<EntityId, f64>], b: &[HashMap<EntityId, f64>], what: &str) {
    for (x, y) in a.iter().zip(b) {
        let keys: std::collections::BTreeSet<_> = x.keys().chain(y.keys()).collect();
        for k in keys {
            let (p, q) = (x.get(k).copied().unwrap_or(0.0), y.get(k).copied().unwrap_or(0.0));
            assert!((p - q).abs() <= 1e-6 * p.abs().max(q.abs()).max(1.0), "{what}: entity {k:?} {p} vs {q}");
        }
    }
}

#[test]
fn timed_border_changes_splice_like_a_full_rebuild() {
    let (mut d, _) = history_world(&TestWorldSpec { nx: 6, ny: 4, events: 80, ..Default::default() });
    d.set_active_layer(POLITICAL).unwrap();
    let mut rng = Rng::new(5);
    let (mut local, mut tried) = (0, 0);
    for _ in 0..40 {
        let y = rng.range(1000.0, 2000.0) as i32;
        d.set_year(Some(y));
        let borders: Vec<(LonLat, EntityId, EntityId)> = d
            .topo
            .edges
            .values()
            .filter(|e| !e.pts.is_empty())
            .filter_map(|e| {
                let (a, b) = (d.owner.get(&e.left)?, d.owner.get(&e.right)?);
                (a != b).then(|| (e.pts[e.pts.len() / 2], *a, *b))
            })
            .collect();
        let (p, a, b) = borders[(rng.next_f64() * borders.len() as f64) as usize % borders.len()];
        let r = rng.range(1.0, 3.0);
        let ring: Vec<LonLat> = (0..20).map(|i| {
            let t = i as f64 / 20.0 * std::f64::consts::TAU;
            LonLat::new(p.lon + r * 1.3 * t.cos(), p.lat + r * t.sin())
        }).collect();
        let intent = if rng.next_f64() < 0.5 { DrawIntent::Take(a) } else { DrawIntent::Expand(b) };
        let Ok((x, rep)) = overlay::apply(&d, &ring, intent) else { continue };
        let (w, _) = overlay::apply_whole_world(&d, &ring, intent).unwrap();
        tried += 1;
        local += rep.local as usize;
        if !rep.local {
            eprintln!("fallback in {y}: {:?}", rep.fallback);
        }
        x.topo.validate().unwrap();
        assert_eq!(x.change_years(), w.change_years(), "{y}");
        for yy in [None, Some(y - 1), Some(y), Some(y + 1), Some(1000), Some(1999)] {
            same(&areas(&x, yy), &areas(&w, yy), &format!("drawn in {y}, viewed in {yy:?}"));
        }
        d = x;
    }
    assert!(tried >= 25, "{tried}");
    assert!(local * 10 >= tried * 8, "most timed border changes are local: {local} of {tried}");
}

/// Coasts too: new land on the sea, land growing into the sea, lakes
/// carved and borders taken near the coast, all with a year selected.
#[test]
fn timed_coast_changes_splice_like_a_full_rebuild() {
    let (mut d, _) = history_world(&TestWorldSpec { nx: 6, ny: 4, events: 80, seed: 3, ..Default::default() });
    d.set_active_layer(POLITICAL).unwrap();
    let mut rng = Rng::new(9);
    let (mut local, mut tried) = (0, 0);
    let mut fell_back = Vec::new();
    for k in 0..40 {
        let y = rng.range(1000.0, 2000.0) as i32;
        d.set_year(Some(y));
        // a point on a coast, and the owner of the land there
        let water = |f| d.topo.faces.get(&f).is_some_and(|x| x.kind == map_core::FaceKind::Water);
        let coasts: Vec<(LonLat, Option<EntityId>)> = d
            .topo
            .edges
            .values()
            .filter(|e| !e.pts.is_empty() && water(e.left) != water(e.right))
            .map(|e| (e.pts[e.pts.len() / 2], d.owner.get(&if water(e.left) { e.right } else { e.left }).copied()))
            .collect();
        let (p, owner) = coasts[(rng.next_f64() * coasts.len() as f64) as usize % coasts.len()];
        let intent = match (k % 4, owner) {
            (0, _) => DrawIntent::AddLand,
            (1, Some(o)) => DrawIntent::Expand(o),
            (2, _) => DrawIntent::CarveWater,
            (_, Some(o)) => DrawIntent::Take(o),
            _ => DrawIntent::AddLand,
        };
        let r = rng.range(0.8, 2.5);
        let ring: Vec<LonLat> = (0..20)
            .map(|i| {
                let t = i as f64 / 20.0 * std::f64::consts::TAU;
                LonLat::new(p.lon + r * 1.3 * t.cos(), p.lat + r * t.sin())
            })
            .collect();
        let Ok((x, rep)) = overlay::apply(&d, &ring, intent) else { continue };
        // the full rebuild must work too (it once failed at the antimeridian,
        // where the boolean kernel left seam points a hair off ±180°)
        let (w, _) = overlay::apply_whole_world(&d, &ring, intent).unwrap_or_else(|e| panic!("full rebuild, {intent:?} in {y}: {e}"));
        tried += 1;
        local += rep.local as usize;
        if !rep.local {
            fell_back.push(format!("{intent:?} in {y} at {:.2},{:.2}: {:?}", p.lon, p.lat, rep.fallback));
        }
        x.topo.validate().unwrap();
        assert_eq!(x.change_years(), w.change_years(), "{intent:?} in {y}");
        for yy in [None, Some(y - 1), Some(y), Some(y + 1), Some(1000), Some(1999)] {
            same(&areas(&x, yy), &areas(&w, yy), &format!("{intent:?} drawn in {y}, viewed in {yy:?}"));
        }
        d = x;
    }
    eprintln!("coast: {local} of {tried} local; fallbacks: {fell_back:?}");
    assert!(tried >= 25, "{tried}");
    assert!(local * 10 >= tried * 8, "most timed coast changes are local: {local} of {tried}: {fell_back:?}");
}
