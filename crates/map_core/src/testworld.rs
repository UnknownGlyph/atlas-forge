//! A large test world with a long, layered history, for checking features
//! together and measuring how the program behaves as history piles up.
//!
//! It starts from a [`grid_world`] continent and then lives through
//! `events` years of history made with the same operations an author uses:
//! borders redrawn by taking land from a neighbour, coasts that grow and
//! lakes that appear, countries that become independent or end (handing
//! their land to a successor), population recorded every 50 years. A
//! culture layer with its own history, a religion share layer, cities,
//! rivers, routes, sea names, battle symbols and hatched areas complete it.
//! Everything is deterministic for a given spec.

use crate::doc::{Document, EntityId, EntityKind, Inherit, LayerKind, LineFeature, LineId, Value, FULL_SHARE, POLITICAL};
use crate::geom::LonLat;
use crate::overlay::{self, DrawIntent};
use crate::sample::{blob, grid_world, Rng};
use crate::topo::{FaceKind, BACKGROUND};
use std::time::Instant;

#[derive(Clone, Debug)]
pub struct TestWorldSpec {
    /// Starting countries: `nx * ny` on one continent.
    pub nx: usize,
    pub ny: usize,
    /// Vertices per starting border.
    pub detail: usize,
    /// Border and coast changes over the years.
    pub events: usize,
    pub from: i32,
    pub to: i32,
    pub seed: u64,
    /// When set, every border and coast is made finer and rougher after the
    /// history, down to segments of about this length (km): detailed data
    /// for measuring large maps. See [`refine_borders`].
    pub refine_km: Option<f64>,
}

impl Default for TestWorldSpec {
    fn default() -> Self {
        Self { nx: 10, ny: 6, detail: 6, events: 240, from: 1000, to: 2000, seed: 11, refine_km: None }
    }
}

impl TestWorldSpec {
    /// The default world with borders refined to about 3 km: some 400 000
    /// vertices and a million triangles.
    pub fn detailed() -> Self {
        Self { refine_km: Some(3.0), ..Default::default() }
    }
}

/// What making the world did, with timings.
#[derive(Clone, Debug, Default)]
pub struct TestWorldReport {
    pub faces_start: usize,
    pub faces_end: usize,
    pub events_applied: usize,
    /// Drawings the overlay refused (a blob in an awkward place); skipped.
    pub events_failed: usize,
    /// Of the applied drawings, how many were rebuilt locally.
    pub local: usize,
    pub founded: usize,
    pub ended: usize,
    pub coast_changes: usize,
    pub culture_events: usize,
    /// Total time and the time spent in drawings.
    pub total_ms: f64,
    pub overlay_ms: f64,
    pub slowest_overlay_ms: f64,
    /// Border refinement, when asked for.
    pub refine: Option<RefineReport>,
}

#[derive(Clone, Debug, Default)]
pub struct RefineReport {
    pub vertices_before: usize,
    pub vertices_after: usize,
    pub edges: usize,
    /// Edges made less rough, or only densified, to avoid crossings.
    pub calmer: usize,
    pub straight: usize,
    pub ms: f64,
}

/// Make every edge finer and rougher: each segment is halved, the midpoint
/// pushed sideways by up to `rough` times the segment's length (segments
/// shorter than 2° only, so the large-scale shape stays), until the
/// segments are about `seg_km` long (a fractal line, as real borders and
/// coasts look at close range). An edge whose new shape would cross
/// another is tried calmer, and at worst only densified along its old line,
/// which cannot cross anything. Nodes stay; the geometry is that of every
/// year (records are untouched).
pub fn refine_borders(doc: &mut Document, seg_km: f64, rough: f64, seed: u64) -> RefineReport {
    let t0 = Instant::now();
    let mut rng = Rng::new(seed);
    let verts = |d: &Document| d.topo.edges.values().map(|e| e.pts.len()).sum::<usize>() + d.topo.nodes.len();
    let mut rep = RefineReport { vertices_before: verts(doc), ..Default::default() };
    let step = seg_km / (doc.radius() * std::f64::consts::PI / 180.0);
    let ids: Vec<crate::topo::EdgeId> = doc.topo.edges.keys().copied().collect();
    rep.edges = ids.len();
    for id in ids {
        let full = doc.topo.edge_full(&doc.topo.edges[&id]);
        for (k, r) in [rough, rough / 3.0, 0.0].into_iter().enumerate() {
            let pts = fractal(&mut rng, &full, step, r);
            let backup = doc.topo.clone();
            if doc.topo.replace_edge_points(id, pts).is_ok() {
                rep.calmer += (k == 1) as usize;
                rep.straight += (k == 2) as usize;
                break;
            }
            doc.topo = backup;
        }
    }
    rep.vertices_after = verts(doc);
    rep.ms = t0.elapsed().as_secs_f64() * 1e3;
    rep
}

