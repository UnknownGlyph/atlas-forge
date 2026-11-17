//! Randomised operation sequences against the topology.
//!
//! After every successful edit: all invariants hold (`validate`) and the
//! faces partition the sphere (every probe point lies in exactly one face).
//! After every failed edit: the document is unchanged.
//! Failures print the seed and step so they can be replayed.

use map_core::doc::Document;
use map_core::geom::{self, LonLat};
use map_core::history::History;
use map_core::sample::{blob, Rng};
use map_core::topo::{FaceKind, VertexRef, BACKGROUND};

fn faces_containing(d: &Document, p: LonLat) -> usize {
    d.topo.faces.keys().filter(|f| d.topo.face_contains(**f, p)).count()
}

fn check(d: &Document, rng: &mut Rng, ctx: &str) {
    if let Err(e) = d.topo.validate() {
        panic!("{ctx}: invalid topology: {e}");
    }
    for _ in 0..40 {
        let p = LonLat::new(rng.range(-180.0, 180.0), rng.range(-80.0, 80.0));
        // points exactly on boundaries are ambiguous; nudge-free random points almost never hit them
        let n = faces_containing(d, p);
        assert_eq!(n, 1, "{ctx}: point {p:?} lies in {n} faces");
    }
}

fn random_cut(d: &Document, rng: &mut Rng) -> Option<Vec<LonLat>> {
    let faces: Vec<_> = d.topo.faces.keys().copied().filter(|f| *f != BACKGROUND).collect();
    if faces.is_empty() {
        return None;
    }
    let f = faces[(rng.next_f64() * faces.len() as f64) as usize];
    let b = d.topo.faces[&f].bbox;
    // a random interior point of the face
    let c = (0..30).find_map(|_| {
        let p = LonLat::new(geom::wrap_lon(rng.range(b[0], b[2])), rng.range(b[1], b[3]));
        d.topo.face_contains(f, p).then_some(p)
    })?;
    let ang = rng.range(0.0, std::f64::consts::PI);
    let (dx, dy) = (ang.cos() * 60.0, ang.sin() * 60.0);
    let a = d.topo.first_hit(c, LonLat::new(geom::wrap_lon(c.lon + dx), (c.lat + dy).clamp(-89.0, 89.0)))?;
    let z = d.topo.first_hit(c, LonLat::new(geom::wrap_lon(c.lon - dx), (c.lat - dy).clamp(-89.0, 89.0)))?;
    let mut pts = vec![a];
    // a kinked path through the centre
    let off = rng.range(-0.3, 0.3);
    pts.push(LonLat::new(geom::wrap_lon(c.lon + off), c.lat - off));
    pts.push(z);
    Some(pts)
}

fn random_vertex(d: &Document, rng: &mut Rng) -> Option<VertexRef> {
    let edges: Vec<_> = d.topo.edges.iter().filter(|(_, e)| !e.pts.is_empty()).map(|(id, e)| (*id, e.pts.len())).collect();
    if edges.is_empty() {
        return None;
    }
    let (e, n) = edges[(rng.next_f64() * edges.len() as f64) as usize];
    if rng.next_f64() < 0.2 {
        let nodes: Vec<_> = d.topo.nodes.keys().copied().collect();
        return Some(VertexRef::Node(nodes[(rng.next_f64() * nodes.len() as f64) as usize]));
    }
    Some(VertexRef::Pt { edge: e, idx: (rng.next_f64() * n as f64) as usize })
}

