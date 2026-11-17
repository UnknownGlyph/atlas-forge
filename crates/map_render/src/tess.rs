//! CPU geometry preparation: triangulation, subdivision and seam cutting.
//!
//! Output geometry is in *relative* lon/lat degrees (lon - lon0), lying
//! entirely inside the strip lon ∈ [-180, 180]. It is projection-independent:
//! the vertex shader applies any 2D projection or the globe transform.

use bytemuck::{Pod, Zeroable};
use map_core::doc::Document;
use map_core::geom::{self, P2};
use map_core::topo::{FaceId, FaceKind, Topology, BACKGROUND};
use map_core::topo::{Edge, EdgeId};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

/// Longest allowed triangle/segment edge (degrees) so straight GPU edges
/// follow the curvature of projections and of the sphere.
pub const MAX_EDGE_DEG: f64 = 2.0;

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct FillVertex {
    pub ll: [f32; 2],
    pub slot: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, PartialEq)]
pub struct LineInstance {
    pub p0: [f32; 2],
    pub p1: [f32; 2],
    pub class: u32,
    pub _pad: u32,
}

pub mod class {
    pub const COAST: u32 = 0;
    pub const BORDER: u32 = 1;
    pub const INTERNAL: u32 = 2;
    pub const RIVER: u32 = 3;
    pub const SELECT: u32 = 4;
    pub const HOVER: u32 = 5;
    pub const GRATICULE: u32 = 6;
    /// Boundaries of a second partition layer drawn over the active one.
    pub const LAYER_OVERLAY: u32 = 7;
    /// Land that changed hands since the first year of a local view.
    pub const FOCUS: u32 = 8;
    /// Bands in the sea along the coasts (a map theme's look), outermost first.
    pub const GLOW: [u32; 3] = [9, 10, 11];
}

/// Triangulate one face into a triangle soup of relative lon/lat positions.
pub fn tessellate_face(topo: &Topology, f: FaceId, lon0: f64) -> Vec<[f32; 2]> {
    let (outer, holes) = topo.face_planar(f);
    if outer.len() < 3 {
        return vec![];
    }
    let mut data: Vec<f64> = Vec::with_capacity((outer.len() + holes.iter().map(|h| h.len()).sum::<usize>()) * 2);
    let mut hole_idx = Vec::new();
    for p in &outer {
        data.extend([p[0] - lon0, p[1]]);
    }
    for h in &holes {
        hole_idx.push(data.len() / 2);
        for p in h {
            data.extend([p[0] - lon0, p[1]]);
        }
    }
    let Ok(idx) = earcutr::earcut(&data, &hole_idx, 2) else {
        return vec![];
    };
    let pt = |i: usize| [data[2 * i], data[2 * i + 1]];
    let mut out = Vec::with_capacity(idx.len() * 2);
    for t in idx.chunks_exact(3) {
        subdivide([pt(t[0]), pt(t[1]), pt(t[2])], &mut |tri| clip_to_strip(tri, &mut out));
    }
    out
}

fn dist2(a: P2, b: P2) -> f64 {
    (a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)
}

/// Longest-edge bisection until every edge is <= MAX_EDGE_DEG.
fn subdivide(t: [P2; 3], emit: &mut dyn FnMut([P2; 3])) {
    let m2 = MAX_EDGE_DEG * MAX_EDGE_DEG;
    let e = [dist2(t[0], t[1]), dist2(t[1], t[2]), dist2(t[2], t[0])];
    let (i, &l) = e.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap();
    if l <= m2 {
        emit(t);
        return;
    }
    let (a, b, c) = (t[i], t[(i + 1) % 3], t[(i + 2) % 3]);
    let m = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
    subdivide([a, m, c], emit);
    subdivide([m, b, c], emit);
}

/// Shift a triangle by multiples of 360° and clip each copy to [-180, 180].
fn clip_to_strip(t: [P2; 3], out: &mut Vec<[f32; 2]>) {
    let xmin = t.iter().map(|p| p[0]).fold(f64::INFINITY, f64::min);
    let xmax = t.iter().map(|p| p[0]).fold(f64::NEG_INFINITY, f64::max);
    for k in [-720.0, -360.0, 0.0, 360.0, 720.0] {
        if xmax + k <= -180.0 || xmin + k >= 180.0 {
            continue;
        }
        let poly: Vec<P2> = t.iter().map(|p| [p[0] + k, p[1]]).collect();
        let emit_poly = |poly: &[P2], out: &mut Vec<[f32; 2]>| {
            for i in 1..poly.len().saturating_sub(1) {
                for p in [poly[0], poly[i], poly[i + 1]] {
                    out.push([p[0] as f32, p[1] as f32]);
                }
            }
        };
        if xmin + k >= -180.0 && xmax + k <= 180.0 {
            emit_poly(&poly, out);
            continue;
        }
        let p = clip_half(&poly, -180.0, true);
        let p = clip_half(&p, 180.0, false);
        if p.len() >= 3 {
            emit_poly(&p, out);
        }
    }
}

