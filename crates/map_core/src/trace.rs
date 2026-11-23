//! Tracing along existing boundaries: the shortest way between two points
//! lying on edges, following the shared edges (and their exact vertices)
//! instead of a straight line. Used by the drawing tools so a new shape can
//! follow a coast or border it touches.

use crate::geom::{self, LonLat};
use crate::topo::{EdgeId, FaceKind, NodeId, Topology};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BinaryHeap, HashMap};

type GridPoint = (i64, i64);
type GridRing = Vec<GridPoint>;

/// Trace all labeled regions in a row-major grid. Corners have y down;
/// in map coordinates each ring keeps its region to the left. Straight
/// runs keep every corner so neighboring regions share the same vertices.
pub(crate) fn grid_regions(
    width: usize,
    height: usize,
    region_count: usize,
    label_at: impl Fn(usize) -> Option<usize>,
) -> Vec<Vec<GridRing>> {
    let label_at_cell = |x: i64, y: i64| {
        if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
            label_at(y as usize * width + x as usize).filter(|&label| label < region_count)
        } else {
            None
        }
    };
    let mut boundaries = vec![BTreeMap::<GridPoint, Vec<GridPoint>>::new(); region_count];
    for y in 0..height as i64 {
        for x in 0..width as i64 {
            let Some(label) = label_at_cell(x, y) else { continue };
            let boundary = &mut boundaries[label];
            for (neighbor, from, to) in [
                ((x, y + 1), (x, y + 1), (x + 1, y + 1)),
                ((x + 1, y), (x + 1, y + 1), (x + 1, y)),
                ((x, y - 1), (x + 1, y), (x, y)),
                ((x - 1, y), (x, y), (x, y + 1)),
            ] {
                if label_at_cell(neighbor.0, neighbor.1) != Some(label) {
                    boundary.entry(from).or_default().push(to);
                }
            }
        }
    }
    boundaries.into_iter().map(|mut next| {
        let mut rings = Vec::new();
        while let Some((&start, _)) = next.first_key_value() {
            let mut ring = vec![start];
            let mut at = start;
            let mut direction = (0, 0);
            loop {
                let std::collections::btree_map::Entry::Occupied(mut entry) = next.entry(at) else {
                    unreachable!("grid boundaries form closed rings");
                };
                let outs = entry.get_mut();
                // Where two regions of the same label touch at a corner,
                // take the left turn first (negative cross product with y down).
                let pick = (0..outs.len()).max_by_key(|&i| {
                    let (x, y) = outs[i];
                    direction.1 * (x - at.0) - direction.0 * (y - at.1)
                }).unwrap();
                let to = outs.swap_remove(pick);
                if outs.is_empty() {
                    entry.remove();
                }
                direction = (to.0 - at.0, to.1 - at.1);
                at = to;
                if at == start {
                    break;
                }
                ring.push(at);
            }
            if ring.len() >= 3 {
                rings.push(ring);
            }
        }
        rings
    }).collect()
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Key {
    Node(NodeId),
    Start,
    End,
}

#[derive(PartialEq)]
struct Item(f64, Key);
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Item {
    fn cmp(&self, o: &Self) -> Ordering {
        o.0.total_cmp(&self.0)
    }
}

fn length(pts: &[LonLat]) -> f64 {
    pts.windows(2).map(|w| geom::geo_dist_deg(w[0], w[1])).sum()
}

/// Whether an edge separates land from water (a coast).
pub fn is_coast(t: &Topology, e: EdgeId) -> bool {
    let Some(e) = t.edges.get(&e) else { return false };
    let k = |f| t.faces.get(&f).map_or(FaceKind::Water, |x| x.kind);
    k(e.left) != k(e.right)
}

/// A point snapped onto the boundary network: the edge, the segment index
/// in its full polyline, and the point.
pub fn locate(t: &Topology, p: LonLat, tol: f64) -> Option<(EdgeId, usize, LonLat)> {
    t.nearest_on_edge(p, tol)
}

