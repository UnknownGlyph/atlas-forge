//! Roads and travel ("道路与旅行"). A road network between cities: each
//! city may link to its nearest neighbours; each link is found over the
//! land on a grid (around seas and lakes, dearer up steep ground when the
//! map has terrain); the links kept are a spanning network plus the
//! shortcuts that save a long way round. Roads are written as ordinary
//! routes (one undo step, the same in every year, like every route). And
//! travel: how long a way takes on foot, on horseback, by caravan, by relay
//! horses or under sail, and the shortest way between two cities along
//! the routes drawn on the map.

use crate::doc::{Dash, Document, EntityId, EntityKind, LineFeature, LineId, RouteStyle};
use crate::geom::{self, LonLat};
use crate::measure::great_circle;
use crate::topo::{FaceKind, TResult};
use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap};

/// A way of travelling and how far it goes in a day.
#[derive(Clone, Copy, Debug)]
pub struct Mode {
    pub name: &'static str,
    pub km_per_day: f64,
    /// Over water only (a ship), not along roads.
    pub by_sea: bool,
}

/// Rough day's journeys of pre-modern travel.
pub const MODES: [Mode; 5] = [
    Mode { name: "步行", km_per_day: 30.0, by_sea: false },
    Mode { name: "车队", km_per_day: 25.0, by_sea: false },
    Mode { name: "骑马", km_per_day: 60.0, by_sea: false },
    Mode { name: "驿马（换马）", km_per_day: 200.0, by_sea: false },
    Mode { name: "帆船", km_per_day: 150.0, by_sea: true },
];

/// Days `km` take for `m`.
pub fn days(km: f64, m: &Mode) -> f64 {
    km / m.km_per_day
}

/// "约 5 小时" under a day (a day's travel is ten hours), else "约 3.5 天".
pub fn format_days(d: f64) -> String {
    if d < 1.0 {
        format!("约 {} 小时", ((d * 10.0).round() as i64).max(1))
    } else if d < 10.0 {
        format!("约 {:.1} 天", d)
    } else {
        format!("约 {:.0} 天", d)
    }
}

/// How a road network is made.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSpec {
    /// The cities to link; empty: every city with a place that exists in
    /// the selected year.
    pub cities: Vec<EntityId>,
    /// How many nearest neighbours each city may link to (candidates).
    pub neighbours: usize,
    /// A shortcut is added where the way along the network is longer than
    /// this times the shortcut's own (1 + : the spanning network only).
    pub detour: f64,
    /// Dearer up steep ground (when the map has terrain).
    pub terrain: bool,
}

impl Default for RoadSpec {
    fn default() -> Self {
        RoadSpec { cities: vec![], neighbours: 4, detour: 1.6, terrain: true }
    }
}

/// One road of a plan.
#[derive(Clone, Debug, PartialEq)]
pub struct Road {
    pub a: EntityId,
    pub b: EntityId,
    pub pts: Vec<LonLat>,
    pub km: f64,
}

/// A road network worked out but not yet written.
#[derive(Clone, Debug, Default)]
pub struct RoadPlan {
    pub roads: Vec<Road>,
    pub cities: usize,
    /// Cities no road reaches (alone on an island, or in the sea).
    pub alone: Vec<EntityId>,
}

/// The look of a generated road.
pub fn road_style() -> RouteStyle {
    RouteStyle { color: [125, 85, 45, 255], width: 1.5, dash: Dash::Solid, arrow: false, ..RouteStyle::default() }
}

/// The land on a grid over the cities, longitudes unwrapped around theirs.
struct Grid {
    x0: f64,
    y1: f64,
    cs: f64,
    w: usize,
    h: usize,
    land: Vec<bool>,
    height: Option<Vec<f32>>,
}