pub use map_core::geom::clip_half;

/// Densify a polyline (unwrapped, relative) and split it at the seam.
pub fn push_polyline(pts: &[P2], cls: u32, out: &mut Vec<LineInstance>) {
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let n = ((dist2(a, b).sqrt() / MAX_EDGE_DEG).ceil() as usize).max(1);
        for i in 0..n {
            let t0 = i as f64 / n as f64;
            let t1 = (i + 1) as f64 / n as f64;
            let p = [a[0] + (b[0] - a[0]) * t0, a[1] + (b[1] - a[1]) * t0];
            let q = [a[0] + (b[0] - a[0]) * t1, a[1] + (b[1] - a[1]) * t1];
            push_segment(p, q, cls, out);
        }
    }
}

fn push_segment(p: P2, q: P2, cls: u32, out: &mut Vec<LineInstance>) {
    // bring p into [-180, 180)
    let k = ((p[0] + 180.0).div_euclid(360.0)) * 360.0;
    let (p, q) = ([p[0] - k, p[1]], [q[0] - k, q[1]]);
    let mk = |a: P2, b: P2| LineInstance { p0: [a[0] as f32, a[1] as f32], p1: [b[0] as f32, b[1] as f32], class: cls, _pad: 0 };
    if q[0] > 180.0 || q[0] < -180.0 {
        let edge = if q[0] > 180.0 { 180.0 } else { -180.0 };
        let t = (edge - p[0]) / (q[0] - p[0]);
        let m = [edge, p[1] + (q[1] - p[1]) * t];
        out.push(mk(p, m));
        out.push(mk([-edge, m[1]], [q[0] - 2.0 * edge, q[1]]));
    } else {
        out.push(mk(p, q));
    }
}

/// Fixed lon/lat grid covering the world, used for the ocean / globe base.
pub fn ocean_mesh(step: f64) -> Vec<FillVertex> {
    let mut v = Vec::new();
    let nx = (360.0 / step) as i32;
    let ny = (180.0 / step) as i32;
    for i in 0..nx {
        for j in 0..ny {
            let (x0, y0) = (-180.0 + i as f64 * step, -90.0 + j as f64 * step);
            let (x1, y1) = (x0 + step, y0 + step);
            for p in [[x0, y0], [x1, y0], [x1, y1], [x0, y0], [x1, y1], [x0, y1]] {
                v.push(FillVertex { ll: [p[0] as f32, p[1] as f32], slot: 0 });
            }
        }
    }
    v
}

pub fn graticule(step: f64) -> Vec<LineInstance> {
    let mut out = Vec::new();
    let mut lon = -180.0;
    while lon < 180.0 {
        push_polyline(&[[lon, -90.0], [lon, 90.0]], class::GRATICULE, &mut out);
        lon += step;
    }
    let mut lat = -90.0 + step;
    while lat < 90.0 {
        push_polyline(&[[-180.0, lat], [180.0, lat]], class::GRATICULE, &mut out);
        lat += step;
    }
    out
}

fn edge_class(doc: &Document, left: FaceId, right: FaceId) -> Option<u32> {
    let kind = |f: FaceId| doc.topo.faces[&f].kind;
    match (kind(left), kind(right)) {
        (FaceKind::Land, FaceKind::Land) => {
            let (a, b) = (doc.owner.get(&left), doc.owner.get(&right));
            let ta = a.map(|e| doc.top_ancestor(*e));
            let tb = b.map(|e| doc.top_ancestor(*e));
            Some(if ta != tb { class::BORDER } else { class::INTERNAL })
        }
        (FaceKind::Water, FaceKind::Water) => None,
        _ => Some(class::COAST),
    }
}

/// Statistics of the last scene update (for the status bar and benchmarks).
#[derive(Clone, Debug, Default)]
pub struct UpdateStats {
    pub faces_retessellated: usize,
    pub fill_chunks_rebuilt: usize,
    pub line_chunks_rebuilt: usize,
    pub fill_rebuilt: bool,
    pub lines_rebuilt: bool,
    pub ms: f64,
}

/// Size of a spatial render chunk in degrees (relative to the central meridian).
pub const CHUNK_DEG: f64 = 30.0;
pub type ChunkId = u32;
const RIVER_CHUNK: ChunkId = u32::MAX;

