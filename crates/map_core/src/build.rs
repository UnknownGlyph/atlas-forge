//! Build a shared-edge topology from independent polygons (strict mode).
//!
//! The classic TopoJSON approach: boundaries that two polygons share
//! *coordinate for coordinate* become one edge. Steps:
//! 1. quantise coordinates (so ±180 and tiny float noise coincide),
//! 2. orient rings (outer counter-clockwise, holes clockwise),
//! 3. find junctions — points where neighbouring rings diverge,
//! 4. cut rings into arcs at junctions and deduplicate shared arcs,
//! 5. every source polygon becomes a face; sides no polygon claims become
//!    lakes (enclosed gaps) or the ocean,
//! 6. split long arcs so no edge exceeds [`MAX_EDGE_POINTS`].
//!
//! Strict mode does not repair data: overlaps (an arc claimed twice from
//! the same side) and other inconsistencies are reported. Gaps between
//! polygons are not errors — they become water.

use crate::geom::{self, LonLat};
use crate::topo::{ArcRef, Edge, EdgeId, Face, FaceId, FaceKind, Node, NodeId, Topology, BACKGROUND};
use std::collections::HashMap;

/// Longer arcs are split with degree-2 nodes so that editing one vertex
/// never copies a huge coordinate list (persistent-structure copy cost).
pub const MAX_EDGE_POINTS: usize = 256;

/// Coordinates closer than this many degrees are treated as identical.
pub const QUANTUM: f64 = 1e-9;

#[derive(Clone, Debug)]
pub struct SourcePolygon {
    /// Outer ring first, then holes. Closing point optional.
    pub rings: Vec<Vec<LonLat>>,
}

#[derive(Clone, Debug, Default)]
pub struct BuildReport {
    pub faces: usize,
    pub edges: usize,
    pub lakes: usize,
    pub skipped_rings: usize,
    pub problems: Vec<String>,
}

type Key = (i64, i64);

/// Remove zero-width spikes `a, b, a` (cyclically), e.g. the detour some
/// datasets draw from a coast to the pole and back along the antimeridian.
fn remove_spikes(ks: &mut Vec<Key>) {
    let mut changed = true;
    while changed && ks.len() >= 3 {
        changed = false;
        let n = ks.len();
        for i in 0..n {
            let (a, c) = (ks[(i + n - 1) % n], ks[(i + 1) % n]);
            if a == c {
                // drop the tip and one copy of the base
                let (tip, base) = (i, (i + 1) % n);
                let (hi, lo) = if tip > base { (tip, base) } else { (base, tip) };
                ks.remove(hi);
                ks.remove(lo);
                changed = true;
                break;
            }
        }
        // merging may create new consecutive duplicates
        ks.dedup();
        while ks.len() > 1 && ks.first() == ks.last() {
            ks.pop();
        }
    }
}

fn key(p: LonLat) -> Key {
    let p = p.normalized();
    ((p.lon / QUANTUM).round() as i64, (p.lat / QUANTUM).round() as i64)
}

fn unkey(k: Key) -> LonLat {
    LonLat::new(geom::wrap_lon(k.0 as f64 * QUANTUM), k.1 as f64 * QUANTUM)
}

struct Arc {
    keys: Vec<Key>,
    left: Option<FaceId>,
    right: Option<FaceId>,
}

/// Build a topology from polygons. Returns the topology and, for each
/// source polygon, the face it became (None if it was unusable).
pub fn build(polys: &[SourcePolygon]) -> Result<(Topology, Vec<Option<FaceId>>, BuildReport), BuildReport> {
    build_from(polys, 1)
}

/// Like [`build`], allocating ids from `first_id` upwards (so a rebuilt
/// topology can continue an existing document's id counter).
pub fn build_from(polys: &[SourcePolygon], first_id: u64) -> Result<(Topology, Vec<Option<FaceId>>, BuildReport), BuildReport> {
    build_with(polys, first_id, &[])
}

