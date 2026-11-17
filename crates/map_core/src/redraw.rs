//! Redrawing a border: a line drawn from one point on a boundary to another
//! replaces the boundary between them. The land between the old boundary
//! and the new line changes hands — it goes to whoever is on the far side
//! of the old boundary (or becomes water when that side is sea). It is an
//! ownership change like any other, so with a year selected it holds from
//! that year on and earlier maps are untouched.

use crate::doc::Document;
use crate::geom::{self, LonLat};
use crate::overlay::{self, DrawIntent, OverlayReport};
use crate::topo::FaceKind;
use crate::EntityId;

/// What redrawing would do, before it is applied.
#[derive(Clone, Debug)]
pub struct Plan {
    /// The region that changes hands: the new line, then the old boundary back.
    pub ring: Vec<LonLat>,
    /// The old boundary between the two ends.
    pub old: Vec<LonLat>,
    pub intent: DrawIntent,
    /// Who gains the region (None: it becomes water).
    pub gainer: Option<EntityId>,
    /// Who held the region next to the old boundary (None: water or unclaimed).
    pub loser: Option<EntityId>,
}

/// Work out the region and who gains it. `tol` (degrees) is how far the
/// two ends may be from a boundary.
pub fn plan(doc: &Document, line: &[LonLat], tol: f64) -> Result<Plan, String> {
    if line.len() < 2 {
        return Err("新边界至少需要两个点".into());
    }
    let (a, b) = (line[0], *line.last().unwrap());
    let old = crate::trace::boundary_path(&doc.topo, a, b, tol, false).ok_or("新边界的起点和终点都要落在已有边界上")?;
    if old.len() < 2 || geom::geo_dist_deg(old[0], *old.last().unwrap()) < 1e-9 {
        return Err("起点和终点不能是同一点".into());
    }
    // the drawn line with its ends exactly on the boundary, then the old boundary back
    let mut ring: Vec<LonLat> = vec![old[0]];
    ring.extend_from_slice(&line[1..line.len() - 1]);
    ring.push(*old.last().unwrap());
    ring.extend(old[1..old.len() - 1].iter().rev());
    if ring.len() < 3 {
        return Err("新边界与原边界重合，没有变化".into());
    }
    let planar = geom::unwrap_path(&ring, None);
    if !geom::ring_is_simple(&planar) {
        return Err("新边界与原边界之间不能交叉（请让新线只在原边界的一侧）".into());
    }
    // the two sides of the old boundary, sampled along it: the far side
    // must be one country (or the sea) all the way
    let old_pl = geom::unwrap_path(&old, Some(planar[0][0]));
    let at = |s: [f64; 2]| {
        let ll = LonLat::new(geom::wrap_lon(s[0]), s[1]);
        let f = doc.topo.face_at(ll);
        let kind = doc.topo.faces.get(&f).map_or(FaceKind::Water, |x| x.kind);
        (kind, if kind == FaceKind::Land { doc.effective_owner_at(f, doc.year) } else { None })
    };
    let segs = old_pl.len() - 1;
    let step = (segs / 24).max(1);
    let mut far: Option<(FaceKind, Option<EntityId>)> = None;
    let mut near: Option<(FaceKind, Option<EntityId>, f64)> = None;
    for i in (0..segs).step_by(step) {
        let (p, q) = (old_pl[i], old_pl[i + 1]);
        let len = ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt();
        if len < 1e-9 {
            continue;
        }
        let m = [(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5];
        let eps = (len * 0.01).min(1e-3);
        let nrm = [-(q[1] - p[1]) / len * eps, (q[0] - p[0]) / len * eps];
        let s1 = [m[0] + nrm[0], m[1] + nrm[1]];
        let s2 = [m[0] - nrm[0], m[1] - nrm[1]];
        let (inside, outside) = if geom::point_in_poly(s1, &planar) { (s1, s2) } else { (s2, s1) };
        let o = at(outside);
        if far.is_some_and(|f| f != o) {
            return Err("新边界的两端要落在同一段边界上：原边界的对侧必须是同一个国家或海".into());
        }
        far = Some(o);
        // who holds the region: beside the longest sampled segment
        if near.is_none_or(|n| len > n.2) {
            let (k, e) = at(inside);
            near = Some((k, e, len));
        }
    }
    let (Some((out_kind, out_owner)), Some((in_kind, in_owner, _))) = (far, near) else { return Err("原边界过短".into()) };
    let loser = in_owner;
    let (intent, gainer) = match (out_kind, out_owner) {
        (FaceKind::Water, _) => {
            if in_kind == FaceKind::Water {
                return Err("两侧都是水域，没有可移交的领土".into());
            }
            (DrawIntent::CarveWater, None)
        }
        (FaceKind::Land, Some(g)) => {
            if in_kind == FaceKind::Land && in_owner == Some(g) {
                return Err("原边界两侧属于同一国家".into());
            }
            (DrawIntent::Take(g), Some(g))
        }
        (FaceKind::Land, None) => return Err("对侧是无主地：请改用领土画笔或“绘制陆地”".into()),
    };
    Ok(Plan { ring, old, intent, gainer, loser })
}

/// Apply a plan (the result is a new document; the input is unchanged).
pub fn apply(doc: &Document, plan: &Plan) -> Result<(Document, OverlayReport), String> {
    overlay::apply(doc, &plan.ring, plan.intent)
}

/// Where a border vertex is being dragged from: an existing interior point
/// of an edge, or a new point on segment `seg` (full-coordinate index).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DragFrom {
    Vertex(crate::topo::VertexRef),
    Segment { edge: crate::topo::EdgeId, seg: usize },
}

/// A vertex drag as a redraw line: its two neighbours on the edge with the
/// dragged point between them. Junction nodes are refused (their edges
/// border several neighbours).
pub fn drag_line(t: &crate::topo::Topology, from: DragFrom, pos: LonLat) -> Result<Vec<LonLat>, String> {
    use crate::topo::VertexRef;
    let (edge, prev) = match from {
        DragFrom::Vertex(VertexRef::Node(_)) => return Err("交汇节点不能作为领土变化拖动：请用“重画国界”，或关闭“作为领土变化”以修改所有年份的形状".into()),
        DragFrom::Vertex(VertexRef::Pt { edge, idx }) => (edge, idx),
        DragFrom::Segment { edge, seg } => (edge, seg),
    };
    let e = t.edges.get(&edge).ok_or("边不存在")?;
    let full = t.edge_full(e);
    // an existing point sits at full[idx + 1]; a new one between full[seg] and full[seg + 1]
    let next = if matches!(from, DragFrom::Vertex(_)) { prev + 2 } else { prev + 1 };
    let (a, b) = (*full.get(prev).ok_or("顶点不存在")?, *full.get(next).ok_or("顶点不存在")?);
    Ok(vec![a, pos, b])
}