fn chunk_of(bb: &[f64; 4], lon0: f64) -> ChunkId {
    let nx = (360.0 / CHUNK_DEG) as u32;
    let ny = (180.0 / CHUNK_DEG) as u32;
    let cx = geom::wrap_lon((bb[0] + bb[2]) * 0.5 - lon0);
    let cy = (bb[1] + bb[3]) * 0.5;
    let x = (((cx + 180.0) / CHUNK_DEG).floor().max(0.0) as u32).min(nx - 1);
    let y = (((cy + 90.0) / CHUNK_DEG).floor().max(0.0) as u32).min(ny - 1);
    y * nx + x
}

/// One independently uploaded piece of render data.
#[derive(Clone)]
pub struct Chunk<T> {
    pub id: ChunkId,
    /// Unique per rebuild: the GPU re-uploads only when it changes.
    pub gen: u64,
    pub data: Arc<Vec<T>>,
}

/// Incrementally maintained render data derived from a [`Document`].
///
/// Faces and edges are grouped into spatial chunks; an edit re-triangulates
/// only changed faces and rebuilds/re-uploads only the chunks that contain
/// changed faces or edges. Colour slots are stable per face, so adding or
/// removing a face does not disturb other chunks.
#[derive(Default)]
pub struct SceneCache {
    lon0: f64,
    valid: bool,
    gen: u64,
    meshes: HashMap<FaceId, (u64, ChunkId, Arc<Vec<[f32; 2]>>)>,
    slots: HashMap<FaceId, u32>,
    free_slots: Vec<u32>,
    next_slot: u32,
    fill_members: BTreeMap<ChunkId, BTreeSet<FaceId>>,
    fill: BTreeMap<ChunkId, Chunk<FillVertex>>,
    edge_chunk: HashMap<EdgeId, ChunkId>,
    line_members: BTreeMap<ChunkId, BTreeSet<EdgeId>>,
    lines: BTreeMap<ChunkId, Chunk<LineInstance>>,
    last: Option<Document>,
    /// Changes whenever any line chunk changes.
    pub line_gen: u64,
    pub colors: Arc<Vec<[f32; 4]>>,
    pub color_gen: u64,
}

pub fn rgba(c: [u8; 4]) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, c[3] as f32 / 255.0]
}

fn edge_instances(doc: &Document, e: &Edge, cls: u32, lon0: f64, out: &mut Vec<LineInstance>) {
    let full = doc.topo.edge_full(e);
    let mut un = geom::unwrap_path(&full, None);
    for p in &mut un {
        p[0] -= lon0;
    }
    push_polyline(&un, cls, out);
}

fn river_instances(doc: &Document, lon0: f64) -> Vec<LineInstance> {
    let mut out = Vec::new();
    // routes are drawn styled, with the overlay; so are rivers that are
    // in a layer, in the turn of their layer
    for (_, r) in doc.rivers.iter().filter(|(id, r)| r.route.is_none() && !r.hidden && doc.group_of_id(**id).is_none()) {
        let mut un = geom::unwrap_path(&r.pts, None);
        for p in &mut un {
            p[0] -= lon0;
        }
        push_polyline(&un, class::RIVER, &mut out);
    }
    out
}

/// Edges bounding any of `faces` in `doc`.
fn edges_of_faces(doc: &Document, faces: &BTreeSet<FaceId>, out: &mut BTreeSet<EdgeId>) {
    for f in faces {
        if let Some(face) = doc.topo.faces.get(f) {
            for r in std::iter::once(&face.outer).chain(face.holes.iter()).flatten() {
                out.insert(r.edge);
            }
        }
    }
}

impl SceneCache {
    pub fn lon0(&self) -> f64 {
        self.lon0
    }

    fn next_gen(&mut self) -> u64 {
        self.gen += 1;
        self.gen
    }

    fn slot(&mut self, f: FaceId) -> u32 {
        if let Some(s) = self.slots.get(&f) {
            return *s;
        }
        let s = self.free_slots.pop().unwrap_or_else(|| {
            self.next_slot += 1;
            self.next_slot
        });
        self.slots.insert(f, s);
        s
    }

