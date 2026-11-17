//! Drawing a polygon over existing land, with an explicit intent.
//!
//! The new shape of every affected face is computed with boolean operations
//! and rebuilt through the repair pipeline with a tiny snap distance, which
//! restores exact shared coordinates (the boolean kernel rounds its output).
//!
//! Normally only the land faces the polygon touches (region W) are rebuilt:
//! the edges on W's boundary are forced to keep their endpoints, the
//! rebuilt piece is spliced into the untouched topology, and the
//! neighbours' rings are re-assembled; then the touched part is validated.
//! Anything the splice cannot handle safely falls back to rebuilding the
//! whole world. Rebuilt faces get new ids (owners are carried over); the
//! id counter, shared with entities and fields, only moves forward.

use crate::doc::{Document, EntityId};
use crate::geom::{self, LonLat, P2};
use crate::repair::{self, RepairOptions};
use crate::seam;
use crate::topo::{ArcRef, Edge, EdgeId, FaceId, FaceKind, NodeId, Topology, BACKGROUND};
use geo::BooleanOps;
use std::collections::{BTreeSet, HashMap};
use std::time::Instant;

/// What a drawn polygon means where it overlaps existing land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawIntent {
    /// New land only where there is water; existing land is untouched.
    AddLand,
    /// Water and unclaimed land inside the polygon go to the country;
    /// other countries keep their land.
    Expand(EntityId),
    /// Everything inside the polygon goes to the country, including land
    /// taken from neighbours.
    Take(EntityId),
    /// Land inside the polygon becomes water.
    CarveWater,
}

#[derive(Clone, Debug, Default)]
pub struct OverlayReport {
    pub added_km2: f64,
    /// Area each previous owner lost (None = unclaimed land).
    pub taken: Vec<(Option<EntityId>, f64)>,
    pub carved_km2: f64,
    pub faces_before: usize,
    pub faces_after: usize,
    /// Faces that were rebuilt (the rest kept their records and ids).
    pub rebuilt_faces: usize,
    /// True when only the affected region was rebuilt and spliced in.
    pub local: bool,
    /// Why the whole world had to be rebuilt instead, if it was.
    pub fallback: Option<String>,
    pub ms: f64,
}

type Parts = Vec<Vec<Vec<P2>>>;

fn to_mp(parts: &Parts) -> geo::MultiPolygon<f64> {
    let ls = |r: &Vec<P2>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    geo::MultiPolygon::new(parts.iter().map(|p| geo::Polygon::new(ls(&p[0]), p[1..].iter().map(ls).collect())).collect())
}

/// A boolean result's longitude put back on the antimeridian when the
/// kernel's rounding left it a hair off (180.0000000007 would otherwise
/// wrap to the far side of the globe, -179.9999999999 would no longer meet
/// the exact seam of an uncut neighbour).
fn on_seam(x: f64) -> f64 {
    if (x.abs() - 180.0).abs() < 1e-7 {
        180.0f64.copysign(x)
    } else {
        x
    }
}

/// Likewise a latitude put back on the pole (land around a pole reaches it
/// exactly; -89.9999999 would leave a sliver of sea there).
fn at_pole(y: f64) -> f64 {
    if (y.abs() - 90.0).abs() < 1e-7 {
        90.0f64.copysign(y)
    } else {
        y
    }
}

fn from_mp(mp: &geo::MultiPolygon<f64>) -> Parts {
    mp.0.iter()
        .map(|p| {
            std::iter::once(p.exterior())
                .chain(p.interiors().iter())
                .map(|ls| {
                    let mut v: Vec<P2> = ls.coords().map(|c| [on_seam(c.x), at_pole(c.y)]).collect();
                    if v.len() > 1 && v.first() == v.last() {
                        v.pop();
                    }
                    v
                })
                .filter(|r| r.len() >= 3)
                .collect::<Vec<_>>()
        })
        .filter(|rs: &Vec<Vec<P2>>| !rs.is_empty())
        .collect()
}

fn area(parts: &Parts, r: f64) -> f64 {
    parts
        .iter()
        .map(|p| {
            let mut a = geom::spherical_area(&p[0], r).abs();
            for h in &p[1..] {
                a -= geom::spherical_area(h, r).abs();
            }
            a.max(0.0)
        })
        .sum()
}

fn bbox(parts: &Parts) -> [f64; 4] {
    let mut b = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for p in parts {
        let q = geom::bbox(&p[0]);
        b = [b[0].min(q[0]), b[1].min(q[1]), b[2].max(q[2]), b[3].max(q[3])];
    }
    b
}

/// The drawn polygon, cleaned and prepared in the strip.
struct Prepared {
    p_mp: geo::MultiPolygon<f64>,
    /// Strip-space bbox of the polygon parts.
    pb: [f64; 4],
    /// Unwrapped lon/lat bbox (for spatial index queries).
    query_bb: [f64; 4],
}

/// A drawn ring in planar form, and whether it goes around a pole:
/// normalised, repeated points dropped; a ring around a pole oriented to
/// hold the nearer pole (it has no inside by its shape alone; eastward
/// winding encloses the north pole) and closed along the pole.
fn ring_planar(ring: &[LonLat]) -> Result<(Vec<P2>, bool), String> {
    let mut pts: Vec<LonLat> = Vec::new();
    for p in ring {
        let p = p.normalized();
        if pts.last().is_none_or(|q| geom::geo_dist_deg(*q, p) > 1e-12) {
            pts.push(p);
        }
    }
    while pts.len() > 1 && geom::geo_dist_deg(pts[0], *pts.last().unwrap()) < 1e-12 {
        pts.pop();
    }
    if pts.len() < 3 {
        return Err("多边形至少需要 3 个点".into());
    }
    let w = geom::ring_winding(&pts);
    let polar = w.abs() > 180.0;
    if polar {
        let mean_lat = pts.iter().map(|p| p.lat).sum::<f64>() / pts.len() as f64;
        if (mean_lat >= 0.0) != (w > 0.0) {
            pts.reverse();
        }
        pts = geom::start_at_seam(&pts);
    }
    let planar = geom::ring_to_planar(&pts);
    if !geom::ring_is_simple(&planar) {
        return Err("多边形自相交".into());
    }
    Ok((planar, polar))
}

/// [`prepare`] for a polygon with holes: outer ring counter-clockwise (a
/// ring around a pole as [`ring_planar`] closes it), holes clockwise and
/// over the outer ring's longitudes. A hole may not go around a pole.
fn prepare_with_holes(ring: &[LonLat], holes: &[Vec<LonLat>]) -> Result<Prepared, String> {
    if holes.is_empty() {
        return prepare(ring);
    }
    let (mut outer, polar) = ring_planar(ring)?;
    if !polar && geom::signed_area(&outer) < 0.0 {
        outer.reverse();
    }
    let ob = geom::bbox(&outer);
    let mid = (ob[0] + ob[2]) / 2.0;
    let mut rings = vec![outer];
    for h in holes {
        let (mut hp, hole_polar) = ring_planar(h)?;
        if hole_polar {
            return Err("洞不能环绕极点".into());
        }
        let shift = ((mid - hp[0][0]) / 360.0).round() * 360.0;
        for q in hp.iter_mut() {
            q[0] += shift;
        }
        if geom::signed_area(&hp) > 0.0 {
            hp.reverse();
        }
        rings.push(hp);
    }
    let query_bb = geom::bbox(&rings[0]);
    let p_parts = seam::split(&rings);
    Ok(Prepared { p_mp: to_mp(&p_parts), pb: bbox(&p_parts), query_bb })
}

