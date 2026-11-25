//! Soak test: long random sequences of authoring operations on the test
//! world, through the same calls the application makes, checking after
//! every step what must always hold:
//!
//! - the topology is valid, and no record names a face or entity that is gone;
//! - the year's view (owners, parents) equals a full refresh of the document;
//! - water has no owner, and no owner is an entity that does not exist then;
//! - an edit made in a year leaves every earlier year's areas alone, in all layers;
//! - saving and loading gives the same world; drawing it does not panic;
//! - undoing everything gives back the starting world exactly.
//!
//! Deterministic per seed. `ATLAS_SOAK_SEEDS` and `ATLAS_SOAK_STEPS` make a
//! longer run (the default is sized for every test run); `ATLAS_SOAK_ONLY`
//! runs one seed, to reproduce a problem. When a step breaks an invariant,
//! the world as it was before that step is saved to the temporary folder
//! and the report names the file and what the step did.

use map_core::doc::{Document, EntityId, EntityKind, LayerKind, FULL_SHARE, POLITICAL};
use map_core::geom::LonLat;
use map_core::history::History;
use map_core::overlay::{self, DrawIntent};
use map_core::sample::Rng;
use map_core::testworld::{history_world, TestWorldSpec};
use map_core::topo::{FaceId, FaceKind, VertexRef, BACKGROUND};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

fn pick<T: Clone>(rng: &mut Rng, v: &[T]) -> Option<T> {
    (!v.is_empty()).then(|| v[((rng.next_f64() * v.len() as f64) as usize).min(v.len() - 1)].clone())
}

fn ellipse(c: LonLat, rx: f64, ry: f64) -> Vec<LonLat> {
    (0..18)
        .map(|i| {
            let t = i as f64 / 18.0 * std::f64::consts::TAU;
            LonLat::new(c.lon + rx * t.cos(), c.lat + ry * t.sin())
        })
        .collect()
}

/// Areas per entity in every layer in `year`, and all land (under id 0).
/// A ring around a pole, along a parallel (a little uneven).
fn parallel(lat: f64, step: f64) -> Vec<LonLat> {
    let n = (360.0 / step) as usize;
    (0..n).map(|i| LonLat::new(-180.0 + 360.0 * i as f64 / n as f64 + 0.37, lat + if i % 2 == 0 { 0.0 } else { 0.6 })).collect()
}

fn areas(d: &Document, year: Option<i32>) -> Vec<HashMap<EntityId, f64>> {
    let mut d = d.clone();
    d.set_year(year);
    let r = d.radius();
    let land: f64 = d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land).map(|(f, _)| d.topo.face_area(*f, r)).sum();
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

fn differ(a: &[HashMap<EntityId, f64>], b: &[HashMap<EntityId, f64>]) -> Option<String> {
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        let keys: std::collections::BTreeSet<_> = x.keys().chain(y.keys()).collect();
        for k in keys {
            let (p, q) = (x.get(k).copied().unwrap_or(0.0), y.get(k).copied().unwrap_or(0.0));
            if (p - q).abs() > 1e-6 * p.abs().max(q.abs()).max(1.0) + 0.5 {
                return Some(format!("layer #{i}, entity {k:?}: {p} km² became {q} km²"));
            }
        }
    }
    None
}

/// The document with face revisions set aside (cache keys, not content).
fn no_revs(d: &Document) -> Document {
    let mut d = d.clone();
    let ids: Vec<FaceId> = d.topo.faces.keys().copied().collect();
    for f in ids {
        let mut face = d.topo.faces[&f].clone();
        face.rev = 0;
        d.topo.faces.insert(f, face);
    }
    d
}