impl Grid {
    fn at(&self, p: LonLat) -> (usize, usize) {
        let i = ((p.lon - self.x0) / self.cs).floor().clamp(0.0, self.w as f64 - 1.0) as usize;
        let j = ((self.y1 - p.lat) / self.cs).floor().clamp(0.0, self.h as f64 - 1.0) as usize;
        (i, j)
    }
    /// Whether a line stays on land (checked every half cell, its two
    /// ends, the cities, apart).
    fn dry(&self, line: &[LonLat]) -> bool {
        let n = line.len();
        line.windows(2).enumerate().all(|(k, w)| {
            let steps = ((w[1].lon - w[0].lon).abs().max((w[1].lat - w[0].lat).abs()) / (self.cs * 0.5)).ceil().max(1.0) as usize;
            (0..=steps).all(|s| {
                if (k == 0 && s == 0) || (k == n - 2 && s == steps) {
                    return true;
                }
                let t = s as f64 / steps as f64;
                let p = LonLat::new(w[0].lon + (w[1].lon - w[0].lon) * t, w[0].lat + (w[1].lat - w[0].lat) * t);
                let (i, j) = self.at(p);
                self.land[j * self.w + i]
            })
        })
    }

    fn center(&self, i: usize, j: usize) -> LonLat {
        LonLat::new(self.x0 + (i as f64 + 0.5) * self.cs, self.y1 - (j as f64 + 0.5) * self.cs)
    }
}

impl Document {
    /// Cities a road plan links (with their places, longitudes unwrapped
    /// around the first).
    fn road_cities(&self, spec: &RoadSpec) -> Vec<(EntityId, LonLat)> {
        let ids: Vec<EntityId> = if spec.cities.is_empty() {
            self.entities.iter().filter(|(_, x)| x.kind == EntityKind::City && x.exists_in(self.year)).map(|(e, _)| *e).collect()
        } else {
            spec.cities.clone()
        };
        let mut out: Vec<(EntityId, LonLat)> = ids.into_iter().filter_map(|e| self.entities.get(&e).and_then(|x| x.pos).map(|p| (e, p))).collect();
        if let Some(&(_, first)) = out.first() {
            for (_, p) in out.iter_mut() {
                p.lon = first.lon + geom::wrap_lon(p.lon - first.lon);
            }
        }
        out
    }