fn prepare(ring: &[LonLat]) -> Result<Prepared, String> {
    let (planar, _) = ring_planar(ring)?;
    let query_bb = geom::bbox(&planar);
    let p_parts = seam::split(&[planar]);
    Ok(Prepared { p_mp: to_mp(&p_parts), pb: bbox(&p_parts), query_bb })
}

/// A piece of the result: shape, owner, and the face it came from (None
/// for new land).
type Piece = (Parts, Option<EntityId>, Option<FaceId>);

/// New shapes for the given land faces under `intent`, plus new land.
/// Returns (polygons with owners, faces that actually changed).
fn emit(
    doc: &Document,
    faces: &[FaceId],
    prep: &Prepared,
    intent: DrawIntent,
    rep: &mut OverlayReport,
) -> Result<(Vec<Piece>, BTreeSet<FaceId>), String> {
    let r = doc.radius();
    let pb = prep.pb;
    let touches = |b: &[f64; 4]| !(b[0] > pb[2] || pb[0] > b[2] || b[1] > pb[3] || pb[1] > b[3]);
    let mut out: Vec<Piece> = Vec::new();
    let mut changed = BTreeSet::new();
    let mut land_union = geo::MultiPolygon::<f64>::new(vec![]);
    let mut taken: HashMap<Option<EntityId>, f64> = HashMap::new();
    for fid in faces {
        let face = &doc.topo.faces[fid];
        if face.kind != FaceKind::Land {
            continue; // water re-emerges from unclaimed space
        }
        let owner = doc.owner.get(fid).copied();
        let (outer, holes) = doc.topo.face_planar(*fid);
        let mut rings = vec![outer];
        rings.extend(holes);
        let parts = seam::split(&rings);
        if !touches(&bbox(&parts)) {
            out.push((parts, owner, Some(*fid)));
            continue;
        }
        let f_mp = to_mp(&parts);
        let inter = from_mp(&f_mp.intersection(&prep.p_mp));
        let inter_area = area(&inter, r);
        if inter_area <= 0.0 {
            out.push((parts, owner, Some(*fid)));
            continue;
        }
        land_union = land_union.union(&f_mp);
        let rest = || from_mp(&f_mp.difference(&prep.p_mp));
        match intent {
            DrawIntent::AddLand => out.push((parts, owner, Some(*fid))),
            DrawIntent::Expand(x) => {
                if owner.is_none() {
                    out.push((rest(), None, Some(*fid)));
                    out.push((inter, Some(x), Some(*fid)));
                    *taken.entry(None).or_default() += inter_area;
                    changed.insert(*fid);
                } else {
                    out.push((parts, owner, Some(*fid)));
                }
            }
            DrawIntent::Take(x) => {
                if owner == Some(x) {
                    out.push((parts, owner, Some(*fid)));
                } else {
                    out.push((rest(), owner, Some(*fid)));
                    out.push((inter, Some(x), Some(*fid)));
                    *taken.entry(owner).or_default() += inter_area;
                    changed.insert(*fid);
                }
            }
            DrawIntent::CarveWater => {
                out.push((rest(), owner, Some(*fid)));
                rep.carved_km2 += inter_area;
                changed.insert(*fid);
            }
        }
        // faces touched by new land along their coast are rebuilt too
        changed.insert(*fid);
    }
    let new_owner = match intent {
        DrawIntent::AddLand => Some(None),
        DrawIntent::Expand(x) | DrawIntent::Take(x) => Some(Some(x)),
        DrawIntent::CarveWater => None,
    };
    if let Some(owner) = new_owner {
        let fresh = from_mp(&prep.p_mp.difference(&land_union));
        rep.added_km2 = area(&fresh, r);
        out.push((fresh, owner, None));
    }
    rep.taken = taken.into_iter().collect();
    rep.taken.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    if rep.added_km2 <= 0.0 && rep.carved_km2 <= 0.0 && rep.taken.is_empty() {
        return Err("这个多边形不会改变任何陆地".into());
    }
    Ok((out, changed))
}

/// [`emit`] with every piece explicit (a land background, or a year
/// selected): water faces are emitted too, and the part of the polygon
/// over the background becomes a piece of its own whose source is the
/// background. Works on `faces` (all faces, or those near the polygon).
/// Returns the pieces, per piece whether it is water, and the faces the
/// polygon actually overlaps.
fn emit_explicit(doc: &Document, faces: &[FaceId], prep: &Prepared, intent: DrawIntent, rep: &mut OverlayReport) -> Result<(Vec<Piece>, Vec<bool>, BTreeSet<FaceId>), String> {
    let r = doc.radius();
    let pb = prep.pb;
    let touches = |b: &[f64; 4]| !(b[0] > pb[2] || pb[0] > b[2] || b[1] > pb[3] || pb[1] > b[3]);
    let mut out: Vec<Piece> = Vec::new();
    let mut water: Vec<bool> = Vec::new();
    let mut covered = geo::MultiPolygon::<f64>::new(vec![]);
    let mut taken: HashMap<Option<EntityId>, f64> = HashMap::new();
    let mut overlapped = BTreeSet::new();
    let mut push = |out: &mut Vec<Piece>, piece: Piece, is_water: bool| {
        if !piece.0.is_empty() {
            out.push(piece);
            water.push(is_water);
        }
    };
    for fid in faces {
        if *fid == BACKGROUND {
            continue;
        }
        let face = &doc.topo.faces[fid];
        let owner = doc.owner.get(fid).copied();
        let is_water = face.kind == FaceKind::Water;
        let (outer, holes) = doc.topo.face_planar(*fid);
        let mut rings = vec![outer];
        rings.extend(holes);
        let parts = seam::split(&rings);
        let f_mp = to_mp(&parts);
        let inter = if touches(&bbox(&parts)) { from_mp(&f_mp.intersection(&prep.p_mp)) } else { vec![] };
        let ia = area(&inter, r);
        if ia <= 0.0 {
            push(&mut out, (parts, owner, Some(*fid)), is_water);
            continue;
        }
        overlapped.insert(*fid);
        covered = covered.union(&f_mp);
        let rest = from_mp(&f_mp.difference(&prep.p_mp));
        match (is_water, intent) {
            (false, DrawIntent::AddLand) => push(&mut out, (parts, owner, Some(*fid)), false),
            (false, DrawIntent::Expand(_)) if owner.is_some() => push(&mut out, (parts, owner, Some(*fid)), false),
            (false, DrawIntent::Take(x)) if owner == Some(x) => push(&mut out, (parts, owner, Some(*fid)), false),
            (false, DrawIntent::Expand(x) | DrawIntent::Take(x)) => {
                push(&mut out, (rest, owner, Some(*fid)), false);
                push(&mut out, (inter, Some(x), Some(*fid)), false);
                *taken.entry(owner).or_default() += ia;
            }
            (false, DrawIntent::CarveWater) => {
                push(&mut out, (rest, owner, Some(*fid)), false);
                push(&mut out, (inter, None, Some(*fid)), true);
                rep.carved_km2 += ia;
            }
            (true, DrawIntent::CarveWater) => push(&mut out, (parts, None, Some(*fid)), true),
            (true, DrawIntent::AddLand | DrawIntent::Expand(_) | DrawIntent::Take(_)) => {
                let who = match intent {
                    DrawIntent::Expand(x) | DrawIntent::Take(x) => Some(x),
                    _ => None,
                };
                push(&mut out, (rest, None, Some(*fid)), true);
                push(&mut out, (inter, who, Some(*fid)), false);
                rep.added_km2 += ia;
            }
        }
    }
    // the polygon over the background itself (its pieces come from the
    // background, so their history is the background's)
    let bg_owner = doc.owner.get(&BACKGROUND).copied();
    let bg = Some(BACKGROUND);
    let bgp = from_mp(&prep.p_mp.difference(&covered));
    let ba = area(&bgp, r);
    if ba > 0.0 && doc.background_kind() == FaceKind::Water {
        // a sea background: new land
        match intent {
            DrawIntent::AddLand => push(&mut out, (bgp, None, bg), false),
            DrawIntent::Expand(x) | DrawIntent::Take(x) => push(&mut out, (bgp, Some(x), bg), false),
            DrawIntent::CarveWater => {}
        }
        if intent != DrawIntent::CarveWater {
            rep.added_km2 += ba;
        }
    } else if ba > 0.0 {
        match intent {
            DrawIntent::AddLand => {}
            DrawIntent::Expand(x) if bg_owner.is_none() => {
                push(&mut out, (bgp, Some(x), bg), false);
                *taken.entry(None).or_default() += ba;
            }
            DrawIntent::Take(x) if bg_owner != Some(x) => {
                push(&mut out, (bgp, Some(x), bg), false);
                *taken.entry(bg_owner).or_default() += ba;
            }
            DrawIntent::Expand(_) | DrawIntent::Take(_) => {}
            DrawIntent::CarveWater => {
                push(&mut out, (bgp, None, bg), true);
                rep.carved_km2 += ba;
            }
        }
    }
    rep.taken = taken.into_iter().collect();
    rep.taken.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
    if rep.added_km2 <= 0.0 && rep.carved_km2 <= 0.0 && rep.taken.is_empty() {
        return Err("这个多边形不会改变任何陆地".into());
    }
    Ok((out, water, overlapped))
}