fn invariants(d: &Document) -> Result<(), String> {
    d.topo.validate().map_err(|e| format!("topology: {e}"))?;
    // every face has an area of its own, and together they fit on the globe
    let r = d.radius();
    let globe = 4.0 * std::f64::consts::PI * r * r;
    let mut sum = 0.0;
    for f in d.topo.faces.keys().filter(|f| **f != BACKGROUND) {
        let a = d.topo.face_area(*f, r);
        if !(a > 0.0 && a < globe) {
            let c = d.topo.ring_coords(&d.topo.faces[f].outer);
            return Err(format!("face {f:?} has area {a} km² ({} points from ({}, {}), {} holes, around a pole: {})", c.len(), c[0].lon, c[0].lat, d.topo.faces[f].holes.len(), d.topo.is_polar(&d.topo.faces[f].outer)));
        }
        sum += a;
    }
    if sum > globe * (1.0 + 1e-9) {
        return Err(format!("the faces add up to {sum} km², more than the globe's {globe}"));
    }
    // no unit is its own ancestor, in the latest state or the selected year
    // (a merge once made a unit its own parent, after which nothing could
    // be put under it)
    for y in [None, d.year] {
        for e in d.entities.keys() {
            let mut cur = *e;
            for _ in 0..=d.entities.len() {
                match d.parent_at(cur, y) {
                    Some(p) if p == *e => return Err(format!("{e:?} is its own ancestor in {y:?}")),
                    Some(p) => cur = p,
                    None => break,
                }
            }
        }
    }
    // every link to a style is from a feature that is there, to a style
    // that is there, and the feature looks like the style
    for (id, s) in d.style_of.iter() {
        let st = d.style(*s).ok_or_else(|| format!("{id:?} follows a style that is gone"))?;
        let f = d.features().into_iter().map(|f| f.what).find(|f| f.id() == *id).ok_or_else(|| format!("a style link is kept for missing feature {id:?}"))?;
        if d.feature_look(f) != Some(st.look.clone()) {
            return Err(format!("{f:?} does not look like its style {}", st.name));
        }
    }
    // every curve belongs to a line or area whose points are the flattened curve
    for (id, c) in d.curves.iter() {
        let pts = match (d.rivers.get(id), d.areas.get(id)) {
            (Some(r), _) => &r.pts,
            (_, Some(a)) => &a.ring,
            _ => return Err(format!("a curve is kept for missing feature {id:?}")),
        };
        if *pts != c.flatten() {
            return Err(format!("the points of {id:?} are not its curve's"));
        }
    }
    let has = |f: &FaceId| d.topo.faces.contains_key(f);
    let known = |e: &EntityId| d.entities.contains_key(e);
    // the view
    for (f, e) in d.owner.iter() {
        if !has(f) {
            return Err(format!("the view has an owner for missing face {f:?}"));
        }
        let Some(x) = d.entities.get(e) else { return Err(format!("face {f:?} is owned by missing entity {e:?}")) };
        if !x.exists_in(d.year) {
            return Err(format!("{} owns land in {:?} but does not exist then", x.name, d.year));
        }
        if d.topo.faces[f].kind != FaceKind::Land {
            return Err(format!("water face {f:?} has an owner"));
        }
    }
    let mut full = d.clone();
    full.refresh_time_view();
    if full.owner != d.owner {
        return Err("the owners in view differ from a full refresh".into());
    }
    if let Some((_, x)) = d.entities.iter().find(|(id, x)| x.parent != full.entities[*id].parent) {
        return Err(format!("the parent of {} in view differs from a full refresh", x.name));
    }
    // the records
    for f in d.owner_base.keys().chain(d.owner_changes.keys()).chain(d.kind_base.keys()).chain(d.kind_changes.keys()) {
        if !has(f) {
            return Err(format!("a history is kept for missing face {f:?}"));
        }
    }
    if let Some(e) = d.owner_base.values().chain(d.owner_changes.values().flat_map(|c| c.values().flatten())).find(|e| !known(e)) {
        return Err(format!("an ownership history names missing entity {e:?}"));
    }
    for (l, x) in d.layers.iter() {
        if let Some(f) = x.parked.base.keys().chain(x.parked.changes.keys()).chain(x.shares.base.keys()).chain(x.shares.changes.keys()).find(|f| !has(f)) {
            return Err(format!("layer {l:?} keeps records for missing face {f:?}"));
        }
        let parked = x.parked.base.values().chain(x.parked.changes.values().flat_map(|c| c.values().flatten()));
        let shared = x.shares.base.values().flat_map(|s| s.keys()).chain(x.shares.changes.values().flat_map(|c| c.values().flat_map(|s| s.keys())));
        if let Some(e) = parked.chain(shared).find(|e| !known(e)) {
            return Err(format!("layer {l:?} names missing entity {e:?}"));
        }
        for s in x.shares.base.values().chain(x.shares.changes.values().flat_map(|c| c.values())) {
            let sum: u32 = s.values().map(|bp| *bp as u32).sum();
            if sum > FULL_SHARE as u32 {
                return Err(format!("layer {l:?} has shares adding up to {sum} of {FULL_SHARE}"));
            }
        }
    }
    for (id, x) in d.entities.iter() {
        let refs = x.parent.iter().chain(x.parent_base.iter()).chain(x.parent_changes.values().flatten()).chain(x.capital.iter()).chain(x.successors.iter());
        if let Some(r) = refs.into_iter().find(|r| !known(r)) {
            return Err(format!("entity {id:?} ({}) refers to missing {r:?}", x.name));
        }
    }
    Ok(())
}