/// The boundary path from `a` to `b` (both within `tol` of some edge),
/// inclusive of both snapped endpoints, or None when they are not connected
/// through the allowed edges. With `coast_only`, only land/water edges are
/// followed (tracing a coastline should not cut through inland borders).
pub fn boundary_path(t: &Topology, a: LonLat, b: LonLat, tol: f64, coast_only: bool) -> Option<Vec<LonLat>> {
    let (ea, sa, qa) = locate(t, a, tol)?;
    let (eb, sb, qb) = locate(t, b, tol)?;
    let allowed = |e: EdgeId| !coast_only || is_coast(t, e);
    if !allowed(ea) || !allowed(eb) {
        return None;
    }
    // adjacency: (to, polyline from -> to)
    let mut adj: HashMap<Key, Vec<(Key, Vec<LonLat>)>> = HashMap::new();
    let mut link = |from: Key, to: Key, pts: Vec<LonLat>| {
        let mut rev = pts.clone();
        rev.reverse();
        adj.entry(from).or_default().push((to, pts));
        adj.entry(to).or_default().push((from, rev));
    };
    for (id, e) in t.edges.iter() {
        if !allowed(*id) {
            continue;
        }
        link(Key::Node(e.a), Key::Node(e.b), t.edge_full(e));
    }
    // the two snapped points split their edges
    let split = |e: EdgeId, s: usize, q: LonLat| {
        let full = t.edge_full(&t.edges[&e]);
        let mut to_a: Vec<LonLat> = full[..=s].to_vec();
        to_a.push(q);
        to_a.reverse();
        let mut to_b = vec![q];
        to_b.extend_from_slice(&full[s + 1..]);
        (t.edges[&e].a, t.edges[&e].b, to_a, to_b)
    };
    for (key, e, s, q) in [(Key::Start, ea, sa, qa), (Key::End, eb, sb, qb)] {
        let (na, nb, to_a, to_b) = split(e, s, q);
        link(key, Key::Node(na), to_a);
        link(key, Key::Node(nb), to_b);
    }
    // both on the same edge: the piece between them
    if ea == eb {
        let full = t.edge_full(&t.edges[&ea]);
        let (first, last, s0, s1, fwd) = if (sa, geom::geo_dist_deg(full[sa], qa)) <= (sb, geom::geo_dist_deg(full[sb], qb)) { (qa, qb, sa, sb, true) } else { (qb, qa, sb, sa, false) };
        let mut piece = vec![first];
        piece.extend_from_slice(&full[s0 + 1..=s1]);
        piece.push(last);
        if !fwd {
            piece.reverse();
        }
        link(Key::Start, Key::End, piece);
    }
    // Dijkstra
    let mut dist: HashMap<Key, f64> = HashMap::new();
    let mut prev: HashMap<Key, (Key, usize)> = HashMap::new();
    let mut heap = BinaryHeap::new();
    dist.insert(Key::Start, 0.0);
    heap.push(Item(0.0, Key::Start));
    while let Some(Item(d, k)) = heap.pop() {
        if k == Key::End {
            break;
        }
        if d > dist.get(&k).copied().unwrap_or(f64::INFINITY) {
            continue;
        }
        for (i, (to, pts)) in adj.get(&k).into_iter().flatten().enumerate() {
            let nd = d + length(pts);
            if nd < dist.get(to).copied().unwrap_or(f64::INFINITY) {
                dist.insert(*to, nd);
                prev.insert(*to, (k, i));
                heap.push(Item(nd, *to));
            }
        }
    }
    if !dist.contains_key(&Key::End) {
        return None;
    }
    let mut legs: Vec<Vec<LonLat>> = Vec::new();
    let mut k = Key::End;
    while k != Key::Start {
        let (p, i) = prev[&k];
        legs.push(adj[&p][i].1.clone());
        k = p;
    }
    legs.reverse();
    let mut out: Vec<LonLat> = Vec::new();
    for leg in legs {
        for q in leg {
            if out.last().is_none_or(|l| geom::geo_dist_deg(*l, q) > 1e-12) {
                out.push(q);
            }
        }
    }
    Some(out)
}

