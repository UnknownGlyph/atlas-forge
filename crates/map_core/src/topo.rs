//! Planar topology: nodes, edges (shared boundary arcs) and faces.
//!
//! Invariants (checked by [`Topology::validate`]):
//! * a face lies on the *left* of every ring that bounds it
//!   (outer rings counter-clockwise, holes clockwise);
//! * a forward reference to an edge in face F's rings implies `edge.left == F`,
//!   a reversed reference implies `edge.right == F`;
//! * every edge is referenced exactly once forward and once reversed;
//! * [`BACKGROUND`] is the face covering everything no other face covers; it
//!   has no outer ring and the outer rings of top-level faces are its holes.
//!   This is purely topological: whether it is sea or land is its `kind`,
//!   like any other face (an all-land planet has a land background with
//!   seas drawn as water faces).

use crate::geom::{self, LonLat, P2};
use crate::index::GridIndex;
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(pub u64);
    };
}
id_type!(NodeId);
id_type!(EdgeId);
id_type!(FaceId);

pub const BACKGROUND: FaceId = FaceId(0);

pub type TResult<T> = Result<T, String>;

static REV: AtomicU64 = AtomicU64::new(1);

/// Process-wide monotonically increasing geometry revision. It deliberately
/// lives outside the (undoable) document so that a revision number is never
/// reused for different geometry, which makes `(FaceId, rev)` a safe cache key.
pub fn next_rev() -> u64 {
    REV.fetch_add(1, Ordering::Relaxed)
}
/// Face areas on the unit sphere by geometry revision (see [`next_rev`]).
fn area_cache() -> &'static std::sync::Mutex<HashMap<u64, f64>> {
    static C: std::sync::OnceLock<std::sync::Mutex<HashMap<u64, f64>>> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