/// One seed's run: (operations applied, operations refused, problems).
fn run(seed: u64, steps: usize, at: &Mutex<String>) -> (usize, usize, Vec<String>) {
    let (mut d, _) = history_world(&TestWorldSpec { nx: 5, ny: 3, events: 40, seed, ..Default::default() });
    d.set_active_layer(POLITICAL).unwrap();
    // every other seed: continents around both poles as well
    if seed % 2 == 1 {
        for lat in [76.0, -74.0] {
            d = overlay::apply(&d, &parallel(lat, 9.0), DrawIntent::AddLand).expect("polar continent").0;
        }
        d.topo.validate().expect("polar continents");
    }
    let start = d.clone();
    let share_layer = d.layer_ids().into_iter().find(|l| d.layer_kind(*l) == LayerKind::Shares);
    // every step stays undoable, however long the run
    let mut hist = History::default();
    (hist.limit, hist.budget_bytes) = (steps + 1, usize::MAX);
    let mut rng = Rng::new(seed * 77 + 1);
    let mut problems: Vec<String> = Vec::new();
    let (mut applied, mut refused) = (0, 0);
    for step in 0..steps {
        let year = if rng.next_f64() < 0.75 { Some(rng.range(990.0, 2010.0) as i32) } else { None };
        d.set_year(year);
        let land: Vec<FaceId> = d.topo.faces.iter().filter(|(f, x)| **f != BACKGROUND && x.kind == FaceKind::Land).map(|(f, _)| *f).collect();
        let ents: Vec<EntityId> = d.entities.iter().filter(|(_, x)| x.kind == EntityKind::Country && x.layer == POLITICAL).map(|(e, _)| *e).collect();
        let cities: Vec<EntityId> = d.entities.iter().filter(|(_, x)| x.kind == EntityKind::City).map(|(e, _)| *e).collect();
        let edges: Vec<_> = d.topo.edges.iter().filter(|(_, e)| !e.pts.is_empty()).map(|(id, e)| (*id, e.pts.len(), e.pts[e.pts.len() / 2])).collect();
        let earlier = year.map(|y| areas(&d, Some(y - 1)));
        let op = (rng.next_f64() * 22.0) as usize;
        let label = format!("seed {seed} step {step} op {op} year {year:?}");
        *at.lock().unwrap() = label.clone();
        // for the report: the world before the step, and what the step did
        let before = d.clone();
        let mut detail = String::new();
        // whether the edit is "from this year on" (earlier years must not change)
        let mut timed = false;
        let res: Result<(), String> = match op {
            // drawing: new land, growth into the sea, lakes, land taken from neighbours
            0..=3 => {
                let Some((_, _, p)) = pick(&mut rng, &edges) else { continue };
                let intent = match (op, pick(&mut rng, &ents)) {
                    (1, Some(e)) => DrawIntent::Expand(e),
                    (2, _) => DrawIntent::CarveWater,
                    (3, Some(e)) => DrawIntent::Take(e),
                    _ => DrawIntent::AddLand,
                };
                timed = true;
                let r = rng.range(0.5, 3.0);
                detail = format!("draw {intent:?}, ellipse at ({:?}, {:?}) radii ({:?}, {:?})", p.lon, p.lat, r * 1.3, r);
                hist.apply(&mut d, &label, |d| {
                    let (n, rep) = overlay::apply(d, &ellipse(p, r * 1.3, r), intent)?;
                    detail += &format!(", local={} fallback={:?}", rep.local, rep.fallback);
                    *d = n;
                    Ok(())
                })
            }
            4 => {
                let (Some(f), who) = (pick(&mut rng, &land), pick(&mut rng, &ents)) else { continue };
                timed = true;
                let who = if rng.next_f64() < 0.2 { None } else { who };
                hist.apply(&mut d, &label, |d| d.record_owner(f, who))
            }
            5 => {
                let Some(f) = pick(&mut rng, &land) else { continue };
                timed = true;
                let k = if rng.next_f64() < 0.5 { FaceKind::Water } else { FaceKind::Land };
                hist.apply(&mut d, &label, |d| d.record_kind(f, k))
            }
            6 => {
                let Some((id, n, p)) = pick(&mut rng, &edges) else { continue };
                let q = LonLat::new(p.lon + rng.range(-0.3, 0.3), p.lat + rng.range(-0.3, 0.3));
                hist.apply(&mut d, &label, |d| d.topo.move_vertex(VertexRef::Pt { edge: id, idx: n / 2 }, q))
            }
            7 => {
                let Some(f) = pick(&mut rng, &land) else { continue };
                let (outer, _) = d.topo.face_planar(f);
                if outer.len() < 6 {
                    continue;
                }
                let (a, b) = (outer[0], outer[outer.len() / 2]);
                detail = format!("split {f:?} from ({:?}, {:?}) to ({:?}, {:?})", a[0], a[1], b[0], b[1]);
                hist.apply(&mut d, &label, |d| d.split_face(&[LonLat::new(a[0], a[1]), LonLat::new(b[0], b[1])]).map(|_| ()))
            }
            8 => {
                let pairs: Vec<(FaceId, FaceId)> = d.topo.edges.values().filter(|e| e.left != BACKGROUND && e.right != BACKGROUND && e.left != e.right).map(|e| (e.left, e.right)).collect();
                let Some((a, b)) = pick(&mut rng, &pairs) else { continue };
                hist.apply(&mut d, &label, |d| d.merge_faces(a, b).map(|_| ()))
            }
            9 => {
                let (Some(a), Some(b)) = (pick(&mut rng, &ents), pick(&mut rng, &ents)) else { continue };
                if a == b {
                    continue;
                }
                let merge = rng.next_f64() < 0.5;
                hist.apply(&mut d, &label, |d| {
                    if merge {
                        d.merge_entities(a, b)
                    } else {
                        d.delete_entity(a);
                        Ok(())
                    }
                })
            }
            10 => {
                let Some(e) = pick(&mut rng, &ents) else { continue };
                let (a, b) = (rng.range(900.0, 2100.0) as i32, rng.range(900.0, 2100.0) as i32);
                let (f, t) = (rng.next_f64() < 0.6, rng.next_f64() < 0.6);
                hist.apply(&mut d, &label, |d| d.set_lifespan(e, f.then_some(a.min(b)), t.then_some(a.max(b))))
            }
            11 => hist.apply(&mut d, &label, |d| {
                d.tidy_faces();
                Ok(())
            }),
            12 => {
                for _ in 0..(rng.next_f64() * 4.0) as usize {
                    hist.undo(&mut d);
                }
                for _ in 0..(rng.next_f64() * 3.0) as usize {
                    hist.redo(&mut d);
                }
                Ok(())
            }
            // a share of a face in the share layer, from this year on
            13 => {
                let (Some(l), Some(f)) = (share_layer, pick(&mut rng, &land)) else { continue };
                let units: Vec<EntityId> = d.entities.iter().filter(|(_, x)| x.layer == l).map(|(e, _)| *e).collect();
                let Some(u) = pick(&mut rng, &units) else { continue };
                let bp = (rng.next_f64() * FULL_SHARE as f64) as u16;
                timed = true;
                hist.apply(&mut d, &label, |d| {
                    d.set_active_layer(l)?;
                    let r = d.set_share(f, Some(u), bp);
                    d.set_active_layer(POLITICAL)?;
                    r
                })
            }
            // a city changes country, or a country ends and hands its land on
            14 => {
                let (Some(c), p) = (pick(&mut rng, &cities), pick(&mut rng, &ents)) else { continue };
                hist.apply(&mut d, &label, |d| d.record_parent(c, p))
            }
            15 => {
                let (Some(a), b) = (pick(&mut rng, &ents), pick(&mut rng, &ents)) else { continue };
                hist.apply(&mut d, &label, |d| d.reassign_after_end(a, b.filter(|b| *b != a)).map(|_| ()))
            }
            // a ring around a pole: land, a country's claim, or sea
            17 => {
                let lat = rng.range(58.0, 88.0) * if rng.next_f64() < 0.5 { 1.0 } else { -1.0 };
                let intent = match ((rng.next_f64() * 4.0) as usize, pick(&mut rng, &ents)) {
                    (1, Some(e)) => DrawIntent::Expand(e),
                    (2, _) => DrawIntent::CarveWater,
                    (3, Some(e)) => DrawIntent::Take(e),
                    _ => DrawIntent::AddLand,
                };
                timed = true;
                detail = format!("draw {intent:?}, ring around the pole at {lat:?}");
                hist.apply(&mut d, &label, |d| {
                    let (n, rep) = overlay::apply(d, &parallel(lat, 7.0), intent)?;
                    detail += &format!(", local={} fallback={:?}", rep.local, rep.fallback);
                    *d = n;
                    Ok(())
                })
            }
            // curves: a new curved line or area, an anchor or handle moved,
            // anchors added and removed, lines turned into curves and back,
            // features with curves moved and copied
            18 => {
                let lines: Vec<map_core::doc::LineId> = d.rivers.keys().chain(d.areas.keys()).copied().collect();
                let curved: Vec<map_core::doc::LineId> = d.curves.keys().copied().collect();
                let k = (rng.next_f64() * 6.0) as usize;
                let at = pick(&mut rng, &edges).map(|e| e.2).unwrap_or(LonLat::new(0.0, 0.0));
                let jitter = |rng: &mut Rng, p: LonLat, r: f64| LonLat::new(p.lon + rng.range(-r, r), (p.lat + rng.range(-r, r)).clamp(-85.0, 85.0));
                detail = format!("curve op {k}");
                match k {
                    0 | 1 => {
                        let n = 2 + (rng.next_f64() * 4.0) as usize + k;
                        let pts: Vec<LonLat> = (0..n).map(|i| LonLat::new(at.lon + i as f64 * 1.5, at.lat + rng.range(-2.0, 2.0))).collect();
                        let pts: Vec<LonLat> = if k == 1 { ellipse(at, 3.0, 2.0)[..n.min(18)].to_vec() } else { pts };
                        let c = map_core::curve::Curve::smooth_through(&pts, k == 1);
                        hist.apply(&mut d, &label, |d| if k == 1 { d.add_curve_area(c).map(|_| ()) } else { d.add_curve_line(c, None).map(|_| ()) })
                    }
                    2 => {
                        let Some(id) = pick(&mut rng, &curved) else { continue };
                        let n = d.curves[&id].nodes.len();
                        let i = (rng.next_f64() * n as f64) as usize % n;
                        let to = jitter(&mut rng, d.curves[&id].nodes[i].at, 1.0);
                        let h = rng.next_f64() < 0.5;
                        hist.apply(&mut d, &label, |d| {
                            d.edit_curve(id, |c| {
                                if h {
                                    c.move_handle(i, map_core::curve::Handle::Out, to);
                                } else {
                                    c.move_node(i, to);
                                }
                                Ok(())
                            })
                        })
                    }
                    3 => {
                        let Some(id) = pick(&mut rng, &curved) else { continue };
                        let n = d.curves[&id].segments().max(1);
                        let (seg, t, del) = ((rng.next_f64() * n as f64) as usize % n, rng.next_f64(), rng.next_f64() < 0.4);
                        hist.apply(&mut d, &label, |d| {
                            d.edit_curve(id, |c| {
                                if del {
                                    c.remove_node(seg)
                                } else {
                                    c.insert_node(seg, t);
                                    Ok(())
                                }
                            })
                        })
                    }
                    4 => {
                        let Some(id) = pick(&mut rng, &lines) else { continue };
                        let (smooth, back) = (rng.next_f64() < 0.5, d.curves.contains_key(&id));
                        let keep = rng.next_f64() < 0.5;
                        hist.apply(&mut d, &label, |d| if back { d.drop_curve(id, keep) } else { d.curve_from_points(id, smooth) })
                    }
                    _ => {
                        let Some(id) = pick(&mut rng, &curved) else { continue };
                        let f = if d.areas.contains_key(&id) { map_core::features::FeatureRef::Area(id) } else { map_core::features::FeatureRef::River(id) };
                        let to = jitter(&mut rng, at, 10.0);
                        let copy = rng.next_f64() < 0.5;
                        hist.apply(&mut d, &label, |d| {
                            if copy {
                                d.duplicate_features(&[f], 2.0, 1.0).map(|_| ())
                            } else {
                                let c = d.features_centre(&[f]).ok_or("no centre")?;
                                d.move_features(&[f], c, to)
                            }
                        })
                    }
                }
            }
            // styles: the usual ones added, features linked and unlinked,
            // a style changed, removed, made from a feature; the eyedropper;
            // a linked feature edited directly and the change carried over
            19 => {
                let fs: Vec<map_core::features::FeatureRef> = d.features().into_iter().map(|f| f.what).filter(|f| d.feature_look(*f).is_some()).collect();
                let styles: Vec<map_core::styles::StyleId> = d.styles.iter().map(|s| s.id).collect();
                let k = (rng.next_f64() * 7.0) as usize;
                detail = format!("style op {k}");
                match k {
                    0 => hist.apply(&mut d, &label, |d| Ok(d.add_preset_styles()).map(|_| ())),
                    1 => {
                        let Some(s) = pick(&mut rng, &styles) else { continue };
                        let some: Vec<_> = fs.iter().copied().filter(|_| rng.next_f64() < 0.3).collect();
                        hist.apply(&mut d, &label, |d| d.link_style(&some, s).map(|_| ()))
                    }
                    2 => {
                        let Some(s) = pick(&mut rng, &styles) else { continue };
                        let mut look = d.style(s).unwrap().look.clone();
                        match &mut look {
                            map_core::styles::Look::Route(r) => r.width = rng.range(0.5, 6.0) as f32,
                            map_core::styles::Look::Area(a) => a.spacing = rng.range(3.0, 20.0) as f32,
                            map_core::styles::Look::Text(t) => t.size = rng.range(5.0, 40.0) as f32,
                            map_core::styles::Look::Symbol(x) => x.size = rng.range(3.0, 15.0) as f32,
                        }
                        hist.apply(&mut d, &label, |d| d.update_style(s, look).map(|_| ()))
                    }
                    3 => {
                        let Some(s) = pick(&mut rng, &styles) else { continue };
                        hist.apply(&mut d, &label, |d| d.remove_style(s).map(|_| ()))
                    }
                    4 => {
                        let Some(f) = pick(&mut rng, &fs) else { continue };
                        hist.apply(&mut d, &label, |d| d.style_from_feature(f, "").map(|_| ()))
                    }
                    5 => {
                        let (Some(from), true) = (pick(&mut rng, &fs), fs.len() > 1) else { continue };
                        let to: Vec<_> = fs.iter().copied().filter(|_| rng.next_f64() < 0.2).collect();
                        hist.apply(&mut d, &label, |d| d.copy_look(from, &to).map(|_| ()))
                    }
                    _ => {
                        // as a feature's own panel does, then what the interface does after it
                        let Some(f) = pick(&mut rng, &fs) else { continue };
                        let c = [(rng.next_f64() * 255.0) as u8, 40, 40, 255];
                        hist.apply(&mut d, &label, |d| {
                            let mut look = d.feature_look(f).ok_or("no look")?;
                            match &mut look {
                                map_core::styles::Look::Route(r) => r.color = c,
                                map_core::styles::Look::Area(a) => a.color = c,
                                map_core::styles::Look::Text(t) => t.color = c,
                                map_core::styles::Look::Symbol(x) => x.color = c,
                            }
                            d.set_feature_look(f, &look)?;
                            d.settle_styles();
                            Ok(())
                        })
                    }
                }
            }
            20 => {
                // local views: kept, changed, dropped; what they report must add up
                let k = (rng.next_f64() * 3.0) as usize;
                detail = format!("local view op {k}");
                let (w, s0) = (rng.range(-180.0, 180.0), rng.range(-80.0, 60.0));
                let bounds = [w, s0, w + rng.range(1.0, 120.0), s0 + rng.range(1.0, 30.0)];
                let a = rng.range(990.0, 2010.0) as i32;
                let years = (rng.next_f64() < 0.8).then(|| (a, a + rng.range(0.0, 60.0) as i32));
                let r = match (k, d.focuses.first().map(|f| f.id)) {
                    (1, Some(id)) => hist.apply(&mut d, &label, |d| {
                        let f = map_core::focus::Focus { bounds, years, ..d.focus(id).unwrap().clone() };
                        d.update_focus(f)
                    }),
                    (2, Some(id)) => hist.apply(&mut d, &label, |d| d.remove_focus(id)),
                    _ => hist.apply(&mut d, &label, |d| d.add_focus("", bounds, years).map(|_| ())),
                };
                for f in d.focuses.clone() {
                    let faces = d.faces_in(&f);
                    let (from, to) = f.years.unwrap_or((a, a));
                    if !d.changed_between(&faces, from, from).is_empty() {
                        problems.push(format!("{label}: a year differs from itself"));
                    }
                    let rep = d.change_report(&faces, from, to);
                    let net: f64 = rep.areas.iter().map(|x| x.2 - x.1).sum();
                    if (net - (rep.to_land - rep.to_water)).abs() > 1e-3 * (1.0 + net.abs()) {
                        problems.push(format!("{label}: a local view's land does not add up ({net} against {})", rep.to_land - rep.to_water));
                    }
                    if rep.years.iter().any(|y| *y <= from || *y > to) {
                        problems.push(format!("{label}: a change year outside the local view's years"));
                    }
                }
                r
            }
            21 => {
                // automatic partition: new countries or provinces, from the year on
                let Some(e) = pick(&mut rng, &ents) else { continue };
                let spec = map_core::partition::PartitionSpec {
                    units: 2 + (rng.next_f64() * 4.0) as usize,
                    seed: (rng.next_f64() * 1000.0) as u64,
                    balance: rng.next_f64() < 0.5,
                    follow: rng.next_f64() < 0.5,
                    provinces: rng.next_f64() < 0.5,
                    ..Default::default()
                };
                detail = format!("partition {e:?} {spec:?}");
                timed = true;
                hist.apply(&mut d, &label, |d| {
                    let plan = d.partition_plan(e, &spec)?;
                    d.apply_partition(&plan, &spec).map(|_| ())
                })
            }
            _ => {
                let l = pick(&mut rng, &d.layer_ids()).unwrap();
                let r = d.set_active_layer(l);
                d.set_active_layer(POLITICAL).unwrap();
                r
            }
        };
        match &res {
            Ok(()) => applied += 1,
            Err(_) => refused += 1,
        }
        if let Err(e) = invariants(&d) {
            let file = std::env::temp_dir().join(format!("atlas_soak_seed{seed}_step{step}.atlas"));
            let saved = map_io::project::save(&file, &before).map(|_| file.display().to_string()).unwrap_or_else(|e| format!("not saved: {e}"));
            problems.push(format!("{label}: {e} (the operation returned {res:?}; {detail}; world before the step: {saved})"));
            break;
        }
        if let (true, Ok(()), Some(y), Some(b)) = (timed, &res, year, &earlier) {
            if let Some(diff) = differ(b, &areas(&d, Some(y - 1))) {
                let file = std::env::temp_dir().join(format!("atlas_soak_seed{seed}_step{step}.atlas"));
                let saved = map_io::project::save(&file, &before).map(|_| file.display().to_string()).unwrap_or_else(|e| format!("not saved: {e}"));
                problems.push(format!("{label}: an edit in {y} changed {}: {diff} ({detail}; world before the step: {saved})", y - 1));
                break;
            }
        }
        if step % 25 == 24 {
            let back = map_io::project::from_bytes(&map_io::project::to_bytes(&d).unwrap()).unwrap();
            if no_revs(&back) != no_revs(&d) {
                problems.push(format!("{label}: the world differs after saving and loading"));
                break;
            }
            // what the application draws must not panic
            let th = map_core::theme::evaluate(&d);
            let _ = map_io::scene::build_scene(&d, &th, &map_io::scene::ExportOptions { width: 600, height: 350, ..Default::default() });
            map_core::labels::LabelCache::default().update(&d);
        }
    }
    // undo everything: the starting world, exactly
    if problems.is_empty() {
        while hist.undo(&mut d).is_some() {}
        let mut s = start.clone();
        for x in [&mut d, &mut s] {
            x.set_active_layer(POLITICAL).unwrap();
            x.set_year(None);
        }
        if no_revs(&d) != no_revs(&s) {
            problems.push(format!("seed {seed}: undoing everything did not give back the starting world"));
        }
    }
    (applied, refused, problems)
}