fn run(seed: u64, steps: usize) -> (usize, usize) {
    let mut rng = Rng::new(seed);
    let mut d = Document::default();
    let mut h = History::default();
    let (mut ok, mut rejected) = (0, 0);
    for step in 0..steps {
        let before = d.clone();
        let r = rng.next_f64();
        let label;
        let res: Result<(), String> = if r < 0.15 || d.topo.faces.len() < 2 {
            label = "island";
            // sometimes straddle the antimeridian, sometimes inside existing land (lake)
            let lon = if rng.next_f64() < 0.3 { rng.range(170.0, 190.0) } else { rng.range(-170.0, 170.0) };
            let c = LonLat::new(geom::wrap_lon(lon), rng.range(-60.0, 60.0));
            let (rx, ry, n) = (rng.range(1.0, 25.0), rng.range(1.0, 20.0), 12 + (rng.next_f64() * 30.0) as usize);
            let ring = blob(&mut rng, c, rx, ry, n, 0.2);
            let kind = if rng.next_f64() < 0.25 { FaceKind::Water } else { FaceKind::Land };
            h.apply(&mut d, label, |d| d.add_ring_face(&ring, kind).map(|_| ()))
        } else if r < 0.40 {
            label = "split";
            match random_cut(&d, &mut rng) {
                Some(cut) => h.apply(&mut d, label, |d| d.split_face(&cut).map(|_| ())),
                None => Ok(()),
            }
        } else if r < 0.60 {
            label = "move";
            match random_vertex(&d, &mut rng) {
                Some(v) => {
                    let p = d.topo.position_of(v);
                    let s = rng.range(0.05, 4.0);
                    let q = LonLat::new(geom::wrap_lon(p.lon + rng.range(-s, s)), (p.lat + rng.range(-s, s)).clamp(-85.0, 85.0));
                    h.apply(&mut d, label, |d| d.topo.move_vertex(v, q))
                }
                None => Ok(()),
            }
        } else if r < 0.70 {
            label = "insert";
            let edges: Vec<_> = d.topo.edges.keys().copied().collect();
            if edges.is_empty() {
                Ok(())
            } else {
                let e = edges[(rng.next_f64() * edges.len() as f64) as usize];
                let full = d.topo.edge_full(&d.topo.edges[&e]);
                let seg = (rng.next_f64() * (full.len() - 1) as f64) as usize;
                let (a, b) = (full[seg], full[seg + 1]);
                let t = rng.range(0.1, 0.9);
                let q = LonLat::new(geom::wrap_lon(a.lon + geom::lon_delta(a.lon, b.lon) * t), a.lat + (b.lat - a.lat) * t);
                h.apply(&mut d, label, |d| d.topo.insert_vertex(e, seg, q).map(|_| ()))
            }
        } else if r < 0.78 {
            label = "delete";
            match random_vertex(&d, &mut rng) {
                Some(v) => h.apply(&mut d, label, |d| d.topo.delete_vertex(v)),
                None => Ok(()),
            }
        } else if r < 0.86 {
            label = "merge";
            let edges: Vec<_> = d.topo.edges.values().filter(|e| e.left != BACKGROUND && e.right != BACKGROUND).map(|e| (e.left, e.right)).collect();
            if edges.is_empty() {
                Ok(())
            } else {
                let (a, b) = edges[(rng.next_f64() * edges.len() as f64) as usize];
                h.apply(&mut d, label, |d| d.merge_faces(a, b).map(|_| ()))
            }
        } else if r < 0.93 {
            label = "undo";
            h.undo(&mut d);
            Ok(())
        } else {
            label = "redo";
            h.redo(&mut d);
            Ok(())
        };
        let ctx = format!("seed {seed} step {step} ({label})");
        match res {
            Ok(()) => ok += 1,
            Err(_) => {
                rejected += 1;
                assert_eq!(d, before, "{ctx}: failed edit modified the document");
            }
        }
        check(&d, &mut rng, &ctx);
    }
    (ok, rejected)
}

#[test]
fn random_operation_sequences() {
    let seeds: u64 = std::env::var("ATLAS_FUZZ_SEEDS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
    let steps: usize = std::env::var("ATLAS_FUZZ_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let (mut ok, mut rej) = (0, 0);
    for seed in 0..seeds {
        let (a, b) = run(seed, steps);
        ok += a;
        rej += b;
    }
    eprintln!("random ops: {ok} applied, {rej} rejected");
    assert!(ok > rej / 4, "too few edits succeed to be a meaningful test");
}