    fn road_grid(&self, cities: &[(EntityId, LonLat)], terrain: bool) -> Grid {
        let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
        for (_, p) in cities {
            lo = [lo[0].min(p.lon), lo[1].min(p.lat)];
            hi = [hi[0].max(p.lon), hi[1].max(p.lat)];
        }
        // room to go round a lake or along a coast between the cities
        let margin = ((hi[0] - lo[0]).max(hi[1] - lo[1]) * 0.4).clamp(2.0, 20.0);
        let (x0, x1) = (lo[0] - margin, hi[0] + margin);
        let (y0, y1) = ((lo[1] - margin).max(-89.0), (hi[1] + margin).min(89.0));
        // about 160 000 cells, none smaller than ~2 km
        let cs = (((x1 - x0) * (y1 - y0)) / 160_000.0).sqrt().max(0.02);
        let (w, h) = (((x1 - x0) / cs).ceil() as usize, ((y1 - y0) / cs).ceil() as usize);
        // land by scanlines: each face's rings filled even-odd (its holes,
        // the lakes and islands inside it, stay out) onto the background's kind
        let bg_land = self.background_kind() == FaceKind::Land;
        let mut g = Grid { x0, y1, cs, w, h, land: vec![bg_land; w * h], height: None };
        let want = if bg_land { FaceKind::Water } else { FaceKind::Land };
        for (f, face) in self.topo.faces.iter() {
            if *f == crate::topo::BACKGROUND || face.kind != want {
                continue;
            }
            let (outer, holes) = self.topo.face_planar(*f);
            if outer.len() < 3 {
                continue;
            }
            let fb = geom::bbox(&outer);
            for shift in [-360.0, 0.0, 360.0] {
                if fb[2] + shift < x0 || fb[0] + shift > x1 || fb[3] < y0 || fb[1] > y1 {
                    continue;
                }
                let mut rows: BTreeMap<usize, Vec<f64>> = BTreeMap::new();
                for ring in std::iter::once(&outer).chain(&holes) {
                    for k in 0..ring.len() {
                        let (p, q) = (ring[k], ring[(k + 1) % ring.len()]);
                        let (ya, yb) = (p[1].min(q[1]), p[1].max(q[1]));
                        // rows whose centre line lies in [ya, yb)
                        let j0 = ((y1 - yb) / cs - 0.5).ceil().max(0.0) as usize;
                        let j1 = ((y1 - ya) / cs - 0.5).floor().min(h as f64 - 1.0);
                        if j1 < 0.0 {
                            continue;
                        }
                        for j in j0..=j1 as usize {
                            let yc = y1 - (j as f64 + 0.5) * cs;
                            if yc < ya || yc >= yb {
                                continue;
                            }
                            let t = (yc - p[1]) / (q[1] - p[1]);
                            rows.entry(j).or_default().push(p[0] + t * (q[0] - p[0]) + shift);
                        }
                    }
                }
                for (j, mut xs) in rows {
                    xs.sort_by(|a, b| a.total_cmp(b));
                    for pair in xs.as_chunks::<2>().0 {
                        let i0 = ((pair[0] - x0) / cs - 0.5).ceil().max(0.0) as usize;
                        let i1 = ((pair[1] - x0) / cs - 0.5).ceil().min(w as f64);
                        for i in i0..(i1.max(0.0) as usize) {
                            g.land[j * w + i] = !bg_land;
                        }
                    }
                }
            }
        }
        if terrain {
            if let Some(t) = &self.terrain {
                g.height = Some((0..w * h).map(|k| t.at(geom::wrap_lon(g.center(k % w, k / w).lon), g.center(k % w, k / w).lat) as f32).collect());
            }
        }
        g
    }