#[test]
fn random_authoring_keeps_every_invariant() {
    let env = |k: &str, d: u64| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let (seeds, steps) = (env("ATLAS_SOAK_SEEDS", 16), env("ATLAS_SOAK_STEPS", 250) as usize);
    let only = env("ATLAS_SOAK_ONLY", 0);
    let (first, seeds) = if only > 0 { (only, only) } else { (1, seeds) };
    let workers = std::thread::available_parallelism().map_or(4, |n| n.get()) as u64;
    let next = std::sync::atomic::AtomicU64::new(first);
    let results: Mutex<Vec<(usize, usize, Vec<String>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..workers.min(seeds - first + 1) {
            s.spawn(|| loop {
                let seed = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if seed > seeds {
                    return;
                }
                let at = Arc::new(Mutex::new(String::new()));
                let at2 = at.clone();
                let r = std::panic::catch_unwind(move || run(seed, steps, &at2)).unwrap_or_else(|p| {
                    let msg = p.downcast_ref::<String>().cloned().or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default();
                    (0, 0, vec![format!("{}: PANIC {msg}", at.lock().map(|g| g.clone()).unwrap_or_default())])
                });
                results.lock().unwrap().push(r);
            });
        }
    });
    let results = results.into_inner().unwrap();
    let (applied, refused): (usize, usize) = results.iter().fold((0, 0), |a, r| (a.0 + r.0, a.1 + r.1));
    let problems: Vec<&String> = results.iter().flat_map(|r| r.2.iter()).collect();
    eprintln!("soak: seeds {first}..={seeds} x {steps} steps: {applied} operations applied, {refused} refused, {} problems", problems.len());
    assert!(applied > refused, "most operations should go through: {applied} applied, {refused} refused");
    assert!(problems.is_empty(), "{} problems:\n{}", problems.len(), problems.iter().map(|p| format!("  {p}")).collect::<Vec<_>>().join("\n"));
}