/// The interior points of `full` subdivided down to `step` degrees, each
/// new midpoint pushed sideways by up to `rough` of its segment.
fn fractal(rng: &mut Rng, full: &[LonLat], step: f64, rough: f64) -> Vec<LonLat> {
    fn split(rng: &mut Rng, a: [f64; 2], b: [f64; 2], step: f64, rough: f64, out: &mut Vec<LonLat>) {
        let k = ((a[1] + b[1]) * 0.5).to_radians().cos().max(0.05);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * k).hypot(dy);
        if len <= step {
            return;
        }
        // long segments are only split: the large-scale shape stays, detail
        // appears below a couple of degrees (some 200 km)
        let off = if len > 2.0 { 0.0 } else { rough * rng.range(-1.0, 1.0) };
        // perpendicular in the local (east·cos lat, north) frame
        let m = [(a[0] + b[0]) * 0.5 - dy * off / k, (a[1] + b[1]) * 0.5 + dx * k * off];
        split(rng, a, m, step, rough, out);
        out.push(LonLat::new(m[0], m[1]));
        split(rng, m, b, step, rough, out);
    }
    let mut out = Vec::new();
    // longitudes continuous along the edge (it may cross the antimeridian)
    let mut prev = [full[0].lon, full[0].lat];
    for (i, q) in full.iter().enumerate().skip(1) {
        let cur = [prev[0] + crate::geom::lon_delta(prev[0], q.lon), q.lat];
        split(rng, prev, cur, step, rough, &mut out);
        if i + 1 < full.len() {
            out.push(LonLat::new(cur[0], cur[1]));
        }
        prev = cur;
    }
    out
}

/// Two land faces owned by different (existing) owners in the selected
/// year that meet along an edge: a point on that border and both owners.
fn borders(doc: &Document) -> Vec<(LonLat, EntityId, EntityId)> {
    doc.topo
        .edges
        .values()
        .filter(|e| e.left != BACKGROUND && e.right != BACKGROUND && !e.pts.is_empty())
        .filter_map(|e| {
            let (a, b) = (doc.owner.get(&e.left)?, doc.owner.get(&e.right)?);
            (a != b).then(|| (e.pts[e.pts.len() / 2], *a, *b))
        })
        .collect()
}

/// Land meeting water: a point on the coast and the land's owner.
fn coasts(doc: &Document) -> Vec<(LonLat, EntityId)> {
    doc.topo
        .edges
        .values()
        .filter(|e| !e.pts.is_empty())
        .filter_map(|e| {
            let water = |f| doc.topo.faces.get(&f).is_some_and(|x| x.kind == FaceKind::Water);
            let land = if water(e.left) { e.right } else if water(e.right) { e.left } else { return None };
            Some((e.pts[e.pts.len() / 2], *doc.owner.get(&land)?))
        })
        .collect()
}

fn pick<T: Clone>(rng: &mut Rng, v: &[T]) -> Option<T> {
    (!v.is_empty()).then(|| v[((rng.next_f64() * v.len() as f64) as usize).min(v.len() - 1)].clone())
}

/// Draw `ring` with `intent` into the document; false when refused.
fn draw(doc: &mut Document, ring: &[LonLat], intent: DrawIntent, rep: &mut TestWorldReport) -> bool {
    let t = Instant::now();
    let r = overlay::apply(doc, ring, intent);
    let ms = t.elapsed().as_secs_f64() * 1e3;
    rep.overlay_ms += ms;
    rep.slowest_overlay_ms = rep.slowest_overlay_ms.max(ms);
    match r {
        Ok((d, o)) => {
            *doc = d;
            rep.events_applied += 1;
            rep.local += o.local as usize;
            true
        }
        Err(_) => {
            rep.events_failed += 1;
            false
        }
    }
}

/// The centre of a face's outline (mean of its outer ring), in lon/lat.
fn face_centre(doc: &Document, f: crate::topo::FaceId) -> Option<LonLat> {
    let (outer, _) = doc.topo.face_planar(f);
    if outer.is_empty() {
        return None;
    }
    let n = outer.len() as f64;
    Some(LonLat::new(outer.iter().map(|p| p[0]).sum::<f64>() / n, outer.iter().map(|p| p[1]).sum::<f64>() / n))
}