/// Simplify a polyline (Douglas–Peucker) with tolerance `eps`, measured in
/// the same units as `pts` (the drawing tools pass screen pixels).
pub fn simplify(pts: &[[f64; 2]], eps: f64) -> Vec<usize> {
    if pts.len() <= 2 {
        return (0..pts.len()).collect();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0usize, pts.len() - 1)];
    while let Some((i, j)) = stack.pop() {
        let (a, b) = (pts[i], pts[j]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        let mut best = (0.0, 0);
        for k in i + 1..j {
            let p = pts[k];
            let d = if len < 1e-12 { ((p[0] - a[0]).powi(2) + (p[1] - a[1]).powi(2)).sqrt() } else { ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx).abs() / len };
            if d > best.0 {
                best = (d, k);
            }
        }
        if best.0 > eps {
            keep[best.1] = true;
            stack.push((i, best.1));
            stack.push((best.1, j));
        }
    }
    (0..pts.len()).filter(|i| keep[*i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_rings_preserve_diagonal_components_and_shared_holes() {
        let diagonal = [Some(0), None, None, Some(0)];
        let regions = grid_regions(2, 2, 1, |index| diagonal[index]);
        assert_eq!(regions[0], vec![
            vec![(0, 0), (0, 1), (1, 1), (1, 0)],
            vec![(1, 1), (1, 2), (2, 2), (2, 1)],
        ]);

        let regions = grid_regions(3, 3, 2, |index| Some(usize::from(index == 4)));
        assert_eq!(regions[0][0].len(), 12);
        assert_eq!(regions[0][1], vec![(1, 1), (2, 1), (2, 2), (1, 2)]);
        assert_eq!(regions[1][0], vec![(1, 1), (1, 2), (2, 2), (2, 1)]);
    }
    use crate::doc::Document;

    fn sq(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<LonLat> {
        vec![LonLat::new(x0, y0), LonLat::new(x1, y0), LonLat::new(x1, y1), LonLat::new(x0, y1)]
    }

    /// A 20×10 block split at x = 10 into two countries.
    fn world() -> Document {
        let mut d = Document::default();
        d.add_ring_face(&sq(0.0, 0.0, 20.0, 10.0), FaceKind::Land).unwrap();
        d.split_face(&[LonLat::new(10.0, 0.0), LonLat::new(10.0, 10.0)]).unwrap();
        d
    }

    #[test]
    fn follows_the_coast_not_the_inland_border() {
        let d = world();
        let t = &d.topo;
        // from the bottom coast at x = 5 to the top coast at x = 5
        let (a, b) = (LonLat::new(5.0, 0.0), LonLat::new(5.0, 10.0));
        let any = boundary_path(t, a, b, 0.1, false).unwrap();
        let coast = boundary_path(t, a, b, 0.1, true).unwrap();
        // both are boundary paths starting and ending at the snapped points
        for p in [&any, &coast] {
            assert!(geom::geo_dist_deg(p[0], a) < 1e-9 && geom::geo_dist_deg(*p.last().unwrap(), b) < 1e-9);
        }
        // the shortest overall goes round the west coast (5 + 10 + 5 = 20)
        assert!((length(&any) - 20.0).abs() < 1e-6, "{}", length(&any));
        assert!(any.iter().all(|p| p.lon <= 5.0 + 1e-9));
        // coast only: same here, never touching the inland border at x = 10
        assert!(coast.iter().all(|p| (p.lon - 10.0).abs() > 1e-9 || p.lat == 0.0 || p.lat == 10.0));
        // to the east half's top coast: the inland border is a shortcut that
        // coast-only tracing must not take
        let c = LonLat::new(15.0, 10.0);
        let via_border = boundary_path(t, a, c, 0.1, false).unwrap();
        let via_coast = boundary_path(t, a, c, 0.1, true).unwrap();
        assert!(length(&via_border) < length(&via_coast) + 1e-9);
        assert!(via_coast.iter().all(|p| !(p.lon == 10.0 && p.lat > 0.0 && p.lat < 10.0)));
    }

    #[test]
    fn same_edge_and_unreachable() {
        let d = world();
        let t = &d.topo;
        // two points on the same bottom edge: just the piece between them
        let p = boundary_path(t, LonLat::new(2.0, 0.0), LonLat::new(7.0, 0.0), 0.1, false).unwrap();
        assert!((length(&p) - 5.0).abs() < 1e-6);
        let p = boundary_path(t, LonLat::new(7.0, 0.0), LonLat::new(2.0, 0.0), 0.1, false).unwrap();
        assert!((length(&p) - 5.0).abs() < 1e-6 && p[0].lon > p.last().unwrap().lon);
        // far from any edge
        assert!(boundary_path(t, LonLat::new(50.0, 50.0), LonLat::new(2.0, 0.0), 0.1, false).is_none());
        // an island elsewhere is not connected
        let mut d2 = world();
        d2.add_ring_face(&sq(40.0, 0.0, 45.0, 5.0), FaceKind::Land).unwrap();
        assert!(boundary_path(&d2.topo, LonLat::new(2.0, 0.0), LonLat::new(42.0, 0.0), 0.1, false).is_none());
    }

    #[test]
    fn simplify_keeps_corners() {
        let pts: Vec<[f64; 2]> = (0..=10).map(|i| [i as f64, 0.0]).chain((1..=10).map(|i| [10.0, i as f64])).collect();
        assert_eq!(simplify(&pts, 0.5), vec![0, 10, 20]);
        let wiggly: Vec<[f64; 2]> = (0..=20).map(|i| [i as f64, if i % 2 == 0 { 0.0 } else { 0.3 }]).collect();
        assert_eq!(simplify(&wiggly, 0.5).len(), 2);
        assert!(simplify(&wiggly, 0.1).len() > 2);
    }
}