    /// Bring render data up to date with `doc`.
    pub fn update_geometry(&mut self, doc: &Document, lon0: f64) -> UpdateStats {
        let t0 = Instant::now();
        let mut st = UpdateStats::default();
        if !self.valid || lon0 != self.lon0 {
            // the seam moved: every mesh is cut differently
            *self = SceneCache { gen: self.gen, line_gen: self.line_gen + 1, color_gen: self.color_gen, ..Default::default() };
            self.lon0 = lon0;
            self.valid = true;
        }
        let prev = self.last.take();

        // ---- fills
        let faces_same = prev.as_ref().is_some_and(|p| p.topo.faces.ptr_eq(&doc.topo.faces));
        let mut dirty: BTreeSet<ChunkId> = BTreeSet::new();
        if !faces_same {
            let removed: Vec<FaceId> = self.meshes.keys().filter(|f| !doc.topo.faces.contains_key(*f)).copied().collect();
            for f in removed {
                let (_, c, _) = self.meshes.remove(&f).unwrap();
                if let Some(m) = self.fill_members.get_mut(&c) {
                    m.remove(&f);
                }
                dirty.insert(c);
                if let Some(s) = self.slots.remove(&f) {
                    self.free_slots.push(s);
                }
            }
            for (fid, face) in doc.topo.faces.iter() {
                if *fid == BACKGROUND || self.meshes.get(fid).map(|m| m.0) == Some(face.rev) {
                    continue;
                }
                let c = chunk_of(&face.bbox, lon0);
                if let Some((_, old, _)) = self.meshes.get(fid) {
                    if *old != c {
                        if let Some(m) = self.fill_members.get_mut(old) {
                            m.remove(fid);
                        }
                        dirty.insert(*old);
                    }
                }
                let mesh = Arc::new(tessellate_face(&doc.topo, *fid, lon0));
                self.meshes.insert(*fid, (face.rev, c, mesh));
                self.fill_members.entry(c).or_default().insert(*fid);
                self.slot(*fid);
                dirty.insert(c);
                st.faces_retessellated += 1;
            }
        }
        for c in &dirty {
            let members = self.fill_members.get(c).cloned().unwrap_or_default();
            if members.is_empty() {
                self.fill_members.remove(c);
                self.fill.remove(c);
                continue;
            }
            let mut v = Vec::with_capacity(members.iter().map(|f| self.meshes[f].2.len()).sum());
            for f in &members {
                let slot = self.slots[f];
                v.extend(self.meshes[f].2.iter().map(|p| FillVertex { ll: *p, slot }));
            }
            let gen = self.next_gen();
            self.fill.insert(*c, Chunk { id: *c, gen, data: Arc::new(v) });
        }
        st.fill_chunks_rebuilt = dirty.len();
        st.fill_rebuilt = !dirty.is_empty();

        // ---- lines: reclassify only edges touched by the change set
        let mut full = prev.is_none();
        let mut dirty_edges: BTreeSet<EdgeId> = BTreeSet::new();
        let mut rivers_dirty = full;
        if let Some(p) = &prev {
            let same = p.topo.edges.ptr_eq(&doc.topo.edges)
                && p.topo.faces.ptr_eq(&doc.topo.faces)
                && p.owner.ptr_eq(&doc.owner)
                && p.entities.ptr_eq(&doc.entities)
                && p.rivers.ptr_eq(&doc.rivers)
                && p.group_of == doc.group_of
                && p.groups.len() == doc.groups.len()
                && p.topo.nodes.ptr_eq(&doc.topo.nodes);
            if !same {
                let cs = map_core::change::ChangeSet::between(p, doc);
                // a changed parent can turn borders into internal lines anywhere
                full = cs.entities.iter().any(|e| p.entities.get(e).map(|x| x.parent) != doc.entities.get(e).map(|x| x.parent));
                // a river moved into or out of a layer changes hands between this mesh and the overlay
                rivers_dirty = cs.rivers || p.group_of != doc.group_of || p.groups.len() != doc.groups.len();
                dirty_edges.extend(cs.edges.iter().copied());
                let faces: BTreeSet<FaceId> = cs.faces.union(&cs.owners).copied().collect();
                edges_of_faces(p, &faces, &mut dirty_edges);
                edges_of_faces(doc, &faces, &mut dirty_edges);
            }
        }
        let mut dirty_lines: BTreeSet<ChunkId> = BTreeSet::new();
        if full {
            for c in self.lines.keys() {
                dirty_lines.insert(*c);
            }
            self.edge_chunk.clear();
            self.line_members.clear();
            dirty_edges = doc.topo.edges.keys().copied().collect();
            rivers_dirty = true;
        }
        for eid in &dirty_edges {
            if let Some(c) = self.edge_chunk.remove(eid) {
                if let Some(m) = self.line_members.get_mut(&c) {
                    m.remove(eid);
                }
                dirty_lines.insert(c);
            }
            if let Some(e) = doc.topo.edges.get(eid) {
                if edge_class(doc, e.left, e.right).is_some() && !doc.topo.on_pole(e) {
                    let c = chunk_of(&e.bbox, lon0);
                    self.edge_chunk.insert(*eid, c);
                    self.line_members.entry(c).or_default().insert(*eid);
                    dirty_lines.insert(c);
                }
            }
        }
        for c in &dirty_lines {
            let members = self.line_members.get(c).cloned().unwrap_or_default();
            if members.is_empty() {
                self.line_members.remove(c);
                self.lines.remove(c);
                continue;
            }
            let mut v = Vec::new();
            for eid in &members {
                let e = &doc.topo.edges[eid];
                if let Some(cls) = edge_class(doc, e.left, e.right) {
                    edge_instances(doc, e, cls, lon0, &mut v);
                }
            }
            let gen = self.next_gen();
            self.lines.insert(*c, Chunk { id: *c, gen, data: Arc::new(v) });
        }
        if rivers_dirty {
            let v = river_instances(doc, lon0);
            if v.is_empty() {
                self.lines.remove(&RIVER_CHUNK);
            } else {
                let gen = self.next_gen();
                self.lines.insert(RIVER_CHUNK, Chunk { id: RIVER_CHUNK, gen, data: Arc::new(v) });
            }
            dirty_lines.insert(RIVER_CHUNK);
        }
        st.line_chunks_rebuilt = dirty_lines.len();
        st.lines_rebuilt = !dirty_lines.is_empty();
        if st.lines_rebuilt {
            self.line_gen += 1;
        }
        self.last = Some(doc.clone());
        st.ms = t0.elapsed().as_secs_f64() * 1e3;
        st
    }