/// A political history over `from`..`to`: `events` changes in random
/// years — borders move (one side takes a piece), countries break away
/// (named by `new_name(n)`) or end (their land to a neighbour), coasts
/// grow and lakes appear. Every change is a drawing in its year, so
/// earlier years stay as they were. Returns some of the border clashes
/// (place and year), for battle symbols.
pub fn random_history(doc: &mut Document, from: i32, to: i32, events: usize, rng: &mut Rng, rep: &mut TestWorldReport, new_name: &dyn Fn(usize) -> String) -> Vec<(LonLat, i32)> {
    let mut years: Vec<i32> = (0..events).map(|_| rng.range(from as f64, to as f64) as i32).collect();
    years.sort();
    let mut battles = Vec::new();
    for (k, y) in years.iter().copied().enumerate() {
        doc.set_year(Some(y));
        let roll = rng.next_f64();
        if roll < 0.08 {
            // a coast grows, or a lake appears
            if rng.next_f64() < 0.6 {
                if let Some((p, owner)) = pick(rng, &coasts(doc)) {
                    let r = rng.range(1.5, 3.5);
                    if draw(doc, &blob(rng, p, r * 1.3, r, 24, 0.2), DrawIntent::Expand(owner), rep) {
                        rep.coast_changes += 1;
                    }
                }
            } else if let Some((p, _, _)) = pick(rng, &borders(doc)) {
                let c = LonLat::new(p.lon + rng.range(-2.0, 2.0), p.lat + rng.range(-2.0, 2.0));
                if draw(doc, &blob(rng, c, 1.2, 0.9, 20, 0.15), DrawIntent::CarveWater, rep) {
                    rep.coast_changes += 1;
                }
            }
        } else if roll < 0.12 {
            // a new country breaks away
            if let Some((p, _, _)) = pick(rng, &borders(doc)) {
                let n = doc.countries().len() + 1;
                let e = doc.add_entity(EntityKind::Country, &new_name(n));
                doc.set_lifespan(e, Some(y), None).unwrap();
                let r = rng.range(3.0, 6.0);
                if draw(doc, &blob(rng, p, r * 1.2, r, 28, 0.25), DrawIntent::Take(e), rep) {
                    rep.founded += 1;
                } else {
                    doc.delete_entity(e);
                }
            }
        } else if roll < 0.15 && doc.countries().len() > 4 {
            // a country ends; its land goes to a neighbour from the next year
            if let Some((_, a, b)) = pick(rng, &borders(doc)) {
                let founded = doc.entities[&a].founded;
                if founded.is_none_or(|f| f < y) && doc.set_lifespan(a, founded, Some(y)).is_ok() {
                    doc.successors_set(a, vec![b]);
                    if doc.reassign_after_end(a, Some(b)).is_ok() {
                        rep.ended += 1;
                    }
                }
            }
        } else if let Some((p, a, b)) = pick(rng, &borders(doc)) {
            // a border moves: one side takes a piece from the other
            let win = if rng.next_f64() < 0.5 { a } else { b };
            let r = rng.range(1.5, 4.5);
            if draw(doc, &blob(rng, p, r * 1.4, r, 24, 0.25), DrawIntent::Take(win), rep) && k % 6 == 0 {
                battles.push((p, y));
            }
        }
    }
    doc.set_year(None);
    battles
}