/// Per rebuilt polygon (one per part of each piece): owner and source face.
struct Rebuilt {
    topo: Topology,
    owners: Vec<Option<EntityId>>,
    srcs: Vec<Option<FaceId>>,
    /// Polygon index → faces it became.
    faces: HashMap<usize, Vec<FaceId>>,
}

impl Rebuilt {
    /// New face → the old face its polygon was cut from.
    fn src_of(&self) -> HashMap<FaceId, Option<FaceId>> {
        self.faces.iter().flat_map(|(i, fs)| fs.iter().map(move |f| (*f, self.srcs[*i]))).collect()
    }
}

/// The exact corners before the boolean kernel rounded them (strip
/// coordinates): the drawn polygon's, and those of every face the pieces
/// were cut from, for the repair to put the output back onto. Without
/// them a border is left a hair (~1e-8°) off where it was, even far from
/// the polygon, and a later polygon drawn exactly along it leaves slivers.
/// The faces' corners come first: where a drawn corner is a rounded copy
/// of one of them, the map's own point stays. Corners near a pole or the
/// antimeridian are left out: there the output is already put exactly on
/// the pole and the seam ([`at_pole`], [`on_seam`]), and a corner a hair
/// from them would pull it off again (a polar cap became a face of no area).
fn exact_corners(doc: &Document, prep: &Prepared, out: &[Piece]) -> Vec<P2> {
    let mut v: Vec<P2> = Vec::new();
    let srcs: BTreeSet<FaceId> = out.iter().filter_map(|p| p.2).filter(|f| doc.topo.faces.contains_key(f)).collect();
    for f in srcs {
        let (outer, holes) = doc.topo.face_planar(f);
        let mut rings = vec![outer];
        rings.extend(holes);
        v.extend(seam::split(&rings).into_iter().flatten().flatten());
    }
    v.extend(prep.p_mp.0.iter().flat_map(|p| std::iter::once(p.exterior()).chain(p.interiors()).flat_map(|ls| ls.coords().map(|c| [c.x, c.y]))));
    v.retain(|p| p[1].abs() < 90.0 - 1e-6 && p[0].abs() < 180.0 - 1e-6);
    v
}

fn rebuild(doc: &Document, out: &[Piece], forced: Vec<LonLat>, fill_slivers: bool, prefer: Vec<P2>) -> Result<Rebuilt, String> {
    let mut polys = Vec::new();
    let mut owners = Vec::new();
    let mut srcs = Vec::new();
    for (parts, owner, src) in out {
        for p in parts {
            polys.push(p.clone());
            owners.push(*owner);
            srcs.push(*src);
        }
    }
    let feature_of: Vec<usize> = (0..polys.len()).collect();
    // the parts are disjoint by construction, so the expensive pairwise
    // overlap removal is skipped; snapping still reconciles the boolean
    // kernel's rounded coordinates with their exact neighbours
    let opts = RepairOptions {
        snap_m: 0.05,
        fill_slivers,
        radius_km: doc.radius(),
        first_id: doc.topo.next_id(),
        resolve_overlaps: false,
        forced_junctions: forced,
        prefer,
    };
    let fixed = repair::repair_strip(&polys, &feature_of, &opts).map_err(|e| format!("重建拓扑失败：{}", e.problems.join("；")))?;
    Ok(Rebuilt { topo: fixed.topo, owners, srcs, faces: fixed.faces })
}

/// Apply a drawn polygon to the document. Returns the new document; the
/// input is not modified (callers wrap this in an undoable edit).
///
/// Only the land faces the polygon touches are rebuilt and spliced back
/// into the untouched topology. If the splice cannot be done safely (for
/// example the polygon reaches into a lake), the whole world is rebuilt
/// instead; the report says which path was taken.
pub fn apply(doc: &Document, ring: &[LonLat], intent: DrawIntent) -> Result<(Document, OverlayReport), String> {
    apply_with_holes(doc, ring, &[], intent)
}

/// [`apply`] for a polygon with holes: nothing inside a hole changes (a
/// lake, another country's enclave, land handled apart). The outer ring
/// may go around a pole; a hole may not.
pub fn apply_with_holes(doc: &Document, ring: &[LonLat], holes: &[Vec<LonLat>], intent: DrawIntent) -> Result<(Document, OverlayReport), String> {
    let t0 = Instant::now();
    let prep = prepare_with_holes(ring, holes)?;
    let mut rep = OverlayReport { faces_before: doc.topo.faces.len() - 1, ..Default::default() };
    let result = match apply_local(doc, &prep, intent, &mut rep) {
        Ok(d) => {
            rep.local = true;
            d
        }
        Err(Fallback::User(msg)) => return Err(msg),
        Err(Fallback::Rebuild(why)) => {
            rep = OverlayReport { faces_before: doc.topo.faces.len() - 1, fallback: Some(why), ..Default::default() };
            apply_whole(doc, &prep, intent, &mut rep)?
        }
    };
    rep.faces_after = result.topo.faces.len() - 1;
    rep.ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok((result, rep))
}

/// Rebuild the whole world (always works, cost proportional to its size).
pub fn apply_whole_world(doc: &Document, ring: &[LonLat], intent: DrawIntent) -> Result<(Document, OverlayReport), String> {
    let t0 = Instant::now();
    let prep = prepare(ring)?;
    let mut rep = OverlayReport { faces_before: doc.topo.faces.len() - 1, ..Default::default() };
    let d = apply_whole(doc, &prep, intent, &mut rep)?;
    rep.faces_after = d.topo.faces.len() - 1;
    rep.ms = t0.elapsed().as_secs_f64() * 1e3;
    Ok((d, rep))
}