    /// Per-slot colours (slot 0 = the background face, drawn as one mesh
    /// over the whole sphere). One entry per face.
    pub fn update_colors(&mut self, ocean: [u8; 4], face_colors: &HashMap<FaceId, [u8; 4]>, highlight: &HashSet<FaceId>) {
        let mut c = vec![rgba(ocean); self.next_slot as usize + 1];
        for (f, slot) in self.slots.iter().chain(std::iter::once((&BACKGROUND, &0))) {
            let fallback = if *f == BACKGROUND { ocean } else { [200, 200, 200, 255] };
            let mut col = rgba(face_colors.get(f).copied().unwrap_or(fallback));
            if highlight.contains(f) {
                for k in 0..3 {
                    col[k] = col[k] * 0.75 + 0.25;
                }
            }
            c[*slot as usize] = col;
        }
        self.colors = Arc::new(c);
        self.color_gen += 1;
    }

    pub fn fill_chunks(&self) -> Vec<Chunk<FillVertex>> {
        self.fill.values().cloned().collect()
    }
    pub fn line_chunks(&self) -> Vec<Chunk<LineInstance>> {
        self.lines.values().cloned().collect()
    }
    pub fn face_of_slot(&self, slot: u32) -> Option<FaceId> {
        self.slots.iter().find(|(_, s)| **s == slot).map(|(f, _)| *f)
    }
    pub fn vertex_count(&self) -> usize {
        self.fill.values().map(|c| c.data.len()).sum()
    }
}

/// The coast bands: every coast segment of `lines` three times, the
/// widest band first (drawn over the sea, under the land).
pub fn glow_lines(lines: &[Chunk<LineInstance>]) -> Vec<LineInstance> {
    let coast: Vec<&LineInstance> = lines.iter().flat_map(|c| c.data.iter()).filter(|l| l.class == class::COAST).collect();
    class::GLOW.iter().flat_map(|cls| coast.iter().map(move |l| LineInstance { class: *cls, ..**l })).collect()
}

/// All line instances of a document in one go (reference implementation;
/// the scene cache builds the same set chunk by chunk).
pub fn build_lines(doc: &Document, lon0: f64) -> Vec<LineInstance> {
    let mut out = Vec::new();
    for (_, e) in doc.topo.edges.iter() {
        let Some(cls) = edge_class(doc, e.left, e.right) else { continue };
        if doc.topo.on_pole(e) {
            continue;
        }
        edge_instances(doc, e, cls, lon0, &mut out);
    }
    out.extend(river_instances(doc, lon0));
    out
}

/// Line instances for the given edges (e.g. another layer's boundaries).
pub fn edge_lines(doc: &Document, edges: &[map_core::topo::EdgeId], lon0: f64, cls: u32) -> Vec<LineInstance> {
    let mut out = Vec::new();
    for id in edges {
        if let Some(e) = doc.topo.edges.get(id) {
            edge_instances(doc, e, cls, lon0, &mut out);
        }
    }
    out
}