    /// Work out a road network (see the module notes). Nothing changes.
    pub fn road_plan(&self, spec: &RoadSpec) -> TResult<RoadPlan> {
        let cities = self.road_cities(spec);
        if cities.len() < 2 {
            return Err("至少需要两座有位置的城市".into());
        }
        let g = self.road_grid(&cities, spec.terrain);
        let r = self.radius();
        let km_per_deg = r * std::f64::consts::PI / 180.0;
        // each city's cell, moved onto land nearby when it sits on the coast line
        let cell: Vec<Option<usize>> = cities
            .iter()
            .map(|(_, p)| {
                let (i, j) = g.at(*p);
                let mut best: Option<(i64, usize)> = None;
                for dj in -3i64..=3 {
                    for di in -3i64..=3 {
                        let (ii, jj) = (i as i64 + di, j as i64 + dj);
                        if ii < 0 || jj < 0 || ii >= g.w as i64 || jj >= g.h as i64 {
                            continue;
                        }
                        let k = jj as usize * g.w + ii as usize;
                        if g.land[k] && best.is_none_or(|b| di * di + dj * dj < b.0) {
                            best = Some((di * di + dj * dj, k));
                        }
                    }
                }
                best.map(|b| b.1)
            })
            .collect();
        // candidate links: each city to its nearest neighbours
        let n = cities.len();
        let mut cand: Vec<(usize, usize)> = Vec::new();
        for a in 0..n {
            let mut near: Vec<(f64, usize)> = (0..n).filter(|b| *b != a).map(|b| (great_circle(cities[a].1, cities[b].1, r), b)).collect();
            near.sort_by(|x, y| x.0.total_cmp(&y.0));
            for (_, b) in near.into_iter().take(spec.neighbours.max(1)) {
                let key = (a.min(b), a.max(b));
                if !cand.contains(&key) {
                    cand.push(key);
                }
            }
        }
        // each candidate's way over the land
        let mut found: Vec<(usize, usize, f64, Vec<usize>)> = Vec::new();
        for (a, b) in cand {
            let (Some(ca), Some(cb)) = (cell[a], cell[b]) else { continue };
            if let Some((cost, path)) = astar(&g, ca, cb, km_per_deg) {
                found.push((a, b, cost, path));
            }
        }
        found.sort_by(|x, y| x.2.total_cmp(&y.2).then((x.0, x.1).cmp(&(y.0, y.1))));
        // the spanning network (Kruskal), then the shortcuts
        let mut root: Vec<usize> = (0..n).collect();
        fn find(root: &mut [usize], x: usize) -> usize {
            let mut x = x;
            while root[x] != x {
                root[x] = root[root[x]];
                x = root[x];
            }
            x
        }
        let mut keep = vec![false; found.len()];
        for (k, (a, b, _, _)) in found.iter().enumerate() {
            let (ra, rb) = (find(&mut root, *a), find(&mut root, *b));
            if ra != rb {
                root[ra] = rb;
                keep[k] = true;
            }
        }
        for k in 0..found.len() {
            if keep[k] {
                continue;
            }
            let (a, b, cost, _) = &found[k];
            let edges: Vec<(usize, usize, f64)> = found.iter().zip(&keep).filter(|(_, kept)| **kept).map(|(f, _)| (f.0, f.1, f.2)).collect();
            if network_cost(n, &edges, *a, *b).is_none_or(|net| net > spec.detour * cost) {
                keep[k] = true;
            }
        }
        let mut plan = RoadPlan { cities: n, ..RoadPlan::default() };
        let mut linked = vec![false; n];
        for ((a, b, _, path), kept) in found.into_iter().zip(keep) {
            if !kept {
                continue;
            }
            linked[a] = true;
            linked[b] = true;
            let mut pts: Vec<LonLat> = path.iter().map(|k| g.center(k % g.w, k / g.w)).collect();
            pts[0] = cities[a].1;
            *pts.last_mut().unwrap() = cities[b].1;
            // simplified and smoothed, unless that cuts into the water (round
            // a lake's corner): then simplified only, or the cells as found
            let planar: Vec<[f64; 2]> = pts.iter().map(|p| [p.lon, p.lat]).collect();
            let kept: Vec<LonLat> = crate::trace::simplify(&planar, g.cs * 0.6).into_iter().map(|i| pts[i]).collect();
            let smooth = if kept.len() >= 3 { crate::worldgen::smooth_line(&kept, 2) } else { kept.clone() };
            let line = [smooth, kept, pts].into_iter().find(|l| g.dry(l)).unwrap_or_default();
            let pts: Vec<LonLat> = line.into_iter().map(|p| LonLat::new(geom::wrap_lon(p.lon), p.lat)).collect();
            let km = self.path_length_km(&pts);
            plan.roads.push(Road { a: cities[a].0, b: cities[b].0, pts, km });
        }
        plan.alone = (0..n).filter(|k| !linked[*k]).map(|k| cities[k].0).collect();
        Ok(plan)
    }

    /// Write a plan's roads as routes named after their two cities (one
    /// undo step for the caller). Returns the new routes.
    pub fn apply_roads(&mut self, plan: &RoadPlan) -> TResult<Vec<LineId>> {
        let mut out = Vec::new();
        for road in &plan.roads {
            let name = |e: EntityId| self.entities.get(&e).map(|x| x.name.clone()).unwrap_or_default();
            let line = LineFeature { name: format!("{}—{}", name(road.a), name(road.b)), pts: road.pts.clone(), route: Some(road_style()), legend: "道路".into(), show_name: false, hidden: false };
            let id = LineId(self.topo.alloc());
            self.rivers.insert(id, line);
            out.push(id);
        }
        Ok(out)
    }