/// Like [`build_from`], additionally forcing a node at each of `forced`
/// (where it lies on some ring), e.g. where a rebuilt region must connect
/// to untouched topology.
pub fn build_with(polys: &[SourcePolygon], first_id: u64, forced: &[LonLat]) -> Result<(Topology, Vec<Option<FaceId>>, BuildReport), BuildReport> {
    let mut rep = BuildReport::default();
    let prof = std::env::var_os("ATLAS_PROFILE").is_some();
    let mut tb = std::time::Instant::now();
    let mut bprof = |name: &str| {
        if prof {
            eprintln!("    build {name}: {:.1} ms", tb.elapsed().as_secs_f64() * 1e3);
        }
        tb = std::time::Instant::now();
    };

    // 1–2: clean, quantise, orient. rings: (polygon, is_outer, keys)
    let mut rings: Vec<(usize, bool, Vec<Key>)> = Vec::new();
    // the first input coordinate seen for each key: written back unchanged,
    // so geometry passing through the builder is not perturbed by the grid
    let mut original: HashMap<Key, LonLat> = HashMap::new();
    for (pi, poly) in polys.iter().enumerate() {
        for (ri, ring) in poly.rings.iter().enumerate() {
            let mut ks: Vec<Key> = Vec::with_capacity(ring.len());
            for p in ring {
                let k = key(*p);
                original.entry(k).or_insert_with(|| p.normalized());
                if ks.last() != Some(&k) {
                    ks.push(k);
                }
            }
            while ks.len() > 1 && ks.first() == ks.last() {
                ks.pop();
            }
            remove_spikes(&mut ks);
            if ks.len() < 3 {
                rep.skipped_rings += 1;
                continue;
            }
            let pts: Vec<LonLat> = ks.iter().map(|k| unkey(*k)).collect();
            let outer = ri == 0;
            let w = geom::ring_winding(&pts);
            if w.abs() > 180.0 {
                // ring around a pole (Antarctica): only an outer ring can do
                // this; orient it so the nearer pole is inside (eastward
                // winding encloses the north pole)
                if !outer {
                    rep.problems.push(format!("多边形 {pi} 的洞 {ri} 包围极点，暂不支持"));
                    continue;
                }
                let mean_lat = pts.iter().map(|p| p.lat).sum::<f64>() / pts.len() as f64;
                if (mean_lat >= 0.0) != (w > 0.0) {
                    ks.reverse();
                }
            } else {
                let area = geom::signed_area(&geom::ring_to_planar(&pts));
                if (outer && area < 0.0) || (!outer && area > 0.0) {
                    ks.reverse();
                }
            }
            if ri > 0 && rings.last().is_none_or(|r| r.0 != pi) {
                // hole without a usable outer ring
                rep.skipped_rings += 1;
                continue;
            }
            rings.push((pi, outer, ks));
        }
    }

    bprof("clean");
    // 3: junctions
    let mut seen: HashMap<Key, (Key, Key)> = HashMap::new();
    let mut junction: std::collections::HashSet<Key> = forced.iter().map(|p| key(*p)).collect();
    for (_, _, ks) in &rings {
        let n = ks.len();
        for i in 0..n {
            let (a, b) = (ks[(i + n - 1) % n], ks[(i + 1) % n]);
            let pair = if a < b { (a, b) } else { (b, a) };
            match seen.get(&ks[i]) {
                None => {
                    seen.insert(ks[i], pair);
                }
                Some(p) if *p != pair => {
                    junction.insert(ks[i]);
                }
                Some(_) => {}
            }
        }
    }
    // rings visiting a point twice (pinches) also need a junction there
    for (_, _, ks) in &rings {
        let mut local = std::collections::HashSet::new();
        for k in ks {
            if !local.insert(*k) {
                junction.insert(*k);
            }
        }
    }

    bprof("junctions");
    // 4: cut into arcs, deduplicate
    let mut arcs: Vec<Arc> = Vec::new();
    let mut by_canon: HashMap<Vec<Key>, usize> = HashMap::new();
    let mut ring_refs: Vec<Vec<(usize, bool)>> = Vec::new();
    for (_, _, ks) in &rings {
        let n = ks.len();
        // start at a junction; junction-free rings start at their smallest
        // point so identical rings (e.g. a filled hole) cut identically
        let start = (0..n).find(|i| junction.contains(&ks[*i])).unwrap_or_else(|| (0..n).min_by_key(|i| ks[*i]).unwrap());
        let mut refs = Vec::new();
        let mut cur = vec![ks[start]];
        for step in 1..=n {
            let k = ks[(start + step) % n];
            cur.push(k);
            if junction.contains(&k) || step == n {
                let rev: Vec<Key> = cur.iter().rev().copied().collect();
                let (canon, fwd) = if cur <= rev { (cur.clone(), true) } else { (rev, false) };
                let id = *by_canon.entry(canon.clone()).or_insert_with(|| {
                    arcs.push(Arc { keys: canon, left: None, right: None });
                    arcs.len() - 1
                });
                refs.push((id, fwd));
                cur = vec![k];
            }
        }
        ring_refs.push(refs);
    }

    bprof("arcs");
    // 5a: faces for source polygons, claim arc sides
    let mut topo = Topology::default();
    topo.reserve_ids(first_id);
    let mut face_of: Vec<Option<FaceId>> = vec![None; polys.len()];
    let mut face_rings: HashMap<FaceId, Vec<Vec<(usize, bool)>>> = HashMap::new();
    for ((pi, _, _), refs) in rings.iter().zip(&ring_refs) {
        let f = *face_of[*pi].get_or_insert_with(|| FaceId(topo.alloc()));
        for &(a, fwd) in refs {
            let mid = arcs[a].keys[arcs[a].keys.len() / 2];
            let side = if fwd { &mut arcs[a].left } else { &mut arcs[a].right };
            if let Some(other) = side {
                if rep.problems.len() < 50 {
                    let p = unkey(mid);
                    rep.problems.push(format!("重叠：两个多边形从同一侧使用了同一段边界（约 {:.4}°, {:.4}°，面 {} 与 {}）", p.lon, p.lat, other.0, f.0));
                }
            } else {
                *side = Some(f);
            }
        }
        face_rings.entry(f).or_default().push(refs.clone());
    }
    if !rep.problems.is_empty() {
        return Err(rep);
    }

    bprof("claim");
    // 6 + edges: split long arcs, create nodes and edges
    let mut node_of: HashMap<Key, NodeId> = HashMap::new();
    let mut node = |topo: &mut Topology, k: Key| -> NodeId {
        *node_of.entry(k).or_insert_with(|| {
            let id = NodeId(topo.alloc());
            topo.nodes.insert(id, Node { pos: original.get(&k).copied().unwrap_or_else(|| unkey(k)) });
            id
        })
    };
    // arc index -> its edges in forward order
    let mut arc_edges: Vec<Vec<EdgeId>> = Vec::with_capacity(arcs.len());
    const UNSET: FaceId = FaceId(u64::MAX);
    for a in &arcs {
        let mut chain = Vec::new();
        let k = &a.keys;
        let mut s = 0;
        while s + 1 < k.len() {
            let e_end = (s + MAX_EDGE_POINTS + 1).min(k.len() - 1);
            let (na, nb) = (node(&mut topo, k[s]), node(&mut topo, k[e_end]));
            let id = EdgeId(topo.alloc());
            let pts = k[s + 1..e_end].iter().map(|x| original.get(x).copied().unwrap_or_else(|| unkey(*x))).collect();
            topo.put_edge(id, Edge { a: na, b: nb, pts, left: a.left.unwrap_or(UNSET), right: a.right.unwrap_or(UNSET), bbox: [0.0; 4] });
            chain.push(id);
            s = e_end;
        }
        arc_edges.push(chain);
    }
    let expand = |refs: &[(usize, bool)]| -> Vec<ArcRef> {
        let mut out = Vec::new();
        for &(a, fwd) in refs {
            let es = &arc_edges[a];
            if fwd {
                out.extend(es.iter().map(|e| ArcRef { edge: *e, rev: false }));
            } else {
                out.extend(es.iter().rev().map(|e| ArcRef { edge: *e, rev: true }));
            }
        }
        out
    };
    bprof("edges");
    let mut ids: Vec<FaceId> = face_rings.keys().copied().collect();
    ids.sort();
    for f in &ids {
        let rs = &face_rings[f];
        let outer = expand(&rs[0]);
        let holes = rs[1..].iter().map(|r| expand(r)).collect();
        topo.faces.insert(*f, Face { kind: FaceKind::Land, outer, holes, rev: 0, bbox: [0.0; 4] });
    }

    bprof("faces");
    // 5b: unclaimed sides -> lakes (counter-clockwise rings) or ocean holes
    let mut open: Vec<ArcRef> = Vec::new();
    for es in &arc_edges {
        for e in es {
            let ed = &topo.edges[e];
            if ed.left == UNSET {
                open.push(ArcRef { edge: *e, rev: false });
            }
            if ed.right == UNSET {
                open.push(ArcRef { edge: *e, rev: true });
            }
        }
    }
    let assembled = topo.assemble_rings(&open).map_err(|e| {
        rep.problems.push(format!("无法闭合外部边界：{e}"));
        rep.clone()
    })?;
    let mut lakes: Vec<(FaceId, Vec<geom::P2>, [f64; 4])> = Vec::new();
    let mut outside: Vec<Vec<ArcRef>> = Vec::new();
    for ring in assembled {
        let planar = topo.ring_planar(&ring);
        // a pole-winding unclaimed ring is the sea around a polar land mass;
        // a ring that stays on a pole (the pole-side edges of land cut
        // along meridians) encloses nothing
        let at_pole = ring.iter().all(|r| topo.on_pole(&topo.edges[&r.edge]));
        if !at_pole && !topo.is_polar(&ring) && geom::signed_area(&planar) > 0.0 {
            let f = FaceId(topo.alloc());
            for r in &ring {
                let e = topo.edges.get_mut(&r.edge).unwrap();
                if r.rev {
                    e.right = f;
                } else {
                    e.left = f;
                }
            }
            topo.faces.insert(f, Face { kind: FaceKind::Water, outer: ring, holes: vec![], rev: 0, bbox: [0.0; 4] });
            let bb = geom::bbox(&planar);
            lakes.push((f, planar, bb));
        } else {
            outside.push(ring);
        }
    }
    lakes.sort_by(|a, b| geom::signed_area(&a.1).partial_cmp(&geom::signed_area(&b.1)).unwrap());
    for ring in outside {
        // a point strictly on this ring's own segment decides containment
        let c = topo.ring_coords(&ring);
        let mid = [c[0].lon + geom::lon_delta(c[0].lon, c[1].lon) * 0.5, (c[0].lat + c[1].lat) * 0.5];
        let container = lakes.iter().find(|l| geom::point_in_poly_wrapped(mid, &l.1, &l.2)).map(|l| l.0).unwrap_or(BACKGROUND);
        for r in &ring {
            let e = topo.edges.get_mut(&r.edge).unwrap();
            if r.rev {
                e.right = container;
            } else {
                e.left = container;
            }
        }
        topo.faces.get_mut(&container).unwrap().holes.push(ring);
    }
    rep.lakes = lakes.len();

    bprof("open");
    let all: Vec<FaceId> = topo.faces.keys().copied().collect();
    for f in all {
        topo.touch(f);
    }
    bprof("touch");
    if let Err(e) = topo.validate() {
        rep.problems.push(format!("生成的拓扑无效：{e}"));
        return Err(rep);
    }
    bprof("validate");
    rep.faces = topo.faces.len() - 1;
    rep.edges = topo.edges.len();
    Ok((topo, face_of, rep))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::grid_world;

    fn faces_as_polygons(t: &Topology) -> Vec<SourcePolygon> {
        t.faces
            .iter()
            .filter(|(id, f)| **id != BACKGROUND && f.kind == FaceKind::Land)
            .map(|(_, f)| {
                let mut rings = vec![t.ring_coords(&f.outer)];
                rings.extend(f.holes.iter().map(|h| t.ring_coords(h)));
                SourcePolygon { rings }
            })
            .collect()
    }

    #[test]
    fn rebuilds_shared_borders_from_independent_polygons() {
        let src = grid_world(6, 4, 5, 2);
        let polys = faces_as_polygons(&src.topo);
        let (t, faces, rep) = build(&polys).unwrap_or_else(|r| panic!("{:?}", r.problems));
        assert!(faces.iter().all(|f| f.is_some()));
        assert_eq!(rep.faces, 24);
        // every internal border is shared: no face borders the ocean except along the coast
        let internal = t.edges.values().filter(|e| e.left != BACKGROUND && e.right != BACKGROUND).count();
        assert!(internal > 0);
        // total area is preserved
        let a1: f64 = src.topo.faces.keys().map(|f| src.topo.face_area(*f, 6371.0)).sum();
        let a2: f64 = t.faces.keys().map(|f| t.face_area(*f, 6371.0)).sum();
        assert!((a1 - a2).abs() / a1 < 1e-9, "{a1} vs {a2}");
        // point location agrees
        let mut rng = crate::sample::Rng::new(1);
        for _ in 0..300 {
            let p = LonLat::new(rng.range(-160.0, 160.0), rng.range(-70.0, 80.0));
            assert_eq!(src.topo.face_at(p) == BACKGROUND, t.face_at(p) == BACKGROUND);
        }
    }

    #[test]
    fn gaps_become_lakes_holes_and_seams_join() {
        let sq = |x: f64, y: f64, s: f64| vec![LonLat::new(x, y), LonLat::new(x + s, y), LonLat::new(x + s, y + s), LonLat::new(x, y + s)];
        // two halves of one country split at the antimeridian (RFC 7946 style)
        let east = SourcePolygon { rings: vec![vec![LonLat::new(170.0, 0.0), LonLat::new(180.0, 0.0), LonLat::new(180.0, 10.0), LonLat::new(170.0, 10.0)]] };
        let west = SourcePolygon { rings: vec![vec![LonLat::new(-180.0, 0.0), LonLat::new(-170.0, 0.0), LonLat::new(-170.0, 10.0), LonLat::new(-180.0, 10.0)]] };
        // a ring of four squares enclosing an empty square: the gap becomes a lake
        let ring4 = vec![
            SourcePolygon { rings: vec![vec![LonLat::new(0.0, 0.0), LonLat::new(30.0, 0.0), LonLat::new(30.0, 10.0), LonLat::new(20.0, 10.0), LonLat::new(10.0, 10.0), LonLat::new(0.0, 10.0)]] },
            SourcePolygon { rings: vec![vec![LonLat::new(0.0, 20.0), LonLat::new(10.0, 20.0), LonLat::new(20.0, 20.0), LonLat::new(30.0, 20.0), LonLat::new(30.0, 30.0), LonLat::new(0.0, 30.0)]] },
            SourcePolygon { rings: vec![sq(0.0, 10.0, 10.0)] },
            SourcePolygon { rings: vec![sq(20.0, 10.0, 10.0)] },
        ];
        // polygon with an unfilled hole and an island inside it
        let holed = SourcePolygon { rings: vec![sq(50.0, 0.0, 30.0), sq(60.0, 10.0, 10.0)] };
        let isle = SourcePolygon { rings: vec![sq(63.0, 13.0, 3.0)] };
        let mut polys = vec![east, west];
        polys.extend(ring4);
        polys.push(holed);
        polys.push(isle);
        let (t, faces, rep) = build(&polys).unwrap_or_else(|r| panic!("{:?}", r.problems));
        assert_eq!(rep.lakes, 2, "the enclosed gap and the unfilled hole");
        // seam halves share an edge along the antimeridian
        let (e, w) = (faces[0].unwrap(), faces[1].unwrap());
        assert!(t.edges.values().any(|x| (x.left == e && x.right == w) || (x.left == w && x.right == e)));
        // the island sits in the lake inside the holed polygon
        assert_eq!(t.face_at(LonLat::new(64.0, 14.0)), faces[7].unwrap());
        let lake = t.face_at(LonLat::new(61.0, 11.0));
        assert_eq!(t.faces[&lake].kind, FaceKind::Water);
        assert_eq!(t.face_at(LonLat::new(15.0, 15.0)) != BACKGROUND, true);
    }

    #[test]
    fn polar_cap_with_seam_spike_like_natural_earth() {
        // Antarctica-style ring: a coastline at -70° all the way round, drawn
        // as Natural Earth does it: down the antimeridian to the pole, along
        // the pole and back up (a zero-width spike after ±180 is merged)
        let mut ring = vec![LonLat::new(-180.0, -70.0)];
        for i in 1..36 {
            ring.push(LonLat::new(-180.0 + i as f64 * 10.0, -70.0));
        }
        ring.extend([LonLat::new(180.0, -70.0), LonLat::new(180.0, -90.0), LonLat::new(-180.0, -90.0)]);
        let isle = SourcePolygon { rings: vec![vec![LonLat::new(0.0, 0.0), LonLat::new(5.0, 0.0), LonLat::new(5.0, 5.0)]] };
        let (t, faces, rep) = build(&[SourcePolygon { rings: vec![ring] }, isle]).unwrap_or_else(|r| panic!("{:?}", r.problems));
        assert_eq!(rep.lakes, 0, "the sea around a polar cap is ocean, not a lake");
        let cap = faces[0].unwrap();
        assert_eq!(t.face_at(LonLat::new(30.0, -85.0)), cap);
        assert_eq!(t.face_at(LonLat::new(30.0, -60.0)), BACKGROUND);
        // spherical cap south of -70°: 2πR²(1 - sin 70°)
        let r = 6371.0;
        let exact = 2.0 * std::f64::consts::PI * r * r * (1.0 - 70f64.to_radians().sin());
        let a = t.face_area(cap, r);
        assert!((a - exact).abs() / exact < 1e-9, "{a} vs {exact}");
    }

    #[test]
    fn overlaps_are_reported_and_long_arcs_are_split() {
        let sq = |x: f64| vec![LonLat::new(x, 0.0), LonLat::new(x + 10.0, 0.0), LonLat::new(x + 10.0, 10.0), LonLat::new(x, 10.0)];
        let dup = vec![SourcePolygon { rings: vec![sq(0.0)] }, SourcePolygon { rings: vec![sq(0.0)] }];
        let err = build(&dup).unwrap_err();
        assert!(!err.problems.is_empty());
        let circle: Vec<LonLat> = (0..2000).map(|i| {
            let t = i as f64 / 2000.0 * std::f64::consts::TAU;
            LonLat::new(20.0 * t.cos(), 20.0 * t.sin())
        }).collect();
        let (t, _, _) = build(&[SourcePolygon { rings: vec![circle] }]).unwrap();
        assert!(t.edges.values().all(|e| e.pts.len() <= MAX_EDGE_POINTS));
        assert!(t.edges.len() >= 2000 / (MAX_EDGE_POINTS + 1));
    }
}