#[test]
fn mixed_board_edits_survive_navigation_undo_and_reopen() {
    use map_core::{doc::LineId, pixel_art::PixelArt, star_chart::StarChart, symbol::{MapSymbol, SymbolKind}};
    use map_core::{underlay::Blob, world_book::{Attachment, Link, Node, WorldBook, ROOT}};

    for seed in 1..=8 {
        let mut rng = Rng::new(seed);
        let mut doc = Document::default();
        let book = WorldBook::edit(&mut doc);
        let region = book.add_map(Document::new("region", 6371.0));
        let city = book.add_map(Document::new("city", 6371.0));
        for (parent, child) in [(ROOT, region), (region, city)] {
            book.add_link(Link::new(Node::Map(parent), Node::Map(child), true)).unwrap();
        }
        let boards = [ROOT, region, city];
        let mut history = History::default();
        for step in 0..32 {
            let board = pick(&mut rng, &boards).unwrap();
            WorldBook::switch(&mut doc, board, None).unwrap();
            let before = doc.clone();
            history.apply(&mut doc, "mixed edit", |d| {
                match step % 4 {
                    0 => {
                        let event = d.add_event("treaty", step, None);
                        let book = WorldBook::edit(d);
                        let article = book.add_article(format!("record {step}"));
                        book.add_link(Link::new(Node::Event(board, event), Node::Article(article), false))?;
                    }
                    1 => {
                        let id = LineId(d.topo.alloc());
                        let mut mark = MapSymbol::new(SymbolKind::Site, LonLat::new(f64::from(step), 5.0));
                        mark.pixels = Some(Arc::new(PixelArt::from_pixels(1, 1, vec![[step as u8, 10, 24, 255]])?));
                        d.symbols.insert(id, mark);
                    }
                    2 => {
                        let book = WorldBook::edit(d);
                        let asset = book.add_asset(Attachment {name: "note.txt".into(), bytes: Blob(Arc::new(vec![step as u8; 16]))});
                        book.add_link(Link::new(Node::Map(board), Node::Asset(asset), false))?;
                    }
                    _ => {
                        let chart = d.star_chart.get_or_insert_with(|| Arc::new(StarChart::default()));
                        Arc::make_mut(chart).galaxy.name = format!("galaxy {step}");
                    }
                }
                Ok(())
            }).unwrap();
            let after = doc.clone();
            history.undo(&mut doc).unwrap();
            assert_eq!(doc, before, "undo seed {seed}, step {step}");
            history.redo(&mut doc).unwrap();
            assert_eq!(doc, after, "redo seed {seed}, step {step}");
            if step % 4 == 3 {
                let encoded = map_io::project::to_bytes(&doc).unwrap();
                let loaded = map_io::project::from_bytes(&encoded).unwrap();
                let book = loaded.book.as_ref().unwrap();
                book.validate().unwrap();
                let expected = doc.book.as_ref().unwrap();
                assert_eq!(book.articles, expected.articles);
                assert_eq!(book.links, expected.links);
                assert_eq!(book.assets, expected.assets);
                for id in boards {
                    let old = expected.map(&doc, id).unwrap();
                    let new = book.map(&loaded, id).unwrap();
                    assert_eq!(new.events, old.events);
                    assert_eq!(new.symbols, old.symbols);
                    assert_eq!(new.star_chart, old.star_chart);
                }
                doc = loaded;
            }
        }
    }
}