    /// The shortest way from city `a` to city `b` along the routes drawn on
    /// the map (a route joins two cities when its ends are at their places,
    /// as roads made here and routes snapped to cities are): the routes in
    /// order and the length in km. None when no such way exists.
    pub fn travel_way(&self, a: EntityId, b: EntityId) -> Option<(Vec<LineId>, f64)> {
        let pos = |e: EntityId| self.entities.get(&e).and_then(|x| x.pos);
        let (pa, pb) = (pos(a)?, pos(b)?);
        // cities by place; routes whose ends are at two of them
        let places: Vec<(EntityId, LonLat)> = self.entities.iter().filter(|(_, x)| x.kind == EntityKind::City).filter_map(|(e, x)| x.pos.map(|p| (*e, p))).collect();
        let near = |p: LonLat| places.iter().find(|(_, q)| geom::geo_dist_deg(p, *q) < 1e-6).map(|(e, _)| *e);
        let mut adj: BTreeMap<EntityId, Vec<(EntityId, LineId, f64)>> = BTreeMap::new();
        for (id, l) in self.rivers.iter() {
            if l.route.as_ref().is_none_or(|s| s.broad) || l.hidden || l.pts.len() < 2 {
                continue;
            }
            let (Some(x), Some(y)) = (near(l.pts[0]), near(*l.pts.last().unwrap())) else { continue };
            let km = self.path_length_km(&l.pts);
            adj.entry(x).or_default().push((y, *id, km));
            adj.entry(y).or_default().push((x, *id, km));
        }
        let (Some(a), Some(b)) = (near(pa), near(pb)) else { return None };
        let mut best: BTreeMap<EntityId, (f64, Option<(EntityId, LineId)>)> = BTreeMap::new();
        let mut heap = BinaryHeap::new();
        best.insert(a, (0.0, None));
        heap.push(Reverse((Cost(0.0), a)));
        while let Some(Reverse((Cost(d), x))) = heap.pop() {
            if x == b {
                break;
            }
            if best.get(&x).is_some_and(|v| d > v.0) {
                continue;
            }
            for (y, line, km) in adj.get(&x).into_iter().flatten() {
                let nd = d + km;
                if best.get(y).is_none_or(|v| nd < v.0) {
                    best.insert(*y, (nd, Some((x, *line))));
                    heap.push(Reverse((Cost(nd), *y)));
                }
            }
        }
        let total = best.get(&b)?.0;
        let mut lines = Vec::new();
        let mut at = b;
        while let Some((_, Some((prev, line)))) = best.get(&at) {
            lines.push(*line);
            at = *prev;
        }
        lines.reverse();
        Some((lines, total))
    }
}

#[derive(Clone, Copy, PartialEq)]
struct Cost(f64);
impl Eq for Cost {}
impl PartialOrd for Cost {
    fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(o))
    }
}
impl Ord for Cost {
    fn cmp(&self, o: &Self) -> std::cmp::Ordering {
        self.0.total_cmp(&o.0)
    }
}