fn apply_whole(doc: &Document, prep: &Prepared, intent: DrawIntent, rep: &mut OverlayReport) -> Result<Document, String> {
    let all: Vec<FaceId> = doc.topo.faces.keys().copied().filter(|f| *f != BACKGROUND).collect();
    let land_bg = doc.background_kind() == FaceKind::Land;
    // with a year selected every piece (water too) is explicit, so each new
    // face can keep its source's land/water and owner history
    let explicit = land_bg || doc.year.is_some();
    let bg_kind = doc.background_kind();
    let (out, water) = if explicit {
        let (o, w, _) = emit_explicit(doc, &all, prep, intent, rep)?;
        (o, w)
    } else {
        (emit(doc, &all, prep, intent, rep)?.0, vec![])
    };
    // sliver filling merges thin unclaimed gaps into land, which is only
    // right when unclaimed space is sea and the pieces are implicit
    let rb = rebuild(doc, &out, vec![], !explicit, exact_corners(doc, prep, &out))?;
    let mut src_of = rb.src_of();
    let (mut topo, owners, faces) = (rb.topo, rb.owners, rb.faces);
    let mut new = doc.clone();
    new.owner = imbl::OrdMap::new();
    for (i, fs) in &faces {
        if let Some(e) = owners[*i] {
            for f in fs {
                new.owner.insert(*f, e);
            }
        }
    }
    if explicit {
        // kinds come from the pieces; unclaimed space (the background and
        // anything it encloses) is of the background's kind and owner
        let bg_owner = doc.owner.get(&BACKGROUND).copied();
        // `faces` is keyed by rebuilt polygon: one per part of each piece
        let water: Vec<bool> = out.iter().zip(&water).flat_map(|(p, w)| std::iter::repeat_n(*w, p.0.len())).collect();
        debug_assert_eq!(water.len(), owners.len());
        let mut kind_of: HashMap<FaceId, FaceKind> = HashMap::new();
        for (i, fs) in &faces {
            for f in fs {
                kind_of.insert(*f, if water.get(*i).copied().unwrap_or(false) { FaceKind::Water } else { FaceKind::Land });
            }
        }
        let ids: Vec<FaceId> = topo.faces.keys().copied().collect();
        for f in ids {
            let kind = kind_of.get(&f).copied().unwrap_or(bg_kind);
            if !kind_of.contains_key(&f) {
                if let Some(e) = bg_owner {
                    new.owner.insert(f, e);
                }
                if f != BACKGROUND {
                    src_of.insert(f, Some(BACKGROUND));
                }
            }
            let mut face = topo.faces[&f].clone();
            face.kind = kind;
            topo.faces.insert(f, face);
        }
        // a sea is one face, except when a year is selected: then water of
        // different histories (a lake and land flooded later) stays apart
        if doc.year.is_none() {
            merge_adjacent_water(&mut topo).map_err(|e| format!("合并水域失败：{e}"))?;
        }
        new.owner = new.owner.into_iter().filter(|(f, _)| topo.faces.contains_key(f)).collect();
    }
    new.topo = topo;
    src_of.retain(|f, _| new.topo.faces.contains_key(f));
    new.adopt_owner_history(doc, &src_of)?;
    rep.rebuilt_faces = all.len();
    Ok(new)
}

/// Merge water faces that share an edge (a carved sea next to an existing
/// one), so a sea is one face. The background is never merged away.
fn merge_adjacent_water(t: &mut Topology) -> Result<(), String> {
    loop {
        let pair = t.edges.values().find_map(|e| {
            let (l, r) = (e.left, e.right);
            (l != r && l != BACKGROUND && r != BACKGROUND && t.faces[&l].kind == FaceKind::Water && t.faces[&r].kind == FaceKind::Water).then_some((l, r))
        });
        let Some((a, b)) = pair else { return Ok(()) };
        t.merge_faces(a, b)?;
    }
}

enum Fallback {
    /// A problem the user must fix (invalid polygon, nothing to do).
    User(String),
    /// The local splice is not safe here; rebuild everything instead.
    Rebuild(String),
}

/// A point just beside the longest segment of `e`, on its left or right
/// (a rounding stub of 1e-8° would put the point practically on the line).
fn beside(t: &Topology, e: &Edge, left: bool) -> LonLat {
    let c = t.edge_full(e);
    let seg_len = |i: usize| {
        let (a, b) = (c[i], c[i + 1]);
        geom::lon_delta(a.lon, b.lon).hypot(b.lat - a.lat)
    };
    let i = (0..c.len() - 1).max_by(|x, y| seg_len(*x).total_cmp(&seg_len(*y))).unwrap_or(0);
    let (a, b) = (c[i], c[i + 1]);
    let dx = geom::lon_delta(a.lon, b.lon);
    let dy = b.lat - a.lat;
    let len = (dx * dx + dy * dy).sqrt().max(1e-12);
    let off = (len * 0.01).min(1e-6);
    let s = if left { 1.0 } else { -1.0 };
    LonLat::new(geom::wrap_lon(a.lon + dx * 0.5 - dy / len * off * s), a.lat + dy * 0.5 + dx / len * off * s)
}