pub fn ensure_rev_above(v: u64) {
    REV.fetch_max(v + 1, Ordering::Relaxed);
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub pos: LonLat,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edge {
    pub a: NodeId,
    pub b: NodeId,
    /// Interior vertices (endpoints are the nodes).
    pub pts: Vec<LonLat>,
    pub left: FaceId,
    pub right: FaceId,
    /// Unwrapped bbox (lon from node `a`), for fast rejection.
    pub bbox: [f64; 4],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaceKind {
    Land,
    Water,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArcRef {
    pub edge: EdgeId,
    pub rev: bool,
}

pub type Ring = Vec<ArcRef>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Face {
    pub kind: FaceKind,
    pub outer: Ring,
    pub holes: Vec<Ring>,
    /// Geometry revision; changes whenever any bounding edge changes.
    pub rev: u64,
    /// Unwrapped bbox of the outer ring.
    pub bbox: [f64; 4],
}

/// A vertex that can be edited.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VertexRef {
    Node(NodeId),
    /// Interior point `idx` of `edge.pts`.
    Pt { edge: EdgeId, idx: usize },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Topology {
    pub nodes: OrdMap<NodeId, Node>,
    pub edges: OrdMap<EdgeId, Edge>,
    pub faces: OrdMap<FaceId, Face>,
    next_id: u64,
    /// Spatial indexes (edge bboxes, face outer bboxes). Not serialised:
    /// rebuilt on load with [`Topology::rebuild_index`].
    #[serde(skip)]
    edge_index: GridIndex,
    #[serde(skip)]
    face_index: GridIndex,
}

impl Default for Topology {
    fn default() -> Self {
        let mut faces = OrdMap::new();
        faces.insert(
            BACKGROUND,
            Face { kind: FaceKind::Water, outer: vec![], holes: vec![], rev: next_rev(), bbox: [-180.0, -90.0, 180.0, 90.0] },
        );
        Self { nodes: OrdMap::new(), edges: OrdMap::new(), faces, next_id: 1, edge_index: GridIndex::default(), face_index: GridIndex::default() }
    }
}

fn bbox_overlap_wrapped(a: &[f64; 4], b: &[f64; 4]) -> bool {
    if a[1] > b[3] || b[1] > a[3] {
        return false;
    }
    [-360.0, 0.0, 360.0].iter().any(|k| !(a[0] > b[2] + k || b[0] + k > a[2]))
}

fn seg_bbox(a: LonLat, b: LonLat) -> [f64; 4] {
    let bx = a.lon + geom::lon_delta(a.lon, b.lon);
    [a.lon.min(bx), a.lat.min(b.lat), a.lon.max(bx), a.lat.max(b.lat)]
}

/// Shrink a segment towards its interior so touching at the endpoints is not
/// reported as a crossing.
fn shrink(a: LonLat, b: LonLat, at_a: bool, at_b: bool) -> (LonLat, LonLat) {
    let dx = geom::lon_delta(a.lon, b.lon);
    let dy = b.lat - a.lat;
    let e = 1e-6;
    let a2 = if at_a { LonLat::new(geom::wrap_lon(a.lon + dx * e), a.lat + dy * e) } else { a };
    let b2 = if at_b { LonLat::new(geom::wrap_lon(b.lon - dx * e), b.lat - dy * e) } else { b };
    (a2, b2)
}

impl Topology {
    /// Next id `alloc` will hand out.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// Never hand out ids below `n` (ids are shared with entities, fields
    /// and lines, so a rebuilt topology must continue the old counter).
    pub fn reserve_ids(&mut self, n: u64) {
        self.next_id = self.next_id.max(n);
    }

    pub fn alloc(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // ---------------------------------------------------------------- access

    pub fn node_pos(&self, n: NodeId) -> LonLat {
        self.nodes[&n].pos
    }

    /// Full coordinates of an edge including its end nodes.
    pub fn edge_full(&self, e: &Edge) -> Vec<LonLat> {
        let mut v = Vec::with_capacity(e.pts.len() + 2);
        v.push(self.node_pos(e.a));
        v.extend_from_slice(&e.pts);
        v.push(self.node_pos(e.b));
        v
    }

    pub fn ref_start(&self, r: ArcRef) -> NodeId {
        let e = &self.edges[&r.edge];
        if r.rev {
            e.b
        } else {
            e.a
        }
    }
    pub fn ref_end(&self, r: ArcRef) -> NodeId {
        let e = &self.edges[&r.edge];
        if r.rev {
            e.a
        } else {
            e.b
        }
    }

    /// Ring coordinates, first point not repeated at the end.
    pub fn ring_coords(&self, ring: &[ArcRef]) -> Vec<LonLat> {
        let mut out = Vec::new();
        for r in ring {
            let mut c = self.edge_full(&self.edges[&r.edge]);
            if r.rev {
                c.reverse();
            }
            c.pop();
            out.extend(c);
        }
        out
    }

    /// Does the edge lie on a pole (every point at latitude ±90)? Land
    /// around a pole that is cut along meridians has such edges: the
    /// stretch of the map's top or bottom line that belongs to each piece.
    /// On the globe they are a single point, so they are no border and are
    /// not drawn.
    pub fn on_pole(&self, e: &Edge) -> bool {
        self.nodes[&e.a].pos.lat.abs() == 90.0 && self.nodes[&e.b].pos.lat.abs() == 90.0 && e.pts.iter().all(|p| p.lat.abs() == 90.0)
    }

    /// Does the ring wind around a pole (e.g. Antarctica's coastline)?
    pub fn is_polar(&self, ring: &[ArcRef]) -> bool {
        geom::ring_winding(&self.ring_coords(ring)).abs() > 180.0
    }

    pub fn ring_planar(&self, ring: &[ArcRef]) -> Vec<P2> {
        geom::ring_to_planar(&self.ring_coords(ring))
    }

    /// Planar outer ring and holes, holes shifted by multiples of 360° to sit
    /// over the outer ring's longitude range.
    pub fn face_planar(&self, f: FaceId) -> (Vec<P2>, Vec<Vec<P2>>) {
        let face = &self.faces[&f];
        let outer = if face.outer.is_empty() { vec![] } else { self.ring_planar(&face.outer) };
        let ob = geom::bbox(&outer);
        let holes = face
            .holes
            .iter()
            .map(|h| {
                let mut p = geom::unwrap_path(&self.ring_coords(h), None);
                if !outer.is_empty() && !p.is_empty() {
                    let c = (ob[0] + ob[2]) * 0.5;
                    let k = ((c - p[0][0]) / 360.0).round() * 360.0;
                    for q in &mut p {
                        q[0] += k;
                    }
                }
                p
            })
            .collect();
        (outer, holes)
    }

    pub fn face_contains(&self, f: FaceId, p: LonLat) -> bool {
        let face = &self.faces[&f];
        let pt = [p.lon, p.lat];
        if f != BACKGROUND {
            if !bbox_overlap_wrapped(&face.bbox, &[p.lon, p.lat, p.lon, p.lat]) {
                return false;
            }
            let outer = self.ring_planar(&face.outer);
            if !geom::point_in_poly_wrapped(pt, &outer, &face.bbox) {
                return false;
            }
        }
        for h in &face.holes {
            // a hole ring's interior is on its right; reverse to get it on the left
            let mut c = self.ring_coords(h);
            c.reverse();
            let hp = geom::ring_to_planar(&c);
            let hb = geom::bbox(&hp);
            if geom::point_in_poly_wrapped(pt, &hp, &hb) {
                return false;
            }
        }
        true
    }

    /// The face containing a point (the background if no other face does).
    pub fn face_at(&self, p: LonLat) -> FaceId {
        let probe = [p.lon, p.lat, p.lon, p.lat];
        for i in self.face_index.query(probe) {
            let id = FaceId(i);
            let Some(f) = self.faces.get(&id) else { continue };
            if bbox_overlap_wrapped(&f.bbox, &probe) && self.face_contains(id, p) {
                return id;
            }
        }
        BACKGROUND
    }

    /// Faces (not the background) whose outer bbox may intersect `bb`.
    pub fn faces_in_bbox(&self, bb: [f64; 4]) -> Vec<FaceId> {
        self.face_index.query(bb).into_iter().map(FaceId).filter(|f| self.faces.get(f).is_some_and(|x| bbox_overlap_wrapped(&x.bbox, &bb))).collect()
    }

    /// Remove an edge record (and its index entry) without touching faces.
    pub(crate) fn remove_edge_raw(&mut self, id: EdgeId) {
        self.edges.remove(&id);
        self.edge_index.remove(id.0);
    }

    /// Remove a face record (and its index entry) without touching edges.
    pub(crate) fn remove_face_raw(&mut self, id: FaceId) {
        self.faces.remove(&id);
        self.face_index.remove(id.0);
    }

    /// Validate only what an edit touched: the given faces' rings (only
    /// rings containing one of `edges`, for faces with many rings such as
    /// the ocean), the given edges' records, and crossings of those edges.
    pub fn validate_local(&self, faces: &std::collections::BTreeSet<FaceId>, edges: &std::collections::BTreeSet<EdgeId>) -> TResult<()> {
        for id in edges {
            let e = self.edges.get(id).ok_or(format!("edge {id:?} missing"))?;
            if !self.nodes.contains_key(&e.a) || !self.nodes.contains_key(&e.b) {
                return Err(format!("edge {id:?} references missing node"));
            }
            if !self.faces.contains_key(&e.left) || !self.faces.contains_key(&e.right) {
                return Err(format!("edge {id:?} references missing face"));
            }
            if e.left == e.right {
                return Err(format!("edge {id:?} has the same face on both sides"));
            }
        }
        let mut seen: HashMap<(EdgeId, bool), usize> = HashMap::new();
        for fid in faces {
            let f = self.faces.get(fid).ok_or(format!("face {fid:?} missing"))?;
            if *fid != BACKGROUND && f.outer.is_empty() {
                return Err(format!("face {fid:?} has no outer ring"));
            }
            for (ri, ring) in std::iter::once(&f.outer).chain(f.holes.iter()).enumerate() {
                if ring.is_empty() || (*fid == BACKGROUND && !ring.iter().any(|r| edges.contains(&r.edge))) {
                    continue;
                }
                for (i, r) in ring.iter().enumerate() {
                    let e = self.edges.get(&r.edge).ok_or(format!("face {fid:?} references missing edge"))?;
                    if (if r.rev { e.right } else { e.left }) != *fid {
                        return Err(format!("face {fid:?} ring {ri} edge {:?} side mismatch", r.edge));
                    }
                    if self.ref_end(*r) != self.ref_start(ring[(i + 1) % ring.len()]) {
                        return Err(format!("face {fid:?} ring {ri} is not connected"));
                    }
                    if edges.contains(&r.edge) {
                        *seen.entry((r.edge, r.rev)).or_default() += 1;
                    }
                }
                if self.is_polar(ring) {
                    continue;
                }
                let area = geom::signed_area(&self.ring_planar(ring));
                let is_outer = ri == 0 && *fid != BACKGROUND;
                if (is_outer && area <= 0.0) || (!is_outer && area >= 0.0) {
                    return Err(format!("face {fid:?} ring {ri} has the wrong orientation"));
                }
            }
        }
        for id in edges {
            let e = &self.edges[id];
            // only edges whose both faces were checked can be counted
            if faces.contains(&e.left) && faces.contains(&e.right) && (seen.get(&(*id, false)) != Some(&1) || seen.get(&(*id, true)) != Some(&1)) {
                return Err(format!("edge {id:?} must be referenced once in each direction"));
            }
        }
        if let Some(c) = self.find_crossing_among(edges) {
            return Err(c);
        }
        Ok(())
    }

    /// Rebuild the spatial indexes from scratch (after deserialisation).
    pub fn rebuild_index(&mut self) {
        self.edge_index = GridIndex::default();
        self.face_index = GridIndex::default();
        let edges: Vec<(EdgeId, [f64; 4])> = self.edges.iter().map(|(id, e)| (*id, e.bbox)).collect();
        for (id, bb) in edges {
            self.edge_index.insert(id.0, bb);
        }
        let faces: Vec<(FaceId, [f64; 4])> = self.faces.iter().filter(|(id, _)| **id != BACKGROUND).map(|(id, f)| (*id, f.bbox)).collect();
        for (id, bb) in faces {
            self.face_index.insert(id.0, bb);
        }
    }

    /// Edges whose bbox may intersect `bb`.
    fn edges_near(&self, bb: [f64; 4]) -> impl Iterator<Item = (EdgeId, &Edge)> + '_ {
        self.edge_index.query(bb).into_iter().filter_map(move |i| self.edges.get(&EdgeId(i)).map(|e| (EdgeId(i), e)))
    }

    /// Spherical area of a face (outer minus holes) on a sphere of radius `r`.
    /// The background's is the sphere minus every other face (O(faces)).
    pub fn face_area(&self, f: FaceId, r: f64) -> f64 {
        if f == BACKGROUND {
            let rest: f64 = self.faces.keys().filter(|k| **k != BACKGROUND).map(|k| self.face_area(*k, r)).sum();
            return (4.0 * std::f64::consts::PI * r * r - rest).max(0.0);
        }
        let face = &self.faces[&f];
        // a revision is never reused for other geometry: its area (on the
        // unit sphere) is worked out once for every state that shares it
        if let Some(a) = area_cache().lock().unwrap().get(&face.rev) {
            return a * r * r;
        }
        let mut a = geom::spherical_area(&self.ring_planar(&face.outer), 1.0);
        for h in &face.holes {
            a -= geom::spherical_area(&self.ring_planar(h), 1.0).abs();
        }
        let mut c = area_cache().lock().unwrap();
        if c.len() > 2_000_000 {
            c.clear();
        }
        c.insert(face.rev, a);
        a * r * r
    }

    pub fn edges_of_node(&self, n: NodeId) -> Vec<EdgeId> {
        let Some(p) = self.nodes.get(&n).map(|x| x.pos) else { return vec![] };
        self.edges_near([p.lon, p.lat, p.lon, p.lat]).filter(|(_, e)| e.a == n || e.b == n).map(|(id, _)| id).collect()
    }

    // --------------------------------------------------------- maintenance

    pub(crate) fn put_edge(&mut self, id: EdgeId, mut e: Edge) {
        let full = self.edge_full(&e);
        e.bbox = geom::bbox(&geom::unwrap_path(&full, None));
        self.edge_index.insert(id.0, e.bbox);
        self.edges.insert(id, e);
    }

    fn refresh_edge_bbox(&mut self, id: EdgeId) {
        let e = self.edges[&id].clone();
        self.put_edge(id, e);
    }

    /// Recompute a face's bbox and give it a new geometry revision.
    pub fn touch(&mut self, f: FaceId) {
        if f == BACKGROUND {
            if let Some(face) = self.faces.get_mut(&BACKGROUND) {
                face.rev = next_rev();
            }
            return;
        }
        let outer = self.ring_planar(&self.faces[&f].outer);
        let bb = geom::bbox(&outer);
        let face = self.faces.get_mut(&f).unwrap();
        face.bbox = bb;
        face.rev = next_rev();
        self.face_index.insert(f.0, bb);
    }

    fn set_side(&mut self, r: ArcRef, f: FaceId) {
        let e = self.edges.get_mut(&r.edge).unwrap();
        if r.rev {
            e.right = f;
        } else {
            e.left = f;
        }
    }

    // ------------------------------------------------------------ queries

    /// Does segment (a,b) cross any existing edge segment? `skip` excludes
    /// specific (edge, segment) pairs.
    fn crosses_any(&self, a: LonLat, b: LonLat, skip: &dyn Fn(EdgeId, usize) -> bool) -> bool {
        let sb = seg_bbox(a, b);
        for (id, e) in self.edges_near(sb) {
            if !bbox_overlap_wrapped(&sb, &e.bbox) {
                continue;
            }
            let full = self.edge_full(e);
            for i in 0..full.len() - 1 {
                if skip(id, i) {
                    continue;
                }
                if !bbox_overlap_wrapped(&sb, &seg_bbox(full[i], full[i + 1])) {
                    continue;
                }
                if geom::geo_segments_intersect(a, b, full[i], full[i + 1]) {
                    return true;
                }
            }
        }
        false
    }

    /// Does segment (a,b) cross an existing edge anywhere but at `ends`,
    /// where it is meant to meet the border? (Border segments through one
    /// of `ends` only touch the line there. Nothing is trimmed off the
    /// line's own ends, so a border that turns across it a hair from where
    /// it starts is still found.)
    fn crosses_except_at(&self, a: LonLat, b: LonLat, ends: &[LonLat]) -> bool {
        let sb = seg_bbox(a, b);
        for (_, e) in self.edges_near(sb) {
            if !bbox_overlap_wrapped(&sb, &e.bbox) {
                continue;
            }
            let full = self.edge_full(e);
            for w in full.windows(2) {
                if !bbox_overlap_wrapped(&sb, &seg_bbox(w[0], w[1])) || ends.iter().any(|p| geom::closest_on_segment(*p, w[0], w[1]).1 < 1e-9) {
                    continue;
                }
                if geom::geo_segments_intersect(a, b, w[0], w[1]) {
                    return true;
                }
            }
        }
        false
    }

    /// Does segment (a,b) pass through an existing vertex (other than at
    /// `ends`)? Such a line does not cross the border there, but pinches
    /// the face it runs in: borders that meet must meet at a node.
    fn touches_vertex(&self, a: LonLat, b: LonLat, ends: &[LonLat]) -> bool {
        const TOL: f64 = 1e-9;
        let sb = seg_bbox(a, b);
        let on = |q: LonLat| geom::closest_on_segment(q, a, b).1 < TOL && !ends.iter().any(|e| geom::geo_dist_deg(*e, q) < TOL);
        for (_, e) in self.edges_near(sb) {
            if !bbox_overlap_wrapped(&sb, &e.bbox) {
                continue;
            }
            if on(self.nodes[&e.a].pos) || on(self.nodes[&e.b].pos) || e.pts.iter().any(|q| on(*q)) {
                return true;
            }
        }
        false
    }

    /// Nearest vertex within `tol` degrees.
    pub fn nearest_vertex(&self, p: LonLat, tol: f64) -> Option<(VertexRef, LonLat)> {
        let mut best: Option<(f64, VertexRef, LonLat)> = None;
        let probe = [p.lon - tol, p.lat - tol, p.lon + tol, p.lat + tol];
        // every node is an endpoint of some edge, so nearby edges cover nodes too
        for (id, e) in self.edges_near(probe) {
            if !bbox_overlap_wrapped(&probe, &e.bbox) {
                continue;
            }
            for n in [e.a, e.b] {
                let q = self.nodes[&n].pos;
                let d = geom::geo_dist_deg(p, q);
                if d <= tol && best.map_or(true, |b| d < b.0) {
                    best = Some((d, VertexRef::Node(n), q));
                }
            }
            for (i, q) in e.pts.iter().enumerate() {
                let d = geom::geo_dist_deg(p, *q);
                if d <= tol && best.map_or(true, |b| d < b.0) {
                    best = Some((d, VertexRef::Pt { edge: id, idx: i }, *q));
                }
            }
        }
        best.map(|b| (b.1, b.2))
    }

    /// Nearest point on any edge within `tol` degrees: (edge, segment index in
    /// full coords, projected point).
    pub fn nearest_on_edge(&self, p: LonLat, tol: f64) -> Option<(EdgeId, usize, LonLat)> {
        let mut best: Option<(f64, EdgeId, usize, LonLat)> = None;
        let probe = [p.lon - tol, p.lat - tol, p.lon + tol, p.lat + tol];
        for (id, e) in self.edges_near(probe) {
            if !bbox_overlap_wrapped(&probe, &e.bbox) {
                continue;
            }
            let full = self.edge_full(e);
            for i in 0..full.len() - 1 {
                let (q, d, _) = geom::closest_on_segment(p, full[i], full[i + 1]);
                if d <= tol && best.map_or(true, |b| d < b.0) {
                    best = Some((d, id, i, q));
                }
            }
        }
        best.map(|b| (b.1, b.2, b.3))
    }

    /// First point where the segment a→b crosses any boundary (excluding a itself).
    pub fn first_hit(&self, a: LonLat, b: LonLat) -> Option<LonLat> {
        let a2 = [a.lon, a.lat];
        let b2 = [a.lon + geom::lon_delta(a.lon, b.lon), b.lat];
        let sb = seg_bbox(a, b);
        let mut best: Option<(f64, P2)> = None;
        for (_, e) in self.edges_near(sb) {
            if !bbox_overlap_wrapped(&sb, &e.bbox) {
                continue;
            }
            let full = self.edge_full(e);
            for i in 0..full.len() - 1 {
                let c = [full[i].lon, full[i].lat];
                let d0 = [full[i].lon + geom::lon_delta(full[i].lon, full[i + 1].lon), full[i + 1].lat];
                for k in [-360.0, 0.0, 360.0] {
                    let (c2, d2) = ([c[0] + k, c[1]], [d0[0] + k, d0[1]]);
                    if let Some((t, p)) = geom::segment_intersection(a2, b2, c2, d2) {
                        if t > 1e-9 && best.map_or(true, |x| t < x.0) {
                            best = Some((t, p));
                        }
                    }
                }
            }
        }
        best.map(|(_, p)| LonLat::new(geom::wrap_lon(p[0]), p[1]))
    }

    /// Snap a point to the boundary: existing vertex first, then edge.
    pub fn snap_boundary(&self, p: LonLat, tol: f64) -> Option<LonLat> {
        if let Some((_, q)) = self.nearest_vertex(p, tol) {
            return Some(q);
        }
        self.nearest_on_edge(p, tol).map(|(_, _, q)| q)
    }

    pub fn vertex_exists(&self, v: VertexRef) -> bool {
        match v {
            VertexRef::Node(n) => self.nodes.contains_key(&n),
            VertexRef::Pt { edge, idx } => self.edges.get(&edge).is_some_and(|e| idx < e.pts.len()),
        }
    }

    pub fn position_of(&self, v: VertexRef) -> LonLat {
        match v {
            VertexRef::Node(n) => self.node_pos(n),
            VertexRef::Pt { edge, idx } => self.edges[&edge].pts[idx],
        }
    }

    // ---------------------------------------------------------- operations

    /// Split an edge at a point lying on segment `seg` (index into full
    /// coords). If the point coincides with an existing interior vertex the
    /// split happens there. Returns the new node.
    pub fn split_edge(&mut self, eid: EdgeId, seg: usize, pos: LonLat) -> NodeId {
        let e = self.edges[&eid].clone();
        let full = self.edge_full(&e);
        let eps = 1e-12;
        let (before, after, npos) = if geom::geo_dist_deg(full[seg], pos) < eps && seg > 0 {
            (full[1..seg].to_vec(), full[seg + 1..full.len() - 1].to_vec(), full[seg])
        } else if geom::geo_dist_deg(full[seg + 1], pos) < eps && seg + 1 < full.len() - 1 {
            (full[1..seg + 1].to_vec(), full[seg + 2..full.len() - 1].to_vec(), full[seg + 1])
        } else {
            (full[1..seg + 1].to_vec(), full[seg + 1..full.len() - 1].to_vec(), pos)
        };
        let n = NodeId(self.alloc());
        self.nodes.insert(n, Node { pos: npos.normalized() });
        let e2id = EdgeId(self.alloc());
        self.put_edge(eid, Edge { a: e.a, b: n, pts: before, left: e.left, right: e.right, bbox: [0.0; 4] });
        self.put_edge(e2id, Edge { a: n, b: e.b, pts: after, left: e.left, right: e.right, bbox: [0.0; 4] });
        for f in [e.left, e.right] {
            let mut face = self.faces[&f].clone();
            let fix = |ring: &mut Ring| {
                let mut out = Vec::with_capacity(ring.len() + 1);
                for r in ring.iter() {
                    if r.edge == eid {
                        if r.rev {
                            out.push(ArcRef { edge: e2id, rev: true });
                            out.push(ArcRef { edge: eid, rev: true });
                        } else {
                            out.push(ArcRef { edge: eid, rev: false });
                            out.push(ArcRef { edge: e2id, rev: false });
                        }
                    } else {
                        out.push(*r);
                    }
                }
                *ring = out;
            };
            fix(&mut face.outer);
            face.holes.iter_mut().for_each(fix);
            self.faces.insert(f, face);
        }
        self.touch(e.left);
        self.touch(e.right);
        n
    }

    /// Return the node at `pos`, splitting an edge if `pos` lies on one.
    fn ensure_node(&mut self, pos: LonLat) -> TResult<NodeId> {
        if let Some((VertexRef::Node(n), q)) = self.nearest_vertex(pos, 1e-9) {
            if geom::geo_dist_deg(q, pos) < 1e-9 {
                return Ok(n);
            }
        }
        match self.nearest_on_edge(pos, 1e-7) {
            Some((e, seg, q)) => Ok(self.split_edge(e, seg, q)),
            None => Err("端点必须位于已有边界上".into()),
        }
    }

    fn clean_points(pts: &[LonLat]) -> Vec<LonLat> {
        let mut out: Vec<LonLat> = Vec::with_capacity(pts.len());
        for p in pts {
            let p = p.normalized();
            if out.last().map_or(true, |q| geom::geo_dist_deg(*q, p) > 1e-12) {
                out.push(p);
            }
        }
        out
    }

    /// Add a closed ring as a new face inside whichever face contains it.
    /// Existing rings enclosed by the new ring become its holes.
    pub fn add_ring_face(&mut self, pts: &[LonLat], kind: FaceKind) -> TResult<FaceId> {
        let mut pts = Self::clean_points(pts);
        while pts.len() > 1 && geom::geo_dist_deg(pts[0], *pts.last().unwrap()) < 1e-12 {
            pts.pop();
        }
        if pts.len() < 3 {
            return Err("多边形至少需要 3 个点".into());
        }
        if geom::ring_winding(&pts).abs() > 180.0 {
            return Err("暂不支持绘制包围极点的环".into());
        }
        let mut planar = geom::ring_to_planar(&pts);
        if !geom::ring_is_simple(&planar) {
            return Err("多边形自相交".into());
        }
        if geom::signed_area(&planar) < 0.0 {
            pts.reverse();
            planar = geom::ring_to_planar(&pts);
        }
        let n = pts.len();
        for i in 0..n {
            if self.crosses_any(pts[i], pts[(i + 1) % n], &|_, _| false) {
                return Err("新多边形与已有边界相交（P1 将支持自动合并/裁剪）".into());
            }
        }
        let container = self.face_at(pts[0]);
        let node = NodeId(self.alloc());
        self.nodes.insert(node, Node { pos: pts[0] });
        let fid = FaceId(self.alloc());
        let eid = EdgeId(self.alloc());
        self.put_edge(eid, Edge { a: node, b: node, pts: pts[1..].to_vec(), left: fid, right: container, bbox: [0.0; 4] });
        let pb = geom::bbox(&planar);
        // adopt enclosed rings of the container
        let mut cont = self.faces[&container].clone();
        let mut adopted = Vec::new();
        cont.holes.retain(|h| {
            let s = self.ring_coords(h)[0];
            if geom::point_in_poly_wrapped([s.lon, s.lat], &planar, &pb) {
                adopted.push(h.clone());
                false
            } else {
                true
            }
        });
        for h in &adopted {
            for r in h {
                self.set_side(*r, fid);
            }
        }
        cont.holes.push(vec![ArcRef { edge: eid, rev: true }]);
        self.faces.insert(container, cont);
        self.faces.insert(fid, Face { kind, outer: vec![ArcRef { edge: eid, rev: false }], holes: adopted, rev: 0, bbox: pb });
        self.touch(fid);
        self.touch(container);
        Ok(fid)
    }

    /// Split a face with a polyline whose first and last points lie on the
    /// face's outer boundary. Returns (kept face, new face); the larger part
    /// keeps the original id.
    pub fn split_face(&mut self, cut: &[LonLat], radius: f64) -> TResult<(FaceId, FaceId)> {
        let cut = Self::clean_points(cut);
        if cut.len() < 2 {
            return Err("切割线至少需要 2 个点".into());
        }
        let m = cut.len();
        // face: the one containing the middle of the first segment
        let mid = {
            let d = geom::lon_delta(cut[0].lon, cut[1].lon);
            LonLat::new(geom::wrap_lon(cut[0].lon + d * 0.5), (cut[0].lat + cut[1].lat) * 0.5)
        };
        let f = self.face_at(mid);
        if f == BACKGROUND {
            return Err("背景面不能直接切分：先用绘制陆地/湖泊工具画出一块区域".into());
        }
        for p in &cut[1..m - 1] {
            if !self.face_contains(f, *p) {
                return Err("切割线的中间点必须位于同一个面内".into());
            }
        }
        if !geom::path_is_simple(&geom::unwrap_path(&cut, None)) {
            return Err("切割线自相交".into());
        }
        let ends = [cut[0], cut[m - 1]];
        for i in 0..m - 1 {
            let (a, b) = (cut[i], cut[i + 1]);
            // half way round the globe in one step has no shorter way
            if geom::lon_delta(a.lon, b.lon).abs() > 180.0 - 1e-9 {
                return Err("切割线的一段横跨了半个地球，走向不明确：请在中间加一个点".into());
            }
            if self.crosses_except_at(a, b, &ends) {
                return Err("切割线与已有边界相交".into());
            }
            if self.touches_vertex(cut[i], cut[i + 1], &[cut[0], cut[m - 1]]) {
                return Err("切割线贴着已有边界的顶点经过：请让它在那里结束，或绕开".into());
            }
        }
        // both endpoints must be on this face's outer ring
        let outer_coords = self.ring_coords(&self.faces[&f].outer);
        let on_outer = |p: LonLat| {
            let k = outer_coords.len();
            (0..k).any(|i| geom::closest_on_segment(p, outer_coords[i], outer_coords[(i + 1) % k]).1 < 1e-7)
        };
        if !on_outer(cut[0]) || !on_outer(cut[m - 1]) {
            return Err("切割线两端必须吸附到该面的外边界上".into());
        }
        if geom::geo_dist_deg(cut[0], cut[m - 1]) < 1e-9 {
            return Err("切割线的两端不能是同一点".into());
        }

        let n1 = self.ensure_node(cut[0])?;
        let n2 = self.ensure_node(cut[m - 1])?;
        let face = self.faces[&f].clone();
        let starts: Vec<NodeId> = face.outer.iter().map(|r| self.ref_start(*r)).collect();
        let i1 = starts.iter().position(|n| *n == n1).ok_or("端点不在外边界上")?;
        let i2 = starts.iter().position(|n| *n == n2).ok_or("端点不在外边界上")?;
        let k = face.outer.len();
        let path = |from: usize, to: usize| {
            let mut v = Vec::new();
            let mut i = from;
            while i != to {
                v.push(face.outer[i]);
                i = (i + 1) % k;
            }
            v
        };
        let p1 = path(i1, i2); // n1 -> n2
        let p2 = path(i2, i1); // n2 -> n1
        let cid = EdgeId(self.alloc());
        self.put_edge(cid, Edge { a: n1, b: n2, pts: cut[1..m - 1].to_vec(), left: f, right: f, bbox: [0.0; 4] });
        let mut ring_a = p2;
        ring_a.push(ArcRef { edge: cid, rev: false });
        let mut ring_b = p1;
        ring_b.push(ArcRef { edge: cid, rev: true });
        let pa = self.ring_planar(&ring_a);
        let pb = self.ring_planar(&ring_b);
        if geom::signed_area(&pa) <= 0.0 || geom::signed_area(&pb) <= 0.0 {
            return Err("切割结果方向异常".into());
        }
        let (keep, give, give_planar) = if geom::spherical_area(&pa, radius) >= geom::spherical_area(&pb, radius) {
            (ring_a, ring_b, pb)
        } else {
            (ring_b, ring_a, pa)
        };
        let g = FaceId(self.alloc());
        for r in &give {
            self.set_side(*r, g);
        }
        let gb = geom::bbox(&give_planar);
        let mut kept_holes = Vec::new();
        let mut given_holes = Vec::new();
        for h in face.holes {
            let s = self.ring_coords(&h)[0];
            if geom::point_in_poly_wrapped([s.lon, s.lat], &give_planar, &gb) {
                for r in &h {
                    self.set_side(*r, g);
                }
                given_holes.push(h);
            } else {
                kept_holes.push(h);
            }
        }
        self.faces.insert(f, Face { kind: face.kind, outer: keep, holes: kept_holes, rev: 0, bbox: [0.0; 4] });
        self.faces.insert(g, Face { kind: face.kind, outer: give, holes: given_holes, rev: 0, bbox: [0.0; 4] });
        self.touch(f);
        self.touch(g);
        Ok((f, g))
    }

    /// Move a vertex. Shared boundaries move for both neighbours. Fails (and
    /// leaves `self` partially modified – callers work on a snapshot) if the
    /// move would create a crossing or break ring nesting.
    pub fn move_vertex(&mut self, v: VertexRef, pos: LonLat) -> TResult<()> {
        let pos = pos.normalized();
        let mut changed: Vec<(EdgeId, usize)> = Vec::new();
        match v {
            VertexRef::Node(n) => {
                // look up incident edges *before* moving: the index finds
                // them by the node's current position
                let incident = self.edges_of_node(n);
                self.nodes.get_mut(&n).ok_or("节点不存在")?.pos = pos;
                for eid in incident {
                    let e = &self.edges[&eid];
                    let last = e.pts.len();
                    if e.a == n {
                        changed.push((eid, 0));
                    }
                    if e.b == n {
                        changed.push((eid, last));
                    }
                }
            }
            VertexRef::Pt { edge, idx } => {
                let e = self.edges.get_mut(&edge).ok_or("边不存在")?;
                *e.pts.get_mut(idx).ok_or("顶点不存在")? = pos;
                changed.push((edge, idx));
                changed.push((edge, idx + 1));
            }
        }
        let edges: HashSet<EdgeId> = changed.iter().map(|c| c.0).collect();
        for e in &edges {
            self.refresh_edge_bbox(*e);
        }
        self.check_segments(&changed)?;
        self.after_geometry_change(&edges)
    }

    /// Replace the interior points of an edge; its end nodes stay. Checked
    /// like a vertex move (no crossings, holes stay inside, islands do not
    /// swallow each other); on error the caller restores its copy.
    pub fn replace_edge_points(&mut self, id: EdgeId, pts: Vec<LonLat>) -> TResult<()> {
        let mut e = self.edges.get(&id).ok_or("边不存在")?.clone();
        e.pts = pts.into_iter().map(|p| p.normalized()).collect();
        let segs = e.pts.len() + 1;
        self.put_edge(id, e);
        let changed: Vec<(EdgeId, usize)> = (0..segs).map(|i| (id, i)).collect();
        self.check_segments(&changed)?;
        self.after_geometry_change(&HashSet::from([id]))
    }

    /// Insert a vertex on segment `seg` (full-coordinate index) of an edge.
    pub fn insert_vertex(&mut self, edge: EdgeId, seg: usize, pos: LonLat) -> TResult<VertexRef> {
        let e = self.edges.get_mut(&edge).ok_or("边不存在")?;
        if seg > e.pts.len() {
            return Err("线段索引越界".into());
        }
        e.pts.insert(seg, pos.normalized());
        let (l, r) = (e.left, e.right);
        self.refresh_edge_bbox(edge);
        self.touch(l);
        self.touch(r);
        Ok(VertexRef::Pt { edge, idx: seg })
    }

    pub fn delete_vertex(&mut self, v: VertexRef) -> TResult<()> {
        let VertexRef::Pt { edge, idx } = v else {
            return Err("暂不支持删除交汇节点".into());
        };
        let e = self.edges.get_mut(&edge).ok_or("边不存在")?;
        let min_pts = if e.a == e.b { 3 } else { 1 };
        if e.pts.len() < min_pts {
            return Err("删除后边界将退化".into());
        }
        e.pts.remove(idx);
        self.refresh_edge_bbox(edge);
        self.check_segments(&[(edge, idx)])?;
        self.after_geometry_change(&HashSet::from([edge]))
    }

    fn check_segments(&self, changed: &[(EdgeId, usize)]) -> TResult<()> {
        if changed.len() > 32 {
            return self.check_segments_batched(changed);
        }
        for &(eid, seg) in changed {
            let full = self.edge_full(&self.edges[&eid]);
            if seg + 1 >= full.len() {
                continue;
            }
            let (a, b) = shrink(full[seg], full[seg + 1], true, true);
            if self.crosses_any(a, b, &|e, i| e == eid && i == seg) {
                return Err("编辑会导致边界相交".into());
            }
        }
        Ok(())
    }

    /// [`Topology::check_segments`] for many segments at once (a whole
    /// redrawn edge): the changed segments go into a grid, and each segment
    /// of every edge near them is tested only against those it may touch.
    fn check_segments_batched(&self, changed: &[(EdgeId, usize)]) -> TResult<()> {
        let mut segs: Vec<(LonLat, LonLat)> = Vec::new();
        let mut which: Vec<(EdgeId, usize)> = Vec::new();
        let mut fulls: HashMap<EdgeId, Vec<LonLat>> = HashMap::new();
        for &(eid, seg) in changed {
            let full = fulls.entry(eid).or_insert_with(|| self.edge_full(&self.edges[&eid]));
            if seg + 1 >= full.len() {
                continue;
            }
            segs.push(shrink(full[seg], full[seg + 1], true, true));
            which.push((eid, seg));
        }
        let pts: Vec<([f64; 2], [f64; 2])> = segs.iter().map(|(a, b)| ([a.lon, a.lat], [b.lon, b.lat])).collect();
        let mut grid = crate::seggrid::SegGrid::new(&pts, 1e-9, true);
        let mut near_edges: std::collections::BTreeSet<EdgeId> = std::collections::BTreeSet::new();
        for eid in fulls.keys() {
            let bb = self.edges[eid].bbox;
            near_edges.extend(self.edges_near(bb).filter(|(_, o)| bbox_overlap_wrapped(&bb, &o.bbox)).map(|(id, _)| id));
        }
        let mut near = Vec::new();
        for oid in near_edges {
            let full = self.edge_full(&self.edges[&oid]);
            for j in 0..full.len() - 1 {
                let (c, d) = (full[j], full[j + 1]);
                grid.near([c.lon, c.lat], [d.lon, d.lat], &mut near);
                for &i in &near {
                    if which[i as usize] == (oid, j) {
                        continue;
                    }
                    let (a, b) = segs[i as usize];
                    if bbox_overlap_wrapped(&seg_bbox(a, b), &seg_bbox(c, d)) && geom::geo_segments_intersect(a, b, c, d) {
                        return Err("编辑会导致边界相交".into());
                    }
                }
            }
        }
        Ok(())
    }

    fn after_geometry_change(&mut self, edges: &HashSet<EdgeId>) -> TResult<()> {
        let mut faces: HashSet<FaceId> = HashSet::new();
        for e in edges {
            let e = &self.edges[e];
            faces.insert(e.left);
            faces.insert(e.right);
        }
        for f in &faces {
            self.touch(*f);
        }
        for f in &faces {
            self.check_nesting(*f)?;
        }
        Ok(())
    }

    /// Holes must stay inside their face, and an island must not swallow a
    /// sibling island.
    fn check_nesting(&self, f: FaceId) -> TResult<()> {
        let face = &self.faces[&f];
        // A vertex jumping across its face can leave a simple but inverted
        // ring, which a final-state crossing test cannot see.
        for h in &face.holes {
            if !self.is_polar(h) && geom::signed_area(&self.ring_planar(h)) >= 0.0 {
                return Err("编辑会使边界翻转".into());
            }
        }
        if f != BACKGROUND {
            let outer = self.ring_planar(&face.outer);
            if !self.is_polar(&face.outer) && geom::signed_area(&outer) <= 0.0 {
                return Err("编辑会使边界翻转".into());
            }
            let bb = geom::bbox(&outer);
            for h in &face.holes {
                let s = self.ring_coords(h)[0];
                if !geom::point_in_poly_wrapped([s.lon, s.lat], &outer, &bb) {
                    return Err("编辑会使内部的岛/湖跑到面外".into());
                }
            }
        }
        // islands sitting in this face: their outer ring must not contain other holes
        let samples: Vec<LonLat> = face.holes.iter().map(|h| self.ring_coords(h)[0]).collect();
        for (hi, h) in face.holes.iter().enumerate() {
            let other: HashSet<FaceId> = h
                .iter()
                .map(|r| {
                    let e = &self.edges[&r.edge];
                    if r.rev {
                        e.left
                    } else {
                        e.right
                    }
                })
                .collect();
            if other.len() != 1 {
                continue;
            }
            let mut c = self.ring_coords(h);
            c.reverse();
            let hp = geom::ring_to_planar(&c);
            let hb = geom::bbox(&hp);
            for (si, s) in samples.iter().enumerate() {
                if si != hi && geom::point_in_poly_wrapped([s.lon, s.lat], &hp, &hb) {
                    return Err("编辑会使一个岛包住另一个岛".into());
                }
            }
        }
        Ok(())
    }

    /// Merge face `b` into face `a` by deleting their shared edges. A
    /// merge that cannot be done leaves the topology exactly as it was.
    pub fn merge_faces(&mut self, a: FaceId, b: FaceId) -> TResult<FaceId> {
        // worked out on a copy (cheap: the maps share their contents)
        let mut t = self.clone();
        let f = t.merge_faces_in_place(a, b)?;
        *self = t;
        Ok(f)
    }

    fn merge_faces_in_place(&mut self, a: FaceId, b: FaceId) -> TResult<FaceId> {
        if a == b || a == BACKGROUND || b == BACKGROUND {
            return Err("无法合并这两个面".into());
        }
        let fa0 = self.faces.get(&a).ok_or("面不存在")?;
        let shared: HashSet<EdgeId> = std::iter::once(&fa0.outer)
            .chain(fa0.holes.iter())
            .flatten()
            .filter(|r| {
                let e = &self.edges[&r.edge];
                (e.left == a && e.right == b) || (e.left == b && e.right == a)
            })
            .map(|r| r.edge)
            .collect();
        if shared.is_empty() {
            return Err("两个面不相邻".into());
        }
        let fa = self.faces[&a].clone();
        let fb = self.faces[&b].clone();
        let mut refs = Vec::new();
        for ring in std::iter::once(&fa.outer).chain(fa.holes.iter()).chain(std::iter::once(&fb.outer)).chain(fb.holes.iter()) {
            for r in ring {
                if !shared.contains(&r.edge) {
                    refs.push(*r);
                }
            }
        }
        for r in &refs {
            self.set_side(*r, a);
        }
        let rings = self.assemble_rings(&refs)?;
        // land around a pole, cut along meridians, joined up again: the
        // pieces' edges on the pole are left as a ring that encloses
        // nothing. They go, and with them the matching ring on their far side.
        let (at_pole, rings): (Vec<Ring>, Vec<Ring>) = rings.into_iter().partition(|ring| ring.iter().all(|r| self.on_pole(&self.edges[&r.edge])));
        let gone: HashSet<EdgeId> = at_pole.iter().flatten().map(|r| r.edge).collect();
        if !gone.is_empty() {
            let far: HashSet<FaceId> = gone.iter().map(|e| if self.edges[e].left == a { self.edges[e].right } else { self.edges[e].left }).collect();
            for o in far {
                let mut f = self.faces.get(&o).ok_or("面不存在")?.clone();
                f.holes.retain(|h| !h.iter().all(|r| gone.contains(&r.edge)));
                if std::iter::once(&f.outer).chain(f.holes.iter()).flatten().any(|r| gone.contains(&r.edge)) {
                    return Err("合并会在极点处留下无法处理的边界".into());
                }
                self.faces.insert(o, f);
                self.touch(o);
            }
        }
        let mut outer: Option<(f64, Ring)> = None;
        let mut holes = Vec::new();
        for ring in rings {
            let area = geom::signed_area(&self.ring_planar(&ring));
            if area > 0.0 {
                if outer.is_some() {
                    return Err("合并结果不是单一面".into());
                }
                outer = Some((area, ring));
            } else {
                holes.push(ring);
            }
        }
        let outer = outer.ok_or("合并结果没有外边界")?.1;
        for e in shared.iter().chain(gone.iter()) {
            self.edges.remove(e);
            self.edge_index.remove(e.0);
        }
        self.remove_orphan_nodes();
        self.faces.remove(&b);
        self.face_index.remove(b.0);
        self.faces.insert(a, Face { kind: fa.kind, outer, holes, rev: 0, bbox: [0.0; 4] });
        self.touch(a);
        Ok(a)
    }

    fn remove_orphan_nodes(&mut self) {
        let used: HashSet<NodeId> = self.edges.values().flat_map(|e| [e.a, e.b]).collect();
        let orphans: Vec<NodeId> = self.nodes.keys().filter(|n| !used.contains(n)).copied().collect();
        for n in orphans {
            self.nodes.remove(&n);
        }
    }

    /// Chain directed arc references into closed rings.
    pub fn assemble_rings(&self, refs: &[ArcRef]) -> TResult<Vec<Ring>> {
        let mut by_start: HashMap<NodeId, Vec<usize>> = HashMap::new();
        for (i, r) in refs.iter().enumerate() {
            by_start.entry(self.ref_start(*r)).or_default().push(i);
        }
        let mut used = vec![false; refs.len()];
        let mut rings = Vec::new();
        for s in 0..refs.len() {
            if used[s] {
                continue;
            }
            used[s] = true;
            let start = self.ref_start(refs[s]);
            let mut ring = vec![refs[s]];
            let mut end = self.ref_end(refs[s]);
            while end != start {
                let next = by_start
                    .get(&end)
                    .and_then(|c| c.iter().copied().find(|i| !used[*i]))
                    .ok_or("边界无法闭合")?;
                used[next] = true;
                ring.push(refs[next]);
                end = self.ref_end(refs[next]);
            }
            rings.push(ring);
        }
        Ok(rings)
    }

    // ---------------------------------------------------------- validation

    pub fn validate(&self) -> TResult<()> {
        // spatial index must match the records exactly
        if self.edge_index.len() != self.edges.len() || self.face_index.len() + 1 != self.faces.len() {
            return Err(format!(
                "spatial index out of sync: {} indexed edges for {}, {} indexed faces for {}",
                self.edge_index.len(),
                self.edges.len(),
                self.face_index.len(),
                self.faces.len() - 1
            ));
        }
        for (id, e) in self.edges.iter() {
            let actual = geom::bbox(&geom::unwrap_path(&self.edge_full(e), None));
            if actual != e.bbox {
                return Err(format!("edge {id:?} has a stale bbox"));
            }
            if self.edge_index.query(e.bbox).binary_search(&id.0).is_err() {
                return Err(format!("edge {id:?} missing from spatial index"));
            }
        }
        for (id, f) in self.faces.iter() {
            if *id != BACKGROUND && self.face_index.query(f.bbox).binary_search(&id.0).is_err() {
                return Err(format!("face {id:?} missing from spatial index"));
            }
        }
        let mut fwd: HashMap<EdgeId, usize> = HashMap::new();
        let mut bwd: HashMap<EdgeId, usize> = HashMap::new();
        for (id, e) in self.edges.iter() {
            if !self.nodes.contains_key(&e.a) || !self.nodes.contains_key(&e.b) {
                return Err(format!("edge {id:?} references missing node"));
            }
            if !self.faces.contains_key(&e.left) || !self.faces.contains_key(&e.right) {
                return Err(format!("edge {id:?} references missing face"));
            }
            if e.left == e.right {
                let p = self.node_pos(e.a);
                return Err(format!("edge {id:?} has the same face on both sides (near {:.4}, {:.4})", p.lon, p.lat));
            }
        }
        for (fid, f) in self.faces.iter() {
            if *fid == BACKGROUND && !f.outer.is_empty() {
                return Err("ocean must not have an outer ring".into());
            }
            if *fid != BACKGROUND && f.outer.is_empty() {
                return Err(format!("face {fid:?} has no outer ring"));
            }
            for (ri, ring) in std::iter::once(&f.outer).chain(f.holes.iter()).enumerate() {
                if ring.is_empty() {
                    continue;
                }
                for (i, r) in ring.iter().enumerate() {
                    let e = self.edges.get(&r.edge).ok_or(format!("face {fid:?} references missing edge"))?;
                    let side = if r.rev { e.right } else { e.left };
                    if side != *fid {
                        return Err(format!("face {fid:?} ring {ri} edge {:?} side mismatch", r.edge));
                    }
                    let next = ring[(i + 1) % ring.len()];
                    if self.ref_end(*r) != self.ref_start(next) {
                        return Err(format!("face {fid:?} ring {ri} is not connected"));
                    }
                    *(if r.rev { &mut bwd } else { &mut fwd }).entry(r.edge).or_default() += 1;
                }
                let is_outer = ri == 0 && *fid != BACKGROUND;
                if self.is_polar(ring) {
                    // winding direction says which pole is inside; there is
                    // no clockwise/counter-clockwise test for such rings
                    continue;
                }
                let area = geom::signed_area(&self.ring_planar(ring));
                if is_outer && area <= 0.0 {
                    return Err(format!("face {fid:?} outer ring is not counter-clockwise"));
                }
                if !is_outer && area >= 0.0 {
                    return Err(format!("face {fid:?} hole is not clockwise"));
                }
            }
        }
        for id in self.edges.keys() {
            if fwd.get(id) != Some(&1) || bwd.get(id) != Some(&1) {
                return Err(format!("edge {id:?} must be referenced once in each direction"));
            }
        }
        if let Some(e) = self.find_crossing() {
            return Err(e);
        }
        Ok(())
    }

    /// First pair of boundary segments that cross (segments sharing an
    /// endpoint are not compared), or None for a proper planar embedding.
    pub fn find_crossing(&self) -> Option<String> {
        self.edges.keys().find_map(|id| self.crossing_of(*id, true))
    }

    /// Crossings involving only the given edges (against everything).
    pub fn find_crossing_among(&self, edges: &std::collections::BTreeSet<EdgeId>) -> Option<String> {
        let segments: usize = edges.iter().filter_map(|id| self.edges.get(id)).map(|e| e.pts.len() + 1).sum();
        if segments <= 64 {
            return edges.iter().find_map(|id| self.crossing_of(*id, false));
        }
        // many segments (finely drawn edges): through a grid of them
        let same = |p: LonLat, q: LonLat| geom::geo_dist_deg(p, q) < 1e-12;
        let mut segs: Vec<(LonLat, LonLat)> = Vec::new();
        let mut which: Vec<(EdgeId, usize)> = Vec::new();
        let mut near_edges: std::collections::BTreeSet<EdgeId> = std::collections::BTreeSet::new();
        for id in edges {
            let Some(e) = self.edges.get(id) else { continue };
            let full = self.edge_full(e);
            for i in 0..full.len() - 1 {
                segs.push((full[i], full[i + 1]));
                which.push((*id, i));
            }
            near_edges.extend(self.edges_near(e.bbox).filter(|(_, o)| bbox_overlap_wrapped(&e.bbox, &o.bbox)).map(|(oid, _)| oid));
        }
        let pts: Vec<([f64; 2], [f64; 2])> = segs.iter().map(|(a, b)| ([a.lon, a.lat], [b.lon, b.lat])).collect();
        let mut grid = crate::seggrid::SegGrid::new(&pts, 1e-9, true);
        let mut near = Vec::new();
        for oid in near_edges {
            let full = self.edge_full(&self.edges[&oid]);
            for j in 0..full.len() - 1 {
                let (c, d) = (full[j], full[j + 1]);
                grid.near([c.lon, c.lat], [d.lon, d.lat], &mut near);
                for &i in &near {
                    let (id, si) = which[i as usize];
                    if id == oid && si == j {
                        continue;
                    }
                    let (a, b) = segs[i as usize];
                    if same(a, c) || same(a, d) || same(b, c) || same(b, d) {
                        continue;
                    }
                    if bbox_overlap_wrapped(&seg_bbox(a, b), &seg_bbox(c, d)) && geom::geo_segments_intersect(a, b, c, d) {
                        return Some(format!("边界相交：边 {} 与边 {}，约 {:.5}°, {:.5}°", id.0, oid.0, a.lon, a.lat));
                    }
                }
            }
        }
        None
    }

    /// First crossing between edge `id` and any edge near it. With
    /// `only_higher`, other edges with smaller ids are skipped (each pair is
    /// then tested once when iterating all edges).
    fn crossing_of(&self, id: EdgeId, only_higher: bool) -> Option<String> {
        let same = |p: LonLat, q: LonLat| geom::geo_dist_deg(p, q) < 1e-12;
        let id = &id;
        let e = self.edges.get(id)?;
        {
            let full = self.edge_full(e);
            // one index query per edge; candidates' coordinates fetched once
            let cands: Vec<(EdgeId, [f64; 4], Vec<LonLat>)> = self
                .edges_near(e.bbox)
                .filter(|(oid, o)| (!only_higher || *oid >= *id) && bbox_overlap_wrapped(&e.bbox, &o.bbox))
                .map(|(oid, o)| (oid, o.bbox, if oid == *id { full.clone() } else { self.edge_full(o) }))
                .collect();
            for i in 0..full.len() - 1 {
                let (a, b) = (full[i], full[i + 1]);
                let sb = seg_bbox(a, b);
                for (oid, obb, of) in &cands {
                    if !bbox_overlap_wrapped(&sb, obb) {
                        continue;
                    }
                    let start = if oid == id { i + 1 } else { 0 };
                    for j in start..of.len() - 1 {
                        let (c, d) = (of[j], of[j + 1]);
                        if !bbox_overlap_wrapped(&sb, &seg_bbox(c, d)) {
                            continue;
                        }
                        if same(a, c) || same(a, d) || same(b, c) || same(b, d) {
                            continue;
                        }
                        if geom::geo_segments_intersect(a, b, c, d) {
                            return Some(format!("边界相交：边 {} 与边 {}，约 {:.5}°, {:.5}°", id.0, oid.0, a.lon, a.lat));
                        }
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: f64, y0: f64, s: f64) -> Vec<LonLat> {
        vec![LonLat::new(x0, y0), LonLat::new(x0 + s, y0), LonLat::new(x0 + s, y0 + s), LonLat::new(x0, y0 + s)]
    }

    #[test]
    fn island_split_move_merge() {
        let mut t = Topology::default();
        let f = t.add_ring_face(&square(0.0, 0.0, 10.0), FaceKind::Land).unwrap();
        t.validate().unwrap();
        assert_eq!(t.face_at(LonLat::new(5.0, 5.0)), f);
        assert_eq!(t.face_at(LonLat::new(15.0, 5.0)), BACKGROUND);
        // split vertically
        let (a, b) = t.split_face(&[LonLat::new(5.0, 0.0), LonLat::new(5.0, 5.0), LonLat::new(5.0, 10.0)], 6371.0).unwrap();
        t.validate().unwrap();
        assert_ne!(a, b);
        let total = t.face_area(a, 6371.0) + t.face_area(b, 6371.0);
        let whole = geom::spherical_area(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]], 6371.0);
        assert!((total - whole).abs() / whole < 1e-9);
        // move a shared vertex: both faces change, topology stays valid
        let (v, _) = t.nearest_vertex(LonLat::new(5.0, 5.0), 0.1).unwrap();
        t.move_vertex(v, LonLat::new(6.0, 5.0)).unwrap();
        t.validate().unwrap();
        let total2 = t.face_area(a, 6371.0) + t.face_area(b, 6371.0);
        assert!((total2 - whole).abs() / whole < 1e-9);
        // crossing move is rejected
        let mut t2 = t.clone();
        assert!(t2.move_vertex(v, LonLat::new(12.0, 5.0)).is_err());
        // merge back
        t.merge_faces(a, b).unwrap();
        t.validate().unwrap();
        assert_eq!(t.faces.len(), 2);
    }

    #[test]
    fn vertex_jump_that_inverts_a_ring_is_rejected() {
        // found by the randomised test: the result is a simple polygon, so
        // only an orientation check can tell that the face turned inside out
        let mut t = Topology::default();
        t.add_ring_face(&square(0.0, 0.0, 10.0), FaceKind::Land).unwrap();
        let (v, _) = t.nearest_vertex(LonLat::new(10.0, 10.0), 0.1).unwrap();
        assert!(t.clone().move_vertex(v, LonLat::new(-5.0, -5.0)).is_err());
        assert!(t.clone().move_vertex(v, LonLat::new(9.0, 11.0)).is_ok());
    }

    #[test]
    fn lake_and_enclosing_island() {
        let mut t = Topology::default();
        let land = t.add_ring_face(&square(0.0, 0.0, 20.0), FaceKind::Land).unwrap();
        let lake = t.add_ring_face(&square(5.0, 5.0, 5.0), FaceKind::Water).unwrap();
        t.validate().unwrap();
        assert_eq!(t.face_at(LonLat::new(7.0, 7.0)), lake);
        assert_eq!(t.face_at(LonLat::new(15.0, 15.0)), land);
        // island inside lake
        let isle = t.add_ring_face(&square(6.0, 6.0, 1.0), FaceKind::Land).unwrap();
        t.validate().unwrap();
        assert_eq!(t.face_at(LonLat::new(6.5, 6.5)), isle);
        // a big ring around everything adopts the existing land as a hole
        let mut t3 = Topology::default();
        let small = t3.add_ring_face(&square(2.0, 2.0, 1.0), FaceKind::Land).unwrap();
        let sea = t3.add_ring_face(&square(0.0, 0.0, 10.0), FaceKind::Water).unwrap();
        t3.validate().unwrap();
        assert_eq!(t3.faces[&sea].holes.len(), 1);
        assert_eq!(t3.face_at(LonLat::new(2.5, 2.5)), small);
        // overlapping ring rejected
        assert!(t3.add_ring_face(&square(9.0, 9.0, 3.0), FaceKind::Land).is_err());
    }

    #[test]
    fn antimeridian_island_split() {
        let mut t = Topology::default();
        let ring = vec![LonLat::new(170.0, -10.0), LonLat::new(-170.0, -10.0), LonLat::new(-170.0, 10.0), LonLat::new(170.0, 10.0)];
        let f = t.add_ring_face(&ring, FaceKind::Land).unwrap();
        t.validate().unwrap();
        assert_eq!(t.face_at(LonLat::new(-179.0, 0.0)), f);
        assert_eq!(t.face_at(LonLat::new(179.0, 0.0)), f);
        // split exactly along the antimeridian
        let (a, b) = t.split_face(&[LonLat::new(180.0, -10.0), LonLat::new(180.0, 10.0)], 6371.0).unwrap();
        t.validate().unwrap();
        let fa = t.face_at(LonLat::new(175.0, 0.0));
        let fb = t.face_at(LonLat::new(-175.0, 0.0));
        assert!(fa != fb && [a, b].contains(&fa) && [a, b].contains(&fb));
    }

    #[test]
    fn split_errors_leave_validation_possible() {
        let mut t = Topology::default();
        t.add_ring_face(&square(0.0, 0.0, 10.0), FaceKind::Land).unwrap();
        // endpoint not on boundary
        assert!(t.clone().split_face(&[LonLat::new(5.0, 1.0), LonLat::new(5.0, 10.0)], 6371.0).is_err());
        // cut leaving the face
        assert!(t
            .clone()
            .split_face(&[LonLat::new(5.0, 0.0), LonLat::new(15.0, 5.0), LonLat::new(5.0, 10.0)], 6371.0)
            .is_err());
    }
}