/// The cheapest way over the land from cell `from` to cell `to` (A*, eight
/// neighbours): its cost in km (steep ground counted longer) and its cells.
fn astar(g: &Grid, from: usize, to: usize, km_per_deg: f64) -> Option<(f64, Vec<usize>)> {
    let pos = |k: usize| ((k % g.w) as f64, (k / g.w) as f64);
    let lat = |k: usize| g.y1 - ((k / g.w) as f64 + 0.5) * g.cs;
    let step_km = |a: usize, b: usize| {
        let (pa, pb) = (pos(a), pos(b));
        let c = ((lat(a) + lat(b)) * 0.5).to_radians().cos();
        let (dx, dy) = ((pb.0 - pa.0) * c, pb.1 - pa.1);
        (dx * dx + dy * dy).sqrt() * g.cs * km_per_deg
    };
    // a cell beside the water costs a little more: roads keep off the shore
    let shore = |b: usize| {
        let (i, j) = (b % g.w, b / g.w);
        (i > 0 && !g.land[b - 1]) || (i + 1 < g.w && !g.land[b + 1]) || (j > 0 && !g.land[b - g.w]) || (j + 1 < g.h && !g.land[b + g.w])
    };
    let cost = |a: usize, b: usize| {
        let km = step_km(a, b);
        let climb = g.height.as_ref().map_or(0.0, |h| ((h[b] - h[a]).abs() as f64 / (km * 1000.0).max(1.0)).min(1.0));
        km * (1.0 + 12.0 * climb) * if shore(b) { 1.3 } else { 1.0 }
    };
    // never more than the cost (the east-west scale of the grid's most
    // poleward row): estimating with the ends' own latitudes overshot a
    // way through higher latitudes, and cells were opened again and again
    let cmin = g.y1.abs().max((g.y1 - g.h as f64 * g.cs).abs()).min(89.9).to_radians().cos();
    let guess = |a: usize| {
        let (pa, pb) = (pos(a), pos(to));
        let (dx, dy) = ((pb.0 - pa.0) * cmin, pb.1 - pa.1);
        (dx * dx + dy * dy).sqrt() * g.cs * km_per_deg
    };
    let mut best: Vec<f64> = vec![f64::INFINITY; g.w * g.h];
    let mut came: Vec<usize> = vec![usize::MAX; g.w * g.h];
    let mut heap = BinaryHeap::new();
    best[from] = 0.0;
    heap.push(Reverse((Cost(guess(from)), Cost(0.0), from)));
    while let Some(Reverse((_, Cost(d), k))) = heap.pop() {
        // an entry left from before a cheaper way was found
        if d > best[k] {
            continue;
        }
        if k == to {
            let mut path = vec![to];
            let mut at = to;
            while at != from {
                at = came[at];
                path.push(at);
            }
            path.reverse();
            return Some((best[to], path));
        }
        let (i, j) = (k % g.w, k / g.w);
        for dj in -1i64..=1 {
            for di in -1i64..=1 {
                if di == 0 && dj == 0 {
                    continue;
                }
                let (ii, jj) = (i as i64 + di, j as i64 + dj);
                if ii < 0 || jj < 0 || ii >= g.w as i64 || jj >= g.h as i64 {
                    continue;
                }
                let m = jj as usize * g.w + ii as usize;
                if !g.land[m] {
                    continue;
                }
                let nd = best[k] + cost(k, m);
                if nd < best[m] {
                    best[m] = nd;
                    came[m] = k;
                    heap.push(Reverse((Cost(nd + guess(m)), Cost(nd), m)));
                }
            }
        }
    }
    None
}

/// The cheapest way from `a` to `b` over the kept links (Dijkstra).
fn network_cost(n: usize, edges: &[(usize, usize, f64)], a: usize, b: usize) -> Option<f64> {
    let mut adj: Vec<Vec<(usize, f64)>> = vec![vec![]; n];
    for (x, y, c) in edges {
        adj[*x].push((*y, *c));
        adj[*y].push((*x, *c));
    }
    let mut best = vec![f64::INFINITY; n];
    let mut heap = BinaryHeap::new();
    best[a] = 0.0;
    heap.push(Reverse((Cost(0.0), a)));
    while let Some(Reverse((Cost(d), x))) = heap.pop() {
        if x == b {
            return Some(d);
        }
        if d > best[x] {
            continue;
        }
        for (y, c) in &adj[x] {
            if d + c < best[*y] {
                best[*y] = d + c;
                heap.push(Reverse((Cost(d + c), *y)));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_and_their_words() {
        assert_eq!(format_days(days(15.0, &MODES[0])), "约 5 小时");
        assert_eq!(format_days(days(90.0, &MODES[0])), "约 3.0 天");
        assert_eq!(format_days(days(3000.0, &MODES[0])), "约 100 天");
    }
}