/// Outline polylines (relative lon) for a set of faces, e.g. the selection.
pub fn outline_lines(doc: &Document, faces: &HashSet<FaceId>, lon0: f64, cls: u32) -> Vec<LineInstance> {
    let mut out = Vec::new();
    for poly in doc.dissolve_faces(faces) {
        for ring in std::iter::once(&poly.outer).chain(poly.holes.iter()) {
            let mut r: Vec<P2> = ring.iter().map(|p| [p[0] - lon0, p[1]]).collect();
            let n = r.len();
            // a ring around a pole is closed through the pole for filling
            // (up the meridian it started on, along the pole, and down
            // again 360° away): that is not part of its outline
            if n >= 5 && r[n - 1][1].abs() == 90.0 && r[n - 2][1] == r[n - 1][1] && ((r[n - 1][0] - r[n - 2][0]).abs() - 360.0).abs() < 1e-9 {
                r.truncate(n - 2);
            } else if let Some(f) = r.first().copied() {
                r.push(f);
            }
            // stretches along a pole (land cut along meridians) are a point on the globe
            let mut run: Vec<P2> = Vec::new();
            for w in r.windows(2) {
                if w[0][1].abs() == 90.0 && w[1][1] == w[0][1] {
                    if run.len() > 1 {
                        push_polyline(&run, cls, &mut out);
                    }
                    run.clear();
                    continue;
                }
                if run.is_empty() {
                    run.push(w[0]);
                }
                run.push(w[1]);
            }
            if run.len() > 1 {
                push_polyline(&run, cls, &mut out);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_core::LonLat;

    /// Land around a pole: the outline of a cap is its coast (not the way
    /// it is closed through the pole for filling), and edges that lie on
    /// the pole, where the land is cut along meridians, are not drawn.
    #[test]
    fn polar_land_has_no_lines_to_the_pole() {
        use map_core::overlay::{self, DrawIntent};
        let ring: Vec<LonLat> = (0..36).map(|i| LonLat::new(-179.0 + 10.0 * i as f64, 70.0 + if i % 2 == 0 { 0.0 } else { 0.7 })).collect();
        let (d, _) = overlay::apply(&Document::default(), &ring, DrawIntent::AddLand).unwrap();
        let cap: HashSet<FaceId> = d.topo.faces.keys().copied().filter(|f| *f != map_core::BACKGROUND).collect();
        assert_eq!(cap.len(), 1);
        let near_pole = |l: &LineInstance| l.p0[1] > 71.0 || l.p1[1] > 71.0;
        let outline = outline_lines(&d, &cap, 0.0, class::SELECT);
        assert!(outline.len() >= 36 && !outline.iter().any(near_pole), "{} lines", outline.len());
        assert!(!build_lines(&d, 0.0).iter().any(near_pole));
        // a lake on the antimeridian cuts the land in two along the prime
        // meridian: those cuts are lines, the stretches along the pole are not
        let lake = [LonLat::new(170.0, 76.0), LonLat::new(-170.0, 76.0), LonLat::new(-170.0, 82.0), LonLat::new(170.0, 82.0)];
        let (d, _) = overlay::apply(&d, &lake, DrawIntent::CarveWater).unwrap();
        assert!(d.topo.edges.values().any(|e| d.topo.on_pole(e)));
        let on_pole = |l: &LineInstance| l.p0[1] > 89.99 && l.p1[1] > 89.99;
        let land: HashSet<FaceId> = d.topo.faces.iter().filter(|(f, x)| **f != map_core::BACKGROUND && x.kind == FaceKind::Land).map(|(f, _)| *f).collect();
        assert_eq!(land.len(), 2);
        for lines in [build_lines(&d, 0.0), outline_lines(&d, &land, 0.0, class::SELECT)] {
            assert!(!lines.is_empty() && !lines.iter().any(on_pole), "{:?}", lines.iter().filter(|l| on_pole(l)).map(|l| (l.p0, l.p1, l.class)).collect::<Vec<_>>());
        }
        let one: HashSet<FaceId> = land.iter().copied().take(1).collect();
        let half = outline_lines(&d, &one, 0.0, class::SELECT);
        assert!(half.iter().any(|l| l.p0[1] > 85.0) && !half.iter().any(on_pole), "the cut to the pole is part of its outline");
    }

    #[test]
    fn land_background_borders_and_colour() {
        let mut d = Document::default();
        let (x, y) = (d.add_entity(map_core::EntityKind::Country, "X"), d.add_entity(map_core::EntityKind::Country, "Y"));
        let f = d.add_ring_face(&[LonLat::new(0.0, 0.0), LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0), LonLat::new(0.0, 10.0)], FaceKind::Land).unwrap();
        d.assign(f, Some(y)).unwrap();
        let e = *d.topo.edges.keys().next().unwrap();
        let (l, r) = (d.topo.edges[&e].left, d.topo.edges[&e].right);
        assert_eq!(edge_class(&d, l, r), Some(class::COAST), "island in a sea background");
        d.set_background_kind(FaceKind::Land);
        assert_eq!(edge_class(&d, l, r), Some(class::BORDER), "unclaimed land background next to Y");
        d.assign(map_core::BACKGROUND, Some(y)).unwrap();
        assert_eq!(edge_class(&d, l, r), Some(class::INTERNAL), "Y's own background and island");
        d.assign(map_core::BACKGROUND, Some(x)).unwrap();
        assert_eq!(edge_class(&d, l, r), Some(class::BORDER));
        // slot 0 (the background mesh) takes the background's theme colour
        let theme = map_core::theme::evaluate(&d);
        let mut sc = SceneCache::default();
        sc.update_geometry(&d, 0.0);
        sc.update_colors(d.style.ocean, &theme.face_colors, &HashSet::new());
        assert_eq!(sc.colors[0], rgba(theme.entity_colors[&x]));
    }

    fn tri_area(soup: &[[f32; 2]]) -> f64 {
        soup.chunks_exact(3)
            .map(|t| {
                let (a, b, c) = (t[0], t[1], t[2]);
                (((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])) as f64 * 0.5).abs()
            })
            .sum()
    }

    #[test]
    fn seam_crossing_island_is_cut_and_area_preserved() {
        let mut d = Document::default();
        let ring = [LonLat::new(170.0, -10.0), LonLat::new(-170.0, -10.0), LonLat::new(-170.0, 10.0), LonLat::new(170.0, 10.0)];
        let f = d.add_ring_face(&ring, FaceKind::Land).unwrap();
        let soup = tessellate_face(&d.topo, f, 0.0);
        assert!(soup.iter().all(|p| p[0] >= -180.0 && p[0] <= 180.0));
        assert!((tri_area(&soup) - 400.0).abs() < 1e-3, "area {}", tri_area(&soup));
        // all edges subdivided
        for t in soup.chunks_exact(3) {
            for i in 0..3 {
                let (a, b) = (t[i], t[(i + 1) % 3]);
                let l = ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt() as f64;
                assert!(l <= MAX_EDGE_DEG * 1.0001);
            }
        }
        // rotated central meridian: the island is now in the middle, no cut needed
        let soup2 = tessellate_face(&d.topo, f, 180.0);
        assert!((tri_area(&soup2) - 400.0).abs() < 1e-3);
        assert!(soup2.iter().all(|p| p[0].abs() <= 10.0001));
    }

    #[test]
    fn incremental_update_only_touches_changed_faces() {
        let mut d = Document::default();
        let sq = |x: f64| [LonLat::new(x, 0.0), LonLat::new(x + 5.0, 0.0), LonLat::new(x + 5.0, 5.0), LonLat::new(x, 5.0)];
        d.add_ring_face(&sq(0.0), FaceKind::Land).unwrap();
        d.add_ring_face(&sq(20.0), FaceKind::Land).unwrap();
        let mut c = SceneCache::default();
        let s = c.update_geometry(&d, 0.0);
        assert_eq!(s.faces_retessellated, 2);
        let s = c.update_geometry(&d, 0.0);
        assert_eq!(s.faces_retessellated, 0);
        assert!(!s.fill_rebuilt && !s.lines_rebuilt);
        let (v, _) = d.topo.nearest_vertex(LonLat::new(20.0, 0.0), 0.1).unwrap();
        d.topo.move_vertex(v, LonLat::new(19.0, -1.0)).unwrap();
        let s = c.update_geometry(&d, 0.0);
        assert_eq!(s.faces_retessellated, 1);
        assert!(s.lines_rebuilt);
    }

    #[test]
    fn seam_segment_split() {
        let mut out = vec![];
        push_polyline(&[[175.0, 0.0], [185.0, 0.0]], 0, &mut out);
        assert!(out.iter().all(|s| s.p0[0].abs() <= 180.0 && s.p1[0].abs() <= 180.0));
        assert_eq!(out.len(), 6); // 5 densified pieces, one split at the seam
    }

    fn fill_signature(c: &SceneCache) -> Vec<(u64, u32, u32)> {
        let mut by_slot: HashMap<u32, FaceId> = HashMap::new();
        let mut v = Vec::new();
        for ch in c.fill_chunks() {
            for x in ch.data.iter() {
                let f = *by_slot.entry(x.slot).or_insert_with(|| c.face_of_slot(x.slot).expect("slot maps to a face"));
                v.push((f.0, x.ll[0].to_bits(), x.ll[1].to_bits()));
            }
        }
        v.sort_unstable();
        v
    }

    fn line_signature(c: &SceneCache) -> Vec<(u32, u32, u32, u32, u32)> {
        let mut v: Vec<_> = c.line_chunks().iter().flat_map(|ch| ch.data.iter().map(|l| (l.p0[0].to_bits(), l.p0[1].to_bits(), l.p1[0].to_bits(), l.p1[1].to_bits(), l.class)).collect::<Vec<_>>()).collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn chunked_updates_equal_a_full_rebuild() {
        use map_core::history::History;
        use map_core::sample::Rng;
        let mut d = map_core::sample::grid_world(10, 6, 4, 11);
        let mut h = History::default();
        let mut rng = Rng::new(3);
        let mut c = SceneCache::default();
        let mut lon0 = 0.0;
        c.update_geometry(&d, lon0);
        for step in 0..120 {
            let r = rng.next_f64();
            let edges: Vec<_> = d.topo.edges.iter().filter(|(_, e)| !e.pts.is_empty()).map(|(id, e)| (*id, e.pts.len())).collect();
            let countries = d.countries();
            let faces: Vec<FaceId> = d.topo.faces.keys().copied().filter(|f| *f != BACKGROUND).collect();
            let _ = if r < 0.35 {
                let (e, n) = edges[(rng.next_f64() * edges.len() as f64) as usize];
                let v = map_core::VertexRef::Pt { edge: e, idx: (rng.next_f64() * n as f64) as usize };
                let p = d.topo.position_of(v);
                h.apply(&mut d, "move", |d| d.topo.move_vertex(v, LonLat::new(p.lon + rng.range(-0.5, 0.5), p.lat + rng.range(-0.5, 0.5))))
            } else if r < 0.55 {
                let f = faces[(rng.next_f64() * faces.len() as f64) as usize];
                let e = countries[(rng.next_f64() * countries.len() as f64) as usize];
                h.apply(&mut d, "assign", |d| d.assign(f, Some(e)))
            } else if r < 0.65 {
                let a = countries[(rng.next_f64() * countries.len() as f64) as usize];
                let b = countries[(rng.next_f64() * countries.len() as f64) as usize];
                h.apply(&mut d, "parent", |d| {
                    if a != b {
                        let _ = d.record_parent(a, Some(b));
                    }
                    Ok(())
                })
            } else if r < 0.75 {
                let pair = d.topo.edges.values().find(|e| e.left != BACKGROUND && e.right != BACKGROUND).map(|e| (e.left, e.right));
                match pair {
                    Some((a, b)) => h.apply(&mut d, "merge", |d| d.merge_faces(a, b).map(|_| ())),
                    None => Ok(()),
                }
            } else if r < 0.82 {
                let x = rng.range(-170.0, 170.0);
                h.apply(&mut d, "river", |d| {
                    let id = map_core::doc::LineId(d.topo.alloc());
                    d.rivers.insert(id, map_core::doc::LineFeature { name: "r".into(), pts: vec![LonLat::new(x, 0.0), LonLat::new(x + 5.0, 3.0)], route: None, legend: String::new(), show_name: false, hidden: false });
                    Ok(())
                })
            } else if r < 0.93 {
                h.undo(&mut d);
                Ok(())
            } else if r < 0.97 {
                h.redo(&mut d);
                Ok(())
            } else {
                lon0 = rng.range(-180.0, 180.0).round();
                Ok(())
            };
            c.update_geometry(&d, lon0);
            let mut fresh = SceneCache::default();
            fresh.update_geometry(&d, lon0);
            assert_eq!(fill_signature(&c), fill_signature(&fresh), "fill differs at step {step}");
            assert_eq!(line_signature(&c), line_signature(&fresh), "lines differ at step {step}");
        }
    }

    #[test]
    fn a_local_edit_rebuilds_few_chunks() {
        let mut d = map_core::sample::grid_world(20, 12, 4, 5);
        let mut c = SceneCache::default();
        let first = c.update_geometry(&d, 0.0);
        assert!(first.fill_chunks_rebuilt > 10);
        let (eid, e) = d.topo.edges.iter().find(|(_, e)| e.pts.len() > 2 && e.left != BACKGROUND && e.right != BACKGROUND).map(|(i, e)| (*i, e.clone())).unwrap();
        let p = e.pts[1];
        d.topo.move_vertex(map_core::VertexRef::Pt { edge: eid, idx: 1 }, LonLat::new(p.lon + 0.01, p.lat)).unwrap();
        let s = c.update_geometry(&d, 0.0);
        assert_eq!(s.faces_retessellated, 2);
        assert!(s.fill_chunks_rebuilt <= 2 && s.line_chunks_rebuilt <= 2, "{s:?}");
    }
}