pub fn history_world(spec: &TestWorldSpec) -> (Document, TestWorldReport) {
    let t0 = Instant::now();
    let mut rng = Rng::new(spec.seed);
    let mut doc = grid_world(spec.nx, spec.ny, spec.detail, spec.seed);
    doc.meta.name = format!("测试世界 {}×{} · {} 次变化", spec.nx, spec.ny, spec.events);
    let mut rep = TestWorldReport { faces_start: doc.topo.faces.len() - 1, ..Default::default() };
    // islands across the antimeridian, each its own country
    for (i, c) in [LonLat::new(172.0, 10.0), LonLat::new(-172.0, -25.0), LonLat::new(178.0, 45.0)].into_iter().enumerate() {
        let ring = blob(&mut rng, c, 6.0, 4.0, 48, 0.2);
        if let Ok(f) = doc.add_ring_face(&ring, FaceKind::Land) {
            let e = doc.add_entity(EntityKind::Country, &format!("岛国{}", i + 1));
            doc.assign(f, Some(e)).unwrap();
        }
    }

    // ---- political history, oldest first
    let battles = random_history(&mut doc, spec.from, spec.to, spec.events, &mut rng, &mut rep, &|n| format!("新国{n}"));

    // ---- data: population every 50 years while a country exists
    let pop = doc.field_by_key("population").unwrap();
    doc.fields.get_mut(&pop).unwrap().inherit = Some(Inherit::Interpolate);
    let step = 50;
    for e in doc.countries() {
        let x = doc.entities[&e].clone();
        let (a, b) = (x.founded.unwrap_or(spec.from).max(spec.from), x.ended.unwrap_or(spec.to).min(spec.to));
        let start = rng.range(2e5, 3e6);
        let growth = rng.range(1.001, 1.004);
        let mut y = a - a.rem_euclid(step) + if a % step == 0 { 0 } else { step };
        while y <= b {
            let v = (start * growth.powi(y - spec.from)).round();
            doc.set_value(e, pop, Some(y), Some(Value::Num(v)));
            y += step;
        }
    }

    // ---- cities, one per four starting countries
    let land: Vec<_> = doc.topo.faces.iter().filter(|(id, f)| **id != BACKGROUND && f.kind == FaceKind::Land).map(|(id, _)| *id).collect();
    let mut cities = Vec::new();
    for (i, f) in land.iter().enumerate().filter(|(i, _)| i % 4 == 0).take(spec.nx * spec.ny / 4 + 1) {
        if let Some(c) = face_centre(&doc, *f) {
            if doc.topo.face_at(c) == *f {
                cities.push((doc.add_city(c, &format!("城{}", i + 1)), *f));
            }
        }
    }
    // a city belongs to whoever holds its land, year by year
    let mut ys: Vec<i32> = doc.change_years().into_iter().collect();
    ys.insert(0, spec.from - 1);
    for y in ys {
        doc.set_year(Some(y));
        for (c, f) in &cities {
            let want = doc.owner.get(f).copied();
            if doc.entities[c].parent != want {
                let _ = doc.record_parent(*c, want);
            }
        }
    }
    doc.set_year(None);

    // ---- culture layer: bands across the continent, with its own changes
    let culture = doc.add_layer("文化区");
    doc.set_active_layer(culture).unwrap();
    let bands = (spec.nx / 2).max(3);
    let cultures: Vec<EntityId> = (0..bands).map(|i| doc.add_entity(EntityKind::Country, &format!("文化{}", i + 1))).collect();
    for f in &land {
        if let Some(c) = face_centre(&doc, *f) {
            let i = (((c.lon + 150.0) / 300.0 * bands as f64) as usize).min(bands - 1);
            let _ = doc.assign(*f, Some(cultures[i]));
        }
    }
    for _ in 0..spec.events / 6 {
        let y = rng.range(spec.from as f64, spec.to as f64) as i32;
        doc.set_year(Some(y));
        if let Some((p, a, _)) = pick(&mut rng, &borders(&doc)) {
            let r = rng.range(2.0, 5.0);
            if draw(&mut doc, &blob(&mut rng, p, r * 1.5, r, 24, 0.3), DrawIntent::Take(a), &mut rep) {
                rep.culture_events += 1;
            }
        }
    }
    doc.set_year(None);

    // ---- religion: a share layer, two faiths mixing, a third arriving
    let faith = doc.add_layer_of("宗教", LayerKind::Shares);
    doc.set_active_layer(faith).unwrap();
    let f: Vec<EntityId> = ["东教", "西教", "新教"].iter().map(|n| doc.add_entity(EntityKind::Country, n)).collect();
    let land_now: Vec<_> = doc.topo.faces.iter().filter(|(id, x)| **id != BACKGROUND && x.kind == FaceKind::Land).map(|(id, _)| *id).collect();
    for face in &land_now {
        let Some(c) = face_centre(&doc, *face) else { continue };
        let east = (((c.lon + 150.0) / 300.0).clamp(0.0, 1.0) * FULL_SHARE as f64) as u16;
        let _ = doc.set_share(*face, Some(f[0]), east.max(1));
        let _ = doc.set_share(*face, Some(f[1]), (FULL_SHARE - east).max(1));
    }
    doc.set_year(Some((spec.from + spec.to) / 2 + 100));
    for face in land_now.iter().step_by(3) {
        let _ = doc.set_share(*face, Some(f[2]), FULL_SHARE / 3);
    }
    doc.set_year(None);
    doc.set_active_layer(POLITICAL).unwrap();

    // ---- layout features
    for i in 0..(spec.nx / 2).max(2) {
        let x0 = -140.0 + 280.0 * i as f64 / (spec.nx / 2).max(2) as f64;
        let pts: Vec<LonLat> = (0..30).map(|k| LonLat::new(x0 + k as f64 * 0.6 + (k as f64 * 0.7).sin() * 1.2, 65.0 - k as f64 * 3.5)).collect();
        doc.rivers.insert(LineId(doc.topo.alloc()), LineFeature { name: format!("河{}", i + 1), pts, route: None, legend: String::new(), show_name: true, hidden: false });
    }
    let route: Vec<LonLat> = (0..12).map(|k| LonLat::new(-130.0 + k as f64 * 22.0, -40.0 + (k as f64).sin() * 8.0)).collect();
    doc.rivers.insert(
        LineId(doc.topo.alloc()),
        LineFeature { name: "商路".into(), pts: route, route: Some(Default::default()), legend: "商路".into(), show_name: false, hidden: false },
    );
    for (text, at) in [("北洋", LonLat::new(0.0, 80.0)), ("南洋", LonLat::new(0.0, -72.0)), ("东海", LonLat::new(162.0, -5.0)), ("西海", LonLat::new(-165.0, 20.0))] {
        doc.annotations.insert(LineId(doc.topo.alloc()), crate::doc::Annotation::new(text, at));
    }
    for (p, y) in battles {
        let mut s = crate::symbol::MapSymbol::new(crate::symbol::SymbolKind::Battle, p);
        s.name = format!("{y} 年之战");
        s.from = Some(y);
        s.to = Some(y + 30);
        doc.symbols.insert(LineId(doc.topo.alloc()), s);
    }
    for k in 0..4 {
        let c = LonLat::new(-120.0 + k as f64 * 70.0, 10.0 + (k as f64) * 5.0);
        let mut a = crate::area::AreaMark::new(blob(&mut rng, c, 8.0, 5.0, 30, 0.2));
        a.name = format!("争议区{}", k + 1);
        a.from = Some(spec.from + 200 * k as i32);
        a.to = Some(spec.from + 200 * k as i32 + 150);
        doc.areas.insert(LineId(doc.topo.alloc()), a);
    }
    doc.theme.field_key = "population".into();
    if let Some(km) = spec.refine_km {
        rep.refine = Some(refine_borders(&mut doc, km, 0.18, spec.seed ^ 0x5eed));
    }
    rep.faces_end = doc.topo.faces.len() - 1;
    rep.total_ms = t0.elapsed().as_secs_f64() * 1e3;
    (doc, rep)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_history_world_is_valid_and_rich() {
        let spec = TestWorldSpec { nx: 5, ny: 3, events: 60, ..Default::default() };
        let (mut d, rep) = history_world(&spec);
        d.topo.validate().unwrap();
        assert!(rep.events_applied >= 40, "{rep:?}");
        assert!(rep.faces_end > rep.faces_start, "history splits faces: {rep:?}");
        assert!(d.change_years().len() >= 30);
        assert_eq!(d.layer_ids().len(), 3);
        // every year view is consistent: owners exist, parents exist
        for y in [1000, 1250, 1500, 1750, 2000] {
            d.set_year(Some(y));
            for e in d.owner.values() {
                assert!(d.entities[e].exists_in(Some(y)), "{y}: owner {} does not exist", d.entities[e].name);
            }
            // nothing left for the author to confirm: ended countries handed
            // their land on, cities follow their land
            assert!(d.lifespan_conflicts(y).is_empty(), "{y}: {:?}", d.lifespan_conflicts(y));
        }
        // same spec, same world
        let (d2, _) = history_world(&spec);
        assert_eq!(d2.topo.faces.len(), d.topo.faces.len());
        assert_eq!(d2.owner_changes, d.owner_changes);
    }
}

#[cfg(test)]
mod refine_tests {
    use super::*;

    #[test]
    fn refined_borders_stay_valid_and_keep_every_years_owners() {
        let spec = TestWorldSpec { nx: 4, ny: 3, events: 30, ..Default::default() };
        let (mut d, _) = history_world(&spec);
        let before: Vec<_> = [1100, 1500, 1900].iter().map(|y| { d.set_year(Some(*y)); d.areas_by_entity() }).collect();
        d.set_year(None);
        let rep = refine_borders(&mut d, 20.0, 0.18, 1);
        d.topo.validate().unwrap();
        assert!(rep.vertices_after > rep.vertices_before * 5, "{rep:?}");
        assert!(rep.straight * 10 < rep.edges, "most edges keep their roughness: {rep:?}");
        // the same countries hold land in every year, with areas close to before
        for (i, y) in [1100, 1500, 1900].iter().enumerate() {
            d.set_year(Some(*y));
            let after = d.areas_by_entity();
            assert_eq!(after.len(), before[i].len(), "{y}");
            for (e, a) in &before[i] {
                let b = after[e];
                assert!((a - b).abs() / a < 0.05, "{y}: {a} -> {b}");
            }
        }
    }
}