fn apply_local(doc: &Document, prep: &Prepared, intent: DrawIntent, rep: &mut OverlayReport) -> Result<Document, Fallback> {
    let r = doc.radius();
    let t = &doc.topo;
    if doc.background_kind() == FaceKind::Land {
        return Err(Fallback::Rebuild("背景为陆地".into()));
    }
    // With a year selected every piece is explicit, as in the full rebuild:
    // the parts of each overlapped face (land or water) and the sea under
    // the polygon (sourced from the background), each adopting its source's
    // land/water and owner history, so sea that becomes land or land carved
    // into water keeps what it was before the year.
    let timed = doc.year.is_some();
    let prof = std::env::var_os("ATLAS_PROFILE").is_some();
    let mut tp = Instant::now();
    let mut lap = |name: &str| {
        if prof {
            eprintln!("  local {name}: {:.1} ms", tp.elapsed().as_secs_f64() * 1e3);
        }
        tp = Instant::now();
    };
    // candidate faces from the spatial index
    let cands = t.faces_in_bbox(prep.query_bb);
    let (out, water, w): (Vec<Piece>, Vec<bool>, BTreeSet<FaceId>) = if timed {
        let (all, water, w) = emit_explicit(doc, &cands, prep, intent, rep).map_err(Fallback::User)?;
        // pieces of the overlapped faces and of the sea under the polygon
        let keep = |p: &Piece| p.2.is_some_and(|f| f == BACKGROUND || w.contains(&f));
        let water = all.iter().zip(water).filter(|(p, _)| keep(p)).map(|(_, x)| x).collect();
        (all.into_iter().filter(keep).collect(), water, w)
    } else {
        // reaching into an existing lake needs the full rebuild
        for f in &cands {
            if t.faces[f].kind == FaceKind::Water {
                let (outer, holes) = t.face_planar(*f);
                let mut rings = vec![outer];
                rings.extend(holes);
                if area(&from_mp(&to_mp(&seam::split(&rings)).intersection(&prep.p_mp)), r) > 0.0 {
                    return Err(Fallback::Rebuild("多边形触及湖泊".into()));
                }
            }
        }
        let land: Vec<FaceId> = cands.into_iter().filter(|f| t.faces[f].kind == FaceKind::Land).collect();
        let (all_out, w) = emit(doc, &land, prep, intent, rep).map_err(Fallback::User)?;
        // work region W: land faces that intersect the polygon; everything
        // else keeps its records and ids. Pieces of faces outside W are dropped.
        (all_out.into_iter().filter(|(_, _, src)| src.is_none_or(|f| w.contains(&f))).collect(), vec![], w)
    };
    lap("emit");
    rep.rebuilt_faces = w.len();

    // edges of W: internal ones disappear, frozen boundary ones are rebuilt
    let mut w_edges: BTreeSet<EdgeId> = BTreeSet::new();
    for f in &w {
        let face = &t.faces[f];
        for rr in std::iter::once(&face.outer).chain(face.holes.iter()).flatten() {
            w_edges.insert(rr.edge);
        }
    }
    let mut forced: Vec<LonLat> = Vec::new();
    for e in &w_edges {
        let ed = &t.edges[e];
        if !(w.contains(&ed.left) && w.contains(&ed.right)) {
            forced.push(t.node_pos(ed.a));
            forced.push(t.node_pos(ed.b));
        }
    }
    lap("edges");
    let rb = rebuild(doc, &out, forced, false, exact_corners(doc, prep, &out)).map_err(Fallback::Rebuild)?;
    let src_of = rb.src_of();
    let (mut sub, owners, sub_faces) = (rb.topo, rb.owners, rb.faces);
    let mut sub_owner: HashMap<FaceId, EntityId> = HashMap::new();
    for (i, fs) in &sub_faces {
        if let Some(e) = owners[*i] {
            for f in fs {
                sub_owner.insert(*f, e);
            }
        }
    }
    // explicit pieces: each rebuilt face is one, of its piece's kind
    // (`sub_faces` is keyed by rebuilt polygon: one per part of each piece)
    let pieces: BTreeSet<FaceId> = if timed {
        let water: Vec<bool> = out.iter().zip(&water).flat_map(|(p, w)| std::iter::repeat_n(*w, p.0.len())).collect();
        for (i, fs) in &sub_faces {
            for f in fs {
                let mut face = sub.faces[f].clone();
                face.kind = if water.get(*i).copied().unwrap_or(false) { FaceKind::Water } else { FaceKind::Land };
                sub.faces.insert(*f, face);
            }
        }
        sub_faces.values().flatten().copied().collect()
    } else {
        BTreeSet::new()
    };

    // where does each unclaimed side of the sub-topology really lie?
    #[derive(Clone, Copy, PartialEq, Debug)]
    enum Side {
        Keep,            // a face of the sub-result
        Outside(FaceId), // an untouched face (or the ocean)
        Gap,             // nothing (explicit pieces should cover W)
    }
    let side_of = |e: &Edge, left: bool| -> Side {
        let sf = if left { e.left } else { e.right };
        if timed {
            if pieces.contains(&sf) {
                return Side::Keep;
            }
            let orig = t.face_at(beside(&sub, e, left));
            return if w.contains(&orig) { Side::Gap } else { Side::Outside(orig) };
        }
        if sf != BACKGROUND && sub.faces[&sf].kind == FaceKind::Land {
            return Side::Keep;
        }
        let orig = t.face_at(beside(&sub, e, left));
        if !w.contains(&orig) {
            Side::Outside(orig)
        } else if sf == BACKGROUND {
            Side::Outside(BACKGROUND) // carved bay connected to the sea
        } else {
            Side::Keep // new water inside the former work region
        }
    };
    // a water face of the sub-result is either new water (kept as a face)
    // or untouched faces seen through it (each edge then takes the face
    // beside it, which may differ edge to edge: with a year the sea is
    // kept as pieces of different histories); new water may not touch them
    let mut lake_map: HashMap<FaceId, Option<Side>> = HashMap::new();
    let mut sides: HashMap<EdgeId, (Side, Side)> = HashMap::new();
    for (id, e) in sub.edges.iter() {
        let (l, rr) = (side_of(e, true), side_of(e, false));
        if l == Side::Gap || rr == Side::Gap {
            return Err(Fallback::Rebuild("局部结果留有空隙".into()));
        }
        for (sf, s) in [(e.left, l), (e.right, rr)] {
            if !timed && sf != BACKGROUND && sub.faces[&sf].kind == FaceKind::Water {
                let entry = lake_map.entry(sf).or_insert(Some(s));
                if (*entry == Some(Side::Keep)) != (s == Side::Keep) {
                    return Err(Fallback::Rebuild("新水域与已有水域相连".into()));
                }
            }
        }
        sides.insert(*id, (l, rr));
    }

    lap("rebuild+sides");
    // ---- splice
    let mut new = doc.clone();
    let nt = &mut new.topo;
    nt.reserve_ids(sub.next_id());
    for f in &w {
        nt.remove_face_raw(*f);
        new.owner.remove(f);
    }
    // faces outside W whose rings referenced removed edges
    let mut outside: BTreeSet<FaceId> = BTreeSet::new();
    for e in &w_edges {
        let ed = &t.edges[e];
        for f in [ed.left, ed.right] {
            if !w.contains(&f) {
                outside.insert(f);
            }
        }
        nt.remove_edge_raw(*e);
    }
    // nodes: reuse existing ones at identical positions on the frozen boundary
    let bits = |p: LonLat| (p.lon.to_bits(), p.lat.to_bits());
    let mut by_pos: HashMap<(u64, u64), NodeId> = HashMap::new();
    for e in &w_edges {
        let ed = &t.edges[e];
        for n in [ed.a, ed.b] {
            by_pos.insert(bits(t.node_pos(n)), n);
        }
    }
    let mut node_map: HashMap<NodeId, NodeId> = HashMap::new();
    for (id, n) in sub.nodes.iter() {
        let target = match by_pos.get(&bits(n.pos)) {
            Some(old) => *old,
            None => {
                nt.nodes.insert(*id, n.clone());
                *id
            }
        };
        node_map.insert(*id, target);
    }
    let final_face = |sf: FaceId, s: Side| -> FaceId {
        match s {
            Side::Outside(o) => o,
            Side::Keep | Side::Gap => sf,
        }
    };
    // kept sub faces: the explicit pieces; else land faces, and water faces
    // that map to "keep"
    let kept: BTreeSet<FaceId> = if timed {
        pieces.clone()
    } else {
        sub.faces
            .iter()
            .filter(|(id, f)| **id != BACKGROUND && (f.kind == FaceKind::Land || lake_map.get(id) == Some(&Some(Side::Keep))))
            .map(|(id, _)| *id)
            .collect()
    };
    let mut new_edges: BTreeSet<EdgeId> = BTreeSet::new();
    for (id, e) in sub.edges.iter() {
        let (ls, rs) = sides[id];
        let mut ne = e.clone();
        ne.a = node_map[&e.a];
        ne.b = node_map[&e.b];
        ne.left = final_face(e.left, ls);
        ne.right = final_face(e.right, rs);
        if ne.left == ne.right {
            return Err(Fallback::Rebuild("拼接后出现两侧同面的边".into()));
        }
        for f in [ne.left, ne.right] {
            if !kept.contains(&f) {
                outside.insert(f);
            }
        }
        nt.put_edge(*id, ne);
        new_edges.insert(*id);
    }
    for f in &kept {
        nt.faces.insert(*f, sub.faces[f].clone());
        if let Some(e) = sub_owner.get(f) {
            new.owner.insert(*f, *e);
        }
    }
    // re-assemble the rings of untouched neighbours that lost edges
    for o in &outside {
        let Some(face) = nt.faces.get(o).cloned() else {
            return Err(Fallback::Rebuild("相邻面不存在".into()));
        };
        let rings: Vec<(bool, Vec<ArcRef>)> = std::iter::once((true, face.outer.clone())).chain(face.holes.iter().map(|h| (false, h.clone()))).filter(|(_, r)| !r.is_empty()).collect();
        let mut keep_rings: Vec<(bool, Vec<ArcRef>)> = Vec::new();
        let mut refs: Vec<ArcRef> = Vec::new();
        for (is_outer, ring) in rings {
            if ring.iter().any(|rr| w_edges.contains(&rr.edge)) {
                refs.extend(ring.into_iter().filter(|rr| !w_edges.contains(&rr.edge)));
            } else {
                keep_rings.push((is_outer, ring));
            }
        }
        for id in &new_edges {
            let e = &nt.edges[id];
            if e.left == *o {
                refs.push(ArcRef { edge: *id, rev: false });
            }
            if e.right == *o {
                refs.push(ArcRef { edge: *id, rev: true });
            }
        }
        let assembled = nt.assemble_rings(&refs).map_err(|e| Fallback::Rebuild(format!("相邻面边界无法闭合：{e}")))?;
        let mut outer: Option<Vec<ArcRef>> = keep_rings.iter().find(|(o, _)| *o).map(|(_, r)| r.clone());
        let mut holes: Vec<Vec<ArcRef>> = keep_rings.into_iter().filter(|(o, _)| !*o).map(|(_, r)| r).collect();
        for ring in assembled {
            let positive = !nt.is_polar(&ring) && geom::signed_area(&nt.ring_planar(&ring)) > 0.0;
            if *o == BACKGROUND && positive {
                // the edit closed a strait: sea the land now encloses is no
                // longer part of the background but a water face of its own
                // (the ring may hold no new edge at all: the old coast of
                // that sea), which the full rebuild makes
                return Err(Fallback::Rebuild("陆地围出了内海".into()));
            }
            if *o != BACKGROUND && positive {
                if outer.is_some() {
                    return Err(Fallback::Rebuild("相邻面出现多个外环".into()));
                }
                outer = Some(ring);
            } else {
                holes.push(ring);
            }
        }
        let f = nt.faces.get_mut(o).unwrap();
        f.outer = if *o == BACKGROUND { vec![] } else { outer.ok_or_else(|| Fallback::Rebuild("相邻面失去外环".into()))? };
        f.holes = holes;
    }
    for f in kept.iter().chain(outside.iter()) {
        nt.touch(*f);
    }
    // drop nodes no edge uses any more
    let mut orphan = Vec::new();
    for e in &w_edges {
        let ed = &t.edges[e];
        for n in [ed.a, ed.b] {
            if nt.nodes.contains_key(&n) && nt.edges_of_node(n).is_empty() {
                orphan.push(n);
            }
        }
    }
    for n in orphan {
        nt.nodes.remove(&n);
    }
    let check: BTreeSet<FaceId> = kept.iter().chain(outside.iter()).copied().collect();
    lap("splice");
    nt.validate_local(&check, &new_edges).map_err(|e| Fallback::Rebuild(format!("局部校验失败：{e}")))?;
    let src_of: HashMap<FaceId, Option<FaceId>> = kept.iter().map(|f| (*f, src_of.get(f).copied().flatten())).collect();
    lap("validate");
    new.adopt_owner_history(doc, &src_of).map_err(Fallback::User)?;
    lap("adopt");
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::EntityKind;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<LonLat> {
        vec![LonLat::new(x0, y0), LonLat::new(x1, y0), LonLat::new(x1, y1), LonLat::new(x0, y1)]
    }
    fn rect_area(x0: f64, y0: f64, x1: f64, y1: f64, r: f64) -> f64 {
        geom::spherical_area(&[[x0, y0], [x1, y0], [x1, y1], [x0, y1]], r)
    }

    /// Two neighbouring countries A (unowned land for some tests) and B, plus
    /// a far island that must never change.
    fn world(a_owned: bool) -> (Document, Option<EntityId>, EntityId) {
        let mut d = Document::default();
        d.add_ring_face(&rect(0.0, 0.0, 20.0, 10.0), FaceKind::Land).unwrap();
        let (_, fb) = d.split_face(&[LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0)]).unwrap();
        d.add_ring_face(&rect(100.0, 30.0, 105.0, 35.0), FaceKind::Land).unwrap();
        let fa = d.topo.face_at(LonLat::new(5.0, 5.0));
        let a = a_owned.then(|| {
            let a = d.add_entity(EntityKind::Country, "A");
            d.assign(fa, Some(a)).unwrap();
            a
        });
        let b = d.add_entity(EntityKind::Country, "B");
        d.assign(fb, Some(b)).unwrap();
        (d, a, b)
    }

    fn area_of(d: &Document, e: EntityId) -> f64 {
        d.areas_by_entity().get(&e).copied().unwrap_or(0.0)
    }

    fn island_coords(d: &Document) -> Vec<(u64, u64)> {
        let f = d.topo.face_at(LonLat::new(102.0, 32.0));
        let mut v: Vec<(u64, u64)> = d.topo.ring_coords(&d.topo.faces[&f].outer).iter().map(|p| (p.lon.to_bits(), p.lat.to_bits())).collect();
        v.sort();
        v
    }

    #[test]
    fn add_land_only_fills_water() {
        let (d, a, b) = world(true);
        let r = d.radius();
        // overlaps B and the sea east of it
        let (n, rep) = apply(&d, &rect(15.0, 2.0, 25.0, 8.0), DrawIntent::AddLand).unwrap();
        n.topo.validate().unwrap();
        assert!((rep.added_km2 - rect_area(20.0, 2.0, 25.0, 8.0, r)).abs() / rep.added_km2 < 1e-6);
        assert!((area_of(&n, b) - area_of(&d, b)).abs() < 1e-3, "B unchanged");
        assert!((area_of(&n, a.unwrap()) - area_of(&d, a.unwrap())).abs() < 1e-3);
        // the new land shares a border with B instead of overlapping it
        let new_face = n.topo.face_at(LonLat::new(22.0, 5.0));
        assert!(n.owner.get(&new_face).is_none());
        assert!(n.topo.edges.values().any(|e| (e.left == new_face || e.right == new_face) && e.left != BACKGROUND && e.right != BACKGROUND));
        assert_eq!(island_coords(&n), island_coords(&d), "untouched faces keep exact coordinates");
        assert!(n.topo.next_id() >= d.topo.next_id(), "id counter must not go back");
    }

    #[test]
    fn expand_takes_water_and_unclaimed_land_only() {
        let (d, _, b) = world(false); // A's face is unclaimed
        let r = d.radius();
        let mut d = d;
        let x = d.add_entity(EntityKind::Country, "X");
        let (n, _) = apply(&d, &rect(5.0, 2.0, 25.0, 8.0), DrawIntent::Expand(x)).unwrap();
        n.topo.validate().unwrap();
        let expect = rect_area(5.0, 2.0, 10.0, 8.0, r) + rect_area(20.0, 2.0, 25.0, 8.0, r);
        assert!((area_of(&n, x) - expect).abs() / expect < 1e-6, "{} vs {expect}", area_of(&n, x));
        assert!((area_of(&n, b) - area_of(&d, b)).abs() < 1e-3, "B keeps its land");
        // entity ids allocated later never collide with the rebuilt topology
        let mut n = n;
        let y = n.add_entity(EntityKind::Country, "Y");
        assert!(!d.entities.contains_key(&y));
    }

    /// The kernel rounds every coordinate it outputs (~1e-8°): a drawn border
    /// keeps the drawn corners exactly, also when a later polygon redraws
    /// the face around it, so a polygon drawn later exactly along the same
    /// line meets it (automatic partition draws its units this way).
    #[test]
    fn drawn_borders_keep_their_exact_corners() {
        let (mut d, _, _) = world(true);
        let u: Vec<EntityId> = (0..3).map(|k| d.add_entity(EntityKind::Country, &format!("U{k}"))).collect();
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let p = |i: f64, j: f64| LonLat::new(1.0 + i * s, 1.0 + j * s * 1.1);
        let quad = |a: LonLat, b: LonLat| rect(a.lon, a.lat, b.lon, b.lat);
        // (as drawn: the polygon's longitudes are wrapped to ±180 first)
        let has = |d: &Document, q: LonLat| {
            let q = q.normalized();
            d.topo.edges.values().any(|e| d.topo.edge_full(e).iter().any(|x| x.lon.to_bits() == q.lon.to_bits() && x.lat.to_bits() == q.lat.to_bits()))
        };
        let first = quad(p(0.0, 0.0), p(2.0, 1.0));
        let (n, _) = apply(&d, &first, DrawIntent::Take(u[0])).unwrap();
        assert!(first.iter().all(|q| has(&n, *q)), "the drawn corners, exactly");
        // elsewhere in the same face: the face is rebuilt around the first border
        let (n, _) = apply(&n, &quad(p(6.0, 6.0), p(7.0, 7.0)), DrawIntent::Take(u[1])).unwrap();
        assert!(first.iter().all(|q| has(&n, *q)), "still exactly, after the face was redrawn");
        // along part of the first border, with a corner in the middle of it
        let third = quad(p(1.0, 1.0), p(3.0, 2.0));
        let (n, _) = apply(&n, &third, DrawIntent::Take(u[2])).unwrap();
        n.topo.validate().unwrap();
        assert!(third.iter().all(|q| has(&n, *q)));
        let r = n.radius();
        for (e, (a, b)) in [(u[0], (p(0.0, 0.0), p(2.0, 1.0))), (u[2], (p(1.0, 1.0), p(3.0, 2.0)))] {
            let want = rect_area(a.lon, a.lat, b.lon, b.lat, r);
            assert!((area_of(&n, e) - want).abs() / want < 1e-9, "{} vs {want}", area_of(&n, e));
        }
    }

    #[test]
    fn take_moves_land_between_countries() {
        let (d, a, b) = world(true);
        let r = d.radius();
        let (n, rep) = apply(&d, &rect(5.0, 2.0, 15.0, 8.0), DrawIntent::Take(b)).unwrap();
        n.topo.validate().unwrap();
        let moved = rect_area(5.0, 2.0, 10.0, 8.0, r);
        let a = a.unwrap();
        assert!((area_of(&d, a) - area_of(&n, a) - moved).abs() / moved < 1e-6);
        assert!((area_of(&n, b) - area_of(&d, b) - moved).abs() / moved < 1e-6);
        assert_eq!(rep.taken.len(), 1);
        assert_eq!(rep.taken[0].0, Some(a));
        // A is now split into a U shape around B's gain; still one valid world
        assert_eq!(n.topo.face_at(LonLat::new(7.0, 5.0)), *n.faces_of(b).iter().find(|f| n.topo.face_contains(**f, LonLat::new(7.0, 5.0))).unwrap());
    }

    #[test]
    fn carve_makes_a_lake_across_a_border() {
        let (d, a, b) = world(true);
        let r = d.radius();
        let (n, rep) = apply(&d, &rect(8.0, 3.0, 12.0, 7.0), DrawIntent::CarveWater).unwrap();
        n.topo.validate().unwrap();
        assert!((rep.carved_km2 - rect_area(8.0, 3.0, 12.0, 7.0, r)).abs() / rep.carved_km2 < 1e-6);
        let lake = n.topo.face_at(LonLat::new(10.0, 5.0));
        assert_eq!(n.topo.faces[&lake].kind, FaceKind::Water);
        let total = area_of(&n, a.unwrap()) + area_of(&n, b);
        let before = area_of(&d, a.unwrap()) + area_of(&d, b);
        assert!((before - total - rep.carved_km2).abs() / rep.carved_km2 < 1e-6);
    }

    #[test]
    fn works_across_the_antimeridian() {
        let mut d = Document::default();
        let f = d.add_ring_face(&rect(170.0, -10.0, 185.0, 10.0), FaceKind::Land).unwrap();
        let x = d.add_entity(EntityKind::Country, "X");
        d.assign(f, Some(x)).unwrap();
        let r = d.radius();
        // extend eastwards from 180+ into open sea
        let (n, rep) = apply(&d, &rect(178.0, -5.0, 195.0, 5.0), DrawIntent::Expand(x)).unwrap();
        n.topo.validate().unwrap();
        let expect = rect_area(185.0, -5.0, 195.0, 5.0, r);
        assert!((rep.added_km2 - expect).abs() / expect < 1e-6, "{} vs {expect}", rep.added_km2);
        let total = rect_area(170.0, -10.0, 185.0, 10.0, r) + expect;
        assert!((area_of(&n, x) - total).abs() / total < 1e-6);
    }

    #[test]
    fn nothing_to_do_is_an_error() {
        let (d, _, b) = world(true);
        // a polygon entirely inside B, taken by B itself
        assert!(apply(&d, &rect(12.0, 2.0, 18.0, 8.0), DrawIntent::Take(b)).is_err());
    }

    #[test]
    fn local_splice_matches_whole_rebuild() {
        use crate::sample::{grid_world, Rng};
        let d = grid_world(12, 10, 4, 4);
        let countries = d.countries();
        let mut rng = Rng::new(21);
        let (mut local, mut fell_back) = (0, Vec::new());
        for trial in 0..60 {
            let cx = rng.range(-160.0, 160.0);
            let cy = rng.range(-65.0, 75.0);
            let (w, h) = (rng.range(2.0, 25.0), rng.range(2.0, 20.0));
            let ring = vec![
                LonLat::new(cx, cy),
                LonLat::new(cx + w, cy + rng.range(-2.0, 2.0)),
                LonLat::new(cx + w * 0.8, cy + h),
                LonLat::new(cx - w * 0.2, cy + h * 0.7),
            ];
            let x = countries[(rng.next_f64() * countries.len() as f64) as usize];
            let intent = match trial % 4 {
                0 => DrawIntent::AddLand,
                1 => DrawIntent::Expand(x),
                2 => DrawIntent::Take(x),
                _ => DrawIntent::CarveWater,
            };
            let a = apply(&d, &ring, intent);
            let b = apply_whole_world(&d, &ring, intent);
            match (a, b) {
                (Ok((la, ra)), Ok((wb, _))) => {
                    la.topo.validate().unwrap_or_else(|e| panic!("trial {trial} {intent:?}: {e}"));
                    if ra.local {
                        local += 1;
                    } else {
                        fell_back.push(ra.fallback.clone().unwrap_or_default());
                    }
                    // Areas per owner. The local splice must leave every
                    // country it did not touch exactly as it was, and losers
                    // must lose exactly what the report says. The whole
                    // rebuild is only a loose reference: its sliver filling
                    // merges boolean noise anywhere on the map (it moved an
                    // untouched country by 0.5 km² in one trial).
                    let (a0, aa, ab) = (d.areas_by_entity(), la.areas_by_entity(), wb.areas_by_entity());
                    let gainer = match intent {
                        DrawIntent::Expand(x) | DrawIntent::Take(x) => Some(x),
                        _ => None,
                    };
                    let mut carved = 0.0;
                    for (e, v0) in &a0 {
                        let u = aa.get(e).copied().unwrap_or(0.0);
                        if Some(*e) == gainer {
                            continue;
                        }
                        let mut lost = ra.taken.iter().find(|t| t.0 == Some(*e)).map(|t| t.1).unwrap_or(0.0);
                        if intent == DrawIntent::CarveWater && u < v0 - 1e-9 * v0 {
                            // carved land is reported as one total, checked below
                            carved += v0 - u;
                            lost = v0 - u;
                        }
                        // 1e-8: borders inside the rebuilt region move by the boolean's ~1 mm quantization
                        let tol = if lost > 0.0 { 1e-6 * v0.max(1.0) } else { 1e-8 * v0.max(1.0) };
                        assert!((u - (v0 - lost)).abs() <= tol, "trial {trial} {intent:?}: entity {e:?} {u} vs expected {}", v0 - lost);
                    }
                    assert!(carved <= ra.carved_km2 * (1.0 + 1e-6) + 1e-6, "trial {trial}: countries lost {carved} but only {} was carved", ra.carved_km2);
                    for (e, v) in &ab {
                        let u = aa.get(e).copied().unwrap_or(0.0);
                        assert!((u - v).abs() <= 1e-5 * v.max(1.0), "trial {trial} {intent:?}: entity {e:?} {u} vs whole {v}");
                    }
                    let lakes = |d: &Document| d.topo.faces.iter().filter(|(id, f)| **id != BACKGROUND && f.kind == FaceKind::Water).map(|(id, _)| d.topo.face_area(*id, d.radius())).sum::<f64>();
                    assert!((lakes(&la) - lakes(&wb)).abs() <= 1e-5 * lakes(&wb).max(1.0), "trial {trial}: water area differs");
                    if ra.local {
                        // every face outside the rebuilt region keeps its id
                        let kept = d.topo.faces.keys().filter(|f| **f != BACKGROUND && la.topo.faces.contains_key(*f)).count();
                        assert_eq!(kept + ra.rebuilt_faces, d.topo.faces.len() - 1, "trial {trial}: {kept} kept, {} rebuilt", ra.rebuilt_faces);
                    }
                }
                (Err(x), Err(y)) => assert_eq!(x, y),
                (a, b) => panic!("trial {trial} {intent:?}: local {:?} vs whole {:?}", a.map(|x| x.1), b.map(|x| x.1)),
            }
        }
        eprintln!("local splices: {local}, fallbacks: {:?}", fell_back);
        assert!(local >= 40, "too many fallbacks: {fell_back:?}");
    }

    /// An all-land planet: the background is land owned by X; seas are
    /// carved out of it, other countries drawn into it. Areas must add up
    /// to the sphere exactly at every step.
    #[test]
    fn land_background_planet() {
        let mut d = Document::default();
        let r = d.radius();
        let sphere = 4.0 * std::f64::consts::PI * r * r;
        let x = d.add_entity(EntityKind::Country, "X");
        let y = d.add_entity(EntityKind::Country, "Y");
        d.set_background_kind(FaceKind::Land);
        d.assign(BACKGROUND, Some(x)).unwrap();
        assert!((d.areas_by_entity()[&x] - sphere).abs() < 1e-6 * sphere);
        let water = |d: &Document| d.topo.faces.iter().filter(|(_, f)| f.kind == FaceKind::Water).map(|(id, _)| d.topo.face_area(*id, r)).sum::<f64>();
        let check = |d: &Document, sea: f64, ax: f64, ay: f64| {
            d.topo.validate().unwrap();
            let a = d.areas_by_entity();
            let (gx, gy) = (a.get(&x).copied().unwrap_or(0.0), a.get(&y).copied().unwrap_or(0.0));
            assert!((water(d) - sea).abs() < 1e-6 * sphere, "sea {} vs {sea}", water(d));
            assert!((gx - ax).abs() < 1e-6 * sphere, "X {gx} vs {ax}");
            assert!((gy - ay).abs() < 1e-6 * sphere, "Y {gy} vs {ay}");
            assert!((gx + gy + water(d) - sphere).abs() < 1e-6 * sphere);
        };

        // carve a sea
        let (d1, rep) = apply(&d, &rect(0.0, 0.0, 40.0, 30.0), DrawIntent::CarveWater).unwrap();
        let sea = rect_area(0.0, 0.0, 40.0, 30.0, r);
        assert!(!rep.local, "a land background takes the whole rebuild");
        assert!((rep.carved_km2 - sea).abs() < 1e-6 * sea);
        check(&d1, sea, sphere - sea, 0.0);
        assert_eq!(d1.background_kind(), FaceKind::Land);
        assert_eq!(d1.owner.get(&BACKGROUND), Some(&x));
        assert_eq!(d1.topo.faces[&d1.topo.face_at(LonLat::new(20.0, 15.0))].kind, FaceKind::Water);

        // a second, overlapping sea merges into one water face
        let (d2, _) = apply(&d1, &rect(30.0, 10.0, 60.0, 20.0), DrawIntent::CarveWater).unwrap();
        let sea2 = sea + rect_area(40.0, 10.0, 60.0, 20.0, r);
        check(&d2, sea2, sphere - sea2, 0.0);
        assert_eq!(d2.topo.faces.values().filter(|f| f.kind == FaceKind::Water).count(), 1);

        // Y takes a box half on X's land, half on the sea
        let (d3, rep) = apply(&d2, &rect(-20.0, -10.0, 10.0, 10.0), DrawIntent::Take(y)).unwrap();
        let on_sea = rect_area(0.0, 0.0, 10.0, 10.0, r);
        let total = rect_area(-20.0, -10.0, 10.0, 10.0, r);
        assert!((rep.added_km2 - on_sea).abs() < 1e-6 * total);
        assert_eq!(rep.taken.len(), 1);
        assert_eq!(rep.taken[0].0, Some(x));
        assert!((rep.taken[0].1 - (total - on_sea)).abs() < 1e-6 * total);
        check(&d3, sea2 - on_sea, sphere - sea2 - (total - on_sea), total);

        // an island inside the sea, then expansion: X owns the background, so
        // expanding Y only claims sea
        let (d4, _) = apply(&d3, &rect(20.0, 12.0, 25.0, 18.0), DrawIntent::AddLand).unwrap();
        let island = rect_area(20.0, 12.0, 25.0, 18.0, r);
        let unowned_island = |d: &Document| d.topo.faces.iter().filter(|(id, f)| f.kind == FaceKind::Land && !d.owner.contains_key(*id)).map(|(id, _)| d.topo.face_area(*id, r)).sum::<f64>();
        assert!((unowned_island(&d4) - island).abs() < 1e-6 * island);
        let (d5, rep) = apply(&d4, &rect(-30.0, 25.0, 5.0, 40.0), DrawIntent::Expand(y)).unwrap();
        let gained = rect_area(0.0, 25.0, 5.0, 30.0, r);
        assert!((rep.added_km2 - gained).abs() < 1e-6 * gained, "{} vs {gained}", rep.added_km2);
        assert!(rep.taken.is_empty());
        d5.topo.validate().unwrap();
        // the background country gets a label somewhere on the background
        let (p, _) = d5.label_anchor(x).unwrap();
        assert_eq!(d5.topo.face_at(p), BACKGROUND);
        // back to a sea background: the background loses its owner
        let mut d6 = d5.clone();
        d6.set_background_kind(FaceKind::Water);
        assert!(!d6.owner.contains_key(&BACKGROUND));
    }

    /// Faces split at the antimeridian become several rebuilt polygons;
    /// every one must keep its own kind and owner.
    #[test]
    fn land_background_keeps_kinds_of_multi_part_faces() {
        let mut d = Document::default();
        let y = d.add_entity(EntityKind::Country, "Y");
        let lake = d.add_ring_face(&rect(170.0, 10.0, -170.0, 20.0), FaceKind::Water).unwrap();
        let island = d.add_ring_face(&rect(170.0, -20.0, -170.0, -10.0), FaceKind::Land).unwrap();
        d.assign(island, Some(y)).unwrap();
        d.set_background_kind(FaceKind::Land);
        let (n, _) = apply(&d, &rect(0.0, 0.0, 20.0, 20.0), DrawIntent::CarveWater).unwrap();
        n.topo.validate().unwrap();
        let kind_at = |lon: f64, lat: f64| n.topo.faces[&n.topo.face_at(LonLat::new(lon, lat))].kind;
        let owner_at = |lon: f64, lat: f64| n.owner.get(&n.topo.face_at(LonLat::new(lon, lat))).copied();
        for lon in [175.0, -175.0] {
            assert_eq!(kind_at(lon, 15.0), FaceKind::Water, "lake at {lon}");
            assert_eq!(kind_at(lon, -15.0), FaceKind::Land, "island at {lon}");
            assert_eq!(owner_at(lon, -15.0), Some(y));
        }
        assert_eq!(kind_at(10.0, 10.0), FaceKind::Water);
        assert_eq!(kind_at(-60.0, 40.0), FaceKind::Land);
        let _ = lake;
    }
}

