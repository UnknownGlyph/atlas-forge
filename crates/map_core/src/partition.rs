//! Automatic partition ("自动划分"): a country's land cut into N connected
//! units — new countries beside it, or its provinces (units under it in
//! the hierarchy). The land is laid on a fine grid; seeds are spread over
//! it and grow together, each step costing more across rivers and steep
//! ground (when the map has them), with the units' sizes evened out by
//! weighting their growth; the units' cells are traced into polygons,
//! cut exactly to the country's outline, and drawn as ordinary edits
//! ("take" in the selected year), after which the new inner borders are
//! smoothed. A plan can be looked at before it is applied.

use crate::doc::{Document, EntityId, EntityKind, PlanarPolygon};
use crate::geom::{self, LonLat, P2};
use crate::names::{self, NameKind, NameStyle};
use crate::overlay::{self, DrawIntent};
use crate::sample::Rng;
use crate::topo::{FaceId, TResult};
use geo::BooleanOps;
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct PartitionSpec {
    /// How many units (2 – 60).
    pub units: usize,
    pub seed: u64,
    /// Units of about the same size.
    pub balance: bool,
    /// Borders drawn along rivers and ridges where there are some.
    pub follow: bool,
    /// Units under the country (provinces) rather than new countries.
    pub provinces: bool,
    pub style: NameStyle,
}

impl Default for PartitionSpec {
    fn default() -> Self {
        PartitionSpec { units: 6, seed: 1, balance: true, follow: true, provinces: false, style: NameStyle::Huaxia }
    }
}

/// One piece of a unit: its outer ring and holes (lon/lat). A hole is land
/// of another unit, a lake, or another country's enclave: drawing the piece
/// leaves it as it is.
#[derive(Clone, Debug, PartialEq)]
pub struct Part {
    pub outer: Vec<LonLat>,
    pub holes: Vec<Vec<LonLat>>,
}

/// A partition worked out but not yet applied.
#[derive(Clone, Debug)]
pub struct Plan {
    pub target: EntityId,
    /// Each unit's pieces.
    pub units: Vec<Vec<Part>>,
    /// Each unit's area (km²).
    pub areas: Vec<f64>,
}

#[derive(PartialEq)]
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

/// A ring cut at the corners it passes twice into rings that do not touch
/// themselves (a figure eight → its two loops).
fn simple_loops(ring: &[LonLat]) -> Vec<Vec<LonLat>> {
    let key = |p: &LonLat| (p.lon.to_bits(), p.lat.to_bits());
    let mut out = Vec::new();
    let mut path: Vec<LonLat> = Vec::new();
    let mut at: std::collections::HashMap<(u64, u64), usize> = std::collections::HashMap::new();
    for p in ring {
        match at.get(&key(p)).copied() {
            Some(i) => {
                // the loop from the earlier visit back to here
                let lp: Vec<LonLat> = path.drain(i..).collect();
                for q in &lp[1..] {
                    at.remove(&key(q));
                }
                path.push(*p);
                if lp.len() >= 3 {
                    out.push(lp);
                }
            }
            None => {
                at.insert(key(p), path.len());
                path.push(*p);
            }
        }
    }
    if path.len() >= 3 {
        out.push(path);
    }
    out
}

fn to_geo(rings: &[Vec<P2>]) -> geo::MultiPolygon<f64> {
    // outer rings counter-clockwise, holes clockwise: each hole to the outer ring around it
    let ls = |r: &Vec<P2>| geo::LineString::from(r.iter().map(|p| (p[0], p[1])).collect::<Vec<_>>());
    let (outer, holes): (Vec<&Vec<P2>>, Vec<&Vec<P2>>) = rings.iter().partition(|r| geom::signed_area(r) > 0.0);
    // (outer ring, its holes, the ring's points)
    type Poly<'a> = (geo::LineString<f64>, Vec<geo::LineString<f64>>, &'a Vec<P2>);
    let mut polys: Vec<Poly> = outer.into_iter().map(|r| (ls(r), vec![], r)).collect();
    for h in holes {
        let p = h[0];
        if let Some(k) = (0..polys.len()).filter(|k| geom::point_in_poly([p[0] + 1e-9, p[1] + 1e-9], polys[*k].2)).min_by(|a, b| geom::signed_area(polys[*a].2).total_cmp(&geom::signed_area(polys[*b].2))) {
            polys[k].1.push(ls(h));
        }
    }
    geo::MultiPolygon::new(polys.into_iter().map(|(o, hs, _)| geo::Polygon::new(o, hs)).collect())
}

impl Document {
    /// Start a subordinate unit in the active exclusive layer.
    pub fn add_subdivision(&mut self, parent: EntityId, name: &str) -> TResult<EntityId> {
        let source = self.entities.get(&parent)
            .filter(|e| e.kind == EntityKind::Country && e.layer == self.active_layer && e.exists_in(self.year))
            .ok_or("请在当前分区层和年份选择有效的上级单位")?;
        if self.active_is_shares() {
            return Err("请在独占分区层中创建省份".into());
        }
        let name = name.trim();
        if name.is_empty() {
            return Err("省份名称不能为空".into());
        }
        let lifespan = (self.year.or(source.founded), source.ended);
        let color = source.color;
        let id = self.add_entity(EntityKind::Country, name);
        self.entities.get_mut(&id).unwrap().color = color;
        self.set_lifespan(id, lifespan.0, lifespan.1)?;
        self.record_parent(id, Some(parent))?;
        Ok(id)
    }

    /// Create a unit and give it a whole face, atomically.
    pub fn subdivision_on_face(&mut self, parent: EntityId, name: &str, face: crate::FaceId) -> TResult<EntityId> {
        if self.owner.get(&face) != Some(&parent) {
            return Err("请选择由上级单位直接管辖的陆地".into());
        }
        let mut draft = self.clone();
        let id = draft.add_subdivision(parent, name)?;
        draft.assign(face, Some(id))?;
        *self = draft;
        Ok(id)
    }

    /// Work out how a country's land would be cut into units.
    pub fn partition_plan(&self, e: EntityId, spec: &PartitionSpec) -> TResult<Plan> {
        if self.active_is_shares()
            || self.entities.get(&e).is_none_or(|x| x.kind != EntityKind::Country || x.layer != self.active_layer || !x.exists_in(self.year)) {
            return Err("请在当前独占分区层和年份选择有效的领土单位".into());
        }
        let n = spec.units.clamp(2, 60);
        let mut pieces: Vec<PlanarPolygon> = self.dissolve(e);
        if pieces.is_empty() {
            return Err("这个国家没有领土".into());
        }
        let r = self.radius();
        pieces.sort_by(|a, b| geom::spherical_area(&b.outer, r).abs().total_cmp(&geom::spherical_area(&a.outer, r).abs()));
        // every piece near the largest one's longitudes
        let ref_lon = geom::bbox(&pieces[0].outer);
        let mid = (ref_lon[0] + ref_lon[2]) * 0.5;
        for p in pieces.iter_mut().skip(1) {
            let b = geom::bbox(&p.outer);
            let k = ((mid - (b[0] + b[2]) * 0.5) / 360.0).round() * 360.0;
            for q in p.outer.iter_mut().chain(p.holes.iter_mut().flatten()) {
                q[0] += k;
            }
        }
        let bb = pieces.iter().map(|p| geom::bbox(&p.outer)).fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
        let area: f64 = pieces.iter().map(|p| geom::signed_area(&p.outer).abs()).sum();
        let cs = (area / 20000.0).sqrt().max(0.005);
        let (x0, y1) = (bb[0] - cs, bb[3] + cs);
        let (w, h) = (((bb[2] - bb[0]) / cs).ceil() as usize + 3, ((bb[3] - bb[1]) / cs).ceil() as usize + 3);
        let centre = |c: usize| -> P2 { [x0 + ((c % w) as f64 + 0.5) * cs, y1 - ((c / w) as f64 + 0.5) * cs] };
        let inside: Vec<bool> = (0..w * h)
            .map(|c| {
                let p = centre(c);
                pieces.iter().any(|q| geom::point_in_poly(p, &q.outer) && !q.holes.iter().any(|hh| geom::point_in_poly(p, hh)))
            })
            .collect();
        let cells: Vec<usize> = (0..w * h).filter(|c| inside[*c]).collect();
        if cells.len() < n * 6 {
            return Err(format!("国土太小，无法划分为 {n} 块"));
        }
        let near = |c: usize| -> Vec<usize> {
            let (i, j) = ((c % w) as i64, (c / w) as i64);
            [(i - 1, j), (i + 1, j), (i, j - 1), (i, j + 1)].into_iter().filter(|(x, y)| *x >= 0 && *y >= 0 && (*x as usize) < w && (*y as usize) < h).map(|(x, y)| y as usize * w + x as usize).collect()
        };
        let cos = |c: usize| centre(c)[1].to_radians().cos().max(0.05);
        // what a step costs more: across a river, up or down steep ground
        let mut river = vec![false; w * h];
        if spec.follow {
            for rv in self.rivers.values().filter(|x| x.route.is_none() && !x.hidden) {
                for s in rv.pts.windows(2) {
                    let (a, b) = (s[0], s[1]);
                    let k = ((mid - a.lon) / 360.0).round() * 360.0;
                    let steps = ((geom::lon_delta(a.lon, b.lon)).hypot(b.lat - a.lat) / (cs * 0.5)).ceil().max(1.0) as usize;
                    for t in 0..=steps {
                        let f = t as f64 / steps as f64;
                        let p = [a.lon + k + geom::lon_delta(a.lon, b.lon) * f, a.lat + (b.lat - a.lat) * f];
                        let (i, j) = (((p[0] - x0) / cs).floor(), ((y1 - p[1]) / cs).floor());
                        if i >= 0.0 && j >= 0.0 && (i as usize) < w && (j as usize) < h {
                            river[j as usize * w + i as usize] = true;
                        }
                    }
                }
            }
        }
        let height = |c: usize| -> f64 {
            match (&self.terrain, spec.follow) {
                (Some(t), true) => {
                    let p = centre(c);
                    t.at(geom::wrap_lon(p[0]), p[1])
                }
                _ => 0.0,
            }
        };
        let heights: Vec<f64> = (0..w * h).map(|c| if inside[c] { height(c) } else { 0.0 }).collect();
        // seeds spread out (farthest first) over a sample of the land
        let mut rng = Rng::new(spec.seed);
        let d2 = |a: usize, b: usize| {
            let (pa, pb) = (centre(a), centre(b));
            ((pa[0] - pb[0]) * cos(a)).powi(2) + (pa[1] - pb[1]).powi(2)
        };
        // seeds for each separate piece of land in proportion to its size
        // (a small island gets none and joins its nearest unit), spread
        // out within it
        let mut groups: Vec<Vec<usize>> = Vec::new();
        {
            let mut seen = vec![false; w * h];
            for c in &cells {
                if seen[*c] {
                    continue;
                }
                let mut g = vec![*c];
                seen[*c] = true;
                let mut k = 0;
                while k < g.len() {
                    for nb in near(g[k]) {
                        if inside[nb] && !seen[nb] {
                            seen[nb] = true;
                            g.push(nb);
                        }
                    }
                    k += 1;
                }
                groups.push(g);
            }
        }
        groups.sort_by_key(|g| Reverse(g.len()));
        let mut share: Vec<usize> = groups.iter().map(|g| (g.len() as f64 / cells.len() as f64 * n as f64).floor() as usize).collect();
        while share.iter().sum::<usize>() < n {
            // the rest where the land per seed is largest
            let k = (0..groups.len()).max_by(|a, b| (groups[*a].len() as f64 / (share[*a] + 1) as f64).total_cmp(&(groups[*b].len() as f64 / (share[*b] + 1) as f64))).unwrap();
            share[k] += 1;
        }
        let mut seeds: Vec<usize> = Vec::new();
        for (g, m) in groups.iter().zip(&share) {
            let sample: Vec<usize> = (0..1500.min(g.len())).map(|_| g[(rng.next_f64() * g.len() as f64) as usize % g.len()]).collect();
            let mut mine: Vec<usize> = Vec::new();
            for _ in 0..*m {
                let far = |c: usize| mine.iter().map(|s| d2(*s, c)).fold(f64::INFINITY, f64::min);
                let pick = if mine.is_empty() { sample[0] } else { *sample.iter().max_by(|a, b| far(**a).total_cmp(&far(**b))).unwrap() };
                mine.push(pick);
            }
            seeds.extend(mine);
        }
        // pieces with no seed (small islands) go to the unit of the nearest
        // seed, and count towards its size while sizes are evened out
        let mut extra = vec![0.0f64; n];
        let mut islands: Vec<(usize, usize)> = Vec::new();
        for (g, m) in groups.iter().zip(&share) {
            if *m == 0 {
                let k = (0..n).min_by(|a, b| d2(seeds[*a], g[0]).total_cmp(&d2(seeds[*b], g[0]))).unwrap();
                extra[k] += g.iter().map(|c| cos(*c)).sum::<f64>();
                islands.extend(g.iter().map(|c| (*c, k)));
            }
        }
        // growth: four rounds moving each seed to the middle of its unit,
        // then a last one in which (for even sizes) the smallest unit always
        // takes its cheapest next cell
        let step_cost = |at: usize, nb: usize| -> f64 {
            let horizontal = nb % w != at % w;
            let mut step = cs * if horizontal { cos(at) } else { 1.0 };
            if river[nb] != river[at] {
                step += cs * 8.0;
            }
            step * (1.0 + (heights[nb] - heights[at]).abs() / 150.0)
        };
        let mut label = vec![usize::MAX; w * h];
        let mut weight = vec![1.0f64; n];
        let rounds = if spec.balance { 28 } else { 5 };
        let mut kept: Option<(f64, Vec<usize>)> = None;
        for round in 0..rounds {
            label.iter_mut().for_each(|l| *l = usize::MAX);
            let mut best = vec![f64::INFINITY; w * h];
            let mut heap: BinaryHeap<(Reverse<Cost>, usize, usize)> = BinaryHeap::new();
            for (k, s) in seeds.iter().enumerate() {
                best[*s] = 0.0;
                heap.push((Reverse(Cost(0.0)), *s, k));
            }
            while let Some((Reverse(Cost(c)), at, k)) = heap.pop() {
                if label[at] != usize::MAX || c > best[at] {
                    continue;
                }
                label[at] = k;
                for nb in near(at) {
                    if !inside[nb] || label[nb] != usize::MAX {
                        continue;
                    }
                    let total = c + step_cost(at, nb) * weight[k];
                    if total < best[nb] {
                        best[nb] = total;
                        heap.push((Reverse(Cost(total)), nb, k));
                    }
                }
            }
            if round >= 4 {
                // sizes evened out: a large unit's steps cost more next round;
                // the most even result so far is kept
                let mut size = extra.clone();
                for c in &cells {
                    if label[*c] < n {
                        size[label[*c]] += cos(*c);
                    }
                }
                let mean = size.iter().sum::<f64>() / n as f64;
                let spread = size.iter().fold(0.0f64, |m, x| m.max((x / mean).ln().abs()));
                if kept.as_ref().is_none_or(|k| spread < k.0) {
                    kept = Some((spread, label.clone()));
                }
                if round + 1 == rounds {
                    break;
                }
                let step = if round < 14 { 0.6 } else { 0.3 };
                for k in 0..n {
                    weight[k] *= (size[k].max(1e-9) / mean).powf(step);
                }
                continue;
            }
            if round + 1 == rounds {
                break;
            }
            // each seed to the cell of its unit nearest the unit's middle
            let mut sum = vec![[0.0f64; 3]; n];
            for c in &cells {
                if label[*c] < n {
                    let p = centre(*c);
                    let s = &mut sum[label[*c]];
                    s[0] += p[0];
                    s[1] += p[1];
                    s[2] += 1.0;
                }
            }
            for k in 0..n {
                if sum[k][2] > 0.0 {
                    let m = [sum[k][0] / sum[k][2], sum[k][1] / sum[k][2]];
                    if let Some(c) = cells.iter().filter(|c| label[**c] == k).min_by(|a, b| {
                        let (pa, pb) = (centre(**a), centre(**b));
                        ((pa[0] - m[0]).powi(2) + (pa[1] - m[1]).powi(2)).total_cmp(&((pb[0] - m[0]).powi(2) + (pb[1] - m[1]).powi(2)))
                    }) {
                        seeds[k] = *c;
                    }
                }
            }
        }
        if let Some((_, l)) = kept {
            label = l;
        }
        // pockets every unit near them was too full for: to a unit beside them
        let mut queue: std::collections::VecDeque<usize> = cells.iter().copied().filter(|c| label[*c] != usize::MAX).collect();
        while let Some(c) = queue.pop_front() {
            for nb in near(c) {
                if inside[nb] && label[nb] == usize::MAX {
                    label[nb] = label[c];
                    queue.push_back(nb);
                }
            }
        }
        // the islands, to the units they were counted with
        for (c, k) in &islands {
            label[*c] = *k;
        }
        // any other land no seed reached: the nearest seed's
        for c in &cells {
            if label[*c] == usize::MAX {
                label[*c] = (0..n).min_by(|a, b| d2(seeds[*a], *c).total_cmp(&d2(seeds[*b], *c))).unwrap();
            }
        }
        // the cells outside take their nearest neighbour inside, so the units cover the whole grid
        let mut queue: std::collections::VecDeque<usize> = cells.iter().copied().collect();
        while let Some(c) = queue.pop_front() {
            for nb in near(c) {
                if label[nb] == usize::MAX {
                    label[nb] = label[c];
                    queue.push_back(nb);
                }
            }
        }
        // each unit's cells as polygons, cut to the country
        let region = to_geo(&pieces.iter().flat_map(|p| std::iter::once(p.outer.clone()).chain(p.holes.iter().map(|hh| {
            // holes clockwise for the grouping
            let mut v = hh.clone();
            if geom::signed_area(&v) > 0.0 {
                v.reverse();
            }
            v
        }))).map(|mut r| {
            if geom::signed_area(&r) == 0.0 {
                r.clear();
            }
            r
        }).filter(|r| r.len() >= 3).collect::<Vec<_>>());
        // pieces that meet (across the antimeridian, where the map's faces
        // are cut) made one: a unit's piece must not stop at the seam line
        let region = region.0.iter().fold(geo::MultiPolygon::<f64>::new(vec![]), |acc, p| acc.union(&geo::MultiPolygon::new(vec![p.clone()])));
        let mut units = Vec::new();
        let mut areas = Vec::new();
        for outline in crate::trace::grid_regions(w, h, n, |index| Some(label[index])) {
            let rings: Vec<Vec<P2>> = outline.into_iter()
                .map(|ring| ring.into_iter().map(|(x, y)| [x0 + x as f64 * cs, y1 - y as f64 * cs]).collect())
                .collect();
            let cut = to_geo(&rings).intersection(&region);
            let mut parts: Vec<Part> = Vec::new();
            let mut km2 = 0.0;
            for poly in cut.0 {
                // the cut rounds coordinates a little, differently for each
                // unit: a corner on a grid line goes back exactly onto it, so
                // that a border two units share is the very same line in both
                // (otherwise drawing one after the other can leave slivers
                // that the rebuild refuses)
                // (the same arithmetic as the corners traced above)
                let sx = |x: f64| -> f64 {
                    let g = ((x - x0) / cs).round();
                    if (x - (x0 + g * cs)).abs() < 1e-7 { x0 + g * cs } else { x }
                };
                let sy = |y: f64| -> f64 {
                    let g = ((y1 - y) / cs).round();
                    if (y - (y1 - g * cs)).abs() < 1e-7 { y1 - g * cs } else { y }
                };
                let ring = |ls: &geo::LineString<f64>| -> Vec<LonLat> {
                    let mut v: Vec<LonLat> = ls.coords().map(|c| LonLat::new(sx(c.x), sy(c.y))).collect();
                    v.dedup();
                    if v.len() > 1 && v.first() == v.last() {
                        v.pop();
                    }
                    v
                };
                // a ring that touches itself at a corner is cut there into
                // simple rings: loops of the outer ring that wind the other
                // way are holes touching it
                let planar = |v: &Vec<LonLat>| -> Vec<P2> { v.iter().map(|p| [p.lon, p.lat]).collect() };
                let mut outers: Vec<Vec<LonLat>> = Vec::new();
                let mut holes: Vec<Vec<LonLat>> = Vec::new();
                for l in simple_loops(&ring(poly.exterior())) {
                    if geom::signed_area(&planar(&l)) > 0.0 { outers.push(l) } else { holes.push(l) }
                }
                for hole in poly.interiors() {
                    holes.extend(simple_loops(&ring(hole)));
                }
                let mut here: Vec<Part> = outers
                    .into_iter()
                    .filter(|o| geom::signed_area(&planar(o)).abs() >= cs * cs * 0.5)
                    .map(|outer| Part { outer, holes: vec![] })
                    .collect();
                for hole in holes {
                    let hp = planar(&hole);
                    if geom::signed_area(&hp).abs() < 1e-10 {
                        continue;
                    }
                    // the part around the hole (tested at a corner not on the part's ring)
                    if let Some(part) = here.iter_mut().find(|part| {
                        let op = planar(&part.outer);
                        hp.iter().find(|q| !op.contains(q)).is_some_and(|q| geom::point_in_poly(*q, &op))
                    }) {
                        part.holes.push(hole);
                    }
                }
                for part in &here {
                    km2 += geom::spherical_area(&planar(&part.outer), r).abs();
                    km2 -= part.holes.iter().map(|h| geom::spherical_area(&planar(h), r).abs()).sum::<f64>();
                }
                // a polar country's land is closed along the pole in the plane;
                // drawn on the globe, a ring around the pole needs no such line
                // (its points on the pole made it touch itself)
                let off_pole = |r: &mut Vec<LonLat>| {
                    r.retain(|p| p.lat.abs() < 90.0 - 1e-9);
                    r.dedup();
                    while r.len() > 1 && r.first() == r.last() {
                        r.pop();
                    }
                };
                for part in here.iter_mut() {
                    off_pole(&mut part.outer);
                    part.holes.iter_mut().for_each(off_pole);
                    part.holes.retain(|h| h.len() >= 3);
                }
                here.retain(|p| p.outer.len() >= 3);
                parts.extend(here);
            }
            units.push(parts);
            areas.push(km2);
        }
        Ok(Plan { target: e, units, areas })
    }

    /// Whether two faces have had the same history in every layer and as
    /// land or water, apart from the active layer from the selected year on
    /// (all of it when no year is selected): [`Document::same_history`]
    /// but for the change being made.
    fn same_past(&self, a: FaceId, b: FaceId) -> bool {
        let active = |f: FaceId| -> (Option<EntityId>, Vec<(i32, Option<EntityId>)>) {
            match self.year {
                Some(y) => (self.owner_base.get(&f).copied(), self.owner_changes.get(&f).map(|c| c.range(..y).map(|(y, o)| (*y, *o)).collect()).unwrap_or_default()),
                None => (None, vec![]),
            }
        };
        if active(a) != active(b) {
            return false;
        }
        if self.kind_base.get(&a) != self.kind_base.get(&b) || self.kind_changes.get(&a) != self.kind_changes.get(&b) {
            return false;
        }
        self.layers.iter().filter(|(l, _)| **l != self.active_layer).all(|(_, l)| {
            l.parked.base.get(&a) == l.parked.base.get(&b)
                && l.parked.changes.get(&a) == l.parked.changes.get(&b)
                && l.shares.base.get(&a) == l.shares.base.get(&b)
                && l.shares.changes.get(&a) == l.shares.changes.get(&b)
        })
    }

    /// Carry out a plan: units drawn into the land (from the selected year
    /// on), new countries beside the old one — which keeps the first unit —
    /// or provinces under it; the new inner borders smoothed. Returns the
    /// units' entities.
    pub fn apply_partition(&mut self, plan: &Plan, spec: &PartitionSpec) -> TResult<Vec<EntityId>> {
        let mut draft = self.clone();
        let units = draft.apply_partition_to_draft(plan, spec)?;
        *self = draft;
        Ok(units)
    }

    fn apply_partition_to_draft(&mut self, plan: &Plan, spec: &PartitionSpec) -> TResult<Vec<EntityId>> {
        let e = plan.target;
        let old = self.entities.get(&e).ok_or("国家不存在")?.clone();
        let taken: HashSet<String> = self.entities.values().map(|x| x.name.clone()).collect();
        let kind = if spec.provinces { NameKind::Region } else { NameKind::Country };
        let names = names::unique(spec.style, kind, plan.units.len(), spec.seed.wrapping_add(101), &taken);
        let mut ents: Vec<EntityId> = Vec::new();
        for k in 0..plan.units.len() {
            if k == 0 && !spec.provinces {
                ents.push(e);
                continue;
            }
            let name = names.get(k).cloned().unwrap_or_else(|| format!("{}{}", old.name, k + 1));
            let id = if spec.provinces {
                self.add_subdivision(e, &name)?
            } else {
                self.add_entity(EntityKind::Country, &name)
            };
            // as long-lived as the country it comes from
            if !spec.provinces && (old.founded.is_some() || old.ended.is_some()) {
                let _ = self.set_lifespan(id, old.founded, old.ended);
            }
            ents.push(id);
        }
        // larger pieces first, so a unit inside another wins its ground
        let mut order: Vec<(usize, &Part)> = plan.units.iter().enumerate().flat_map(|(k, ps)| ps.iter().map(move |p| (k, p))).collect();
        order.sort_by(|a, b| {
            let ar = |r: &Vec<LonLat>| geom::signed_area(&r.iter().map(|p| [p.lon, p.lat]).collect::<Vec<_>>()).abs();
            ar(&b.1.outer).total_cmp(&ar(&a.1.outer))
        });
        for (k, part) in order {
            if k == 0 && !spec.provinces {
                continue;
            }
            // with its holes: a lake or an enclave inside stays as it is
            let intent = if spec.provinces {
                DrawIntent::Subdivide { parent: e, child: ents[k] }
            } else {
                DrawIntent::Take(ents[k])
            };
            let (next, _) = overlay::apply_with_holes(self, &part.outer, &part.holes, intent).map_err(|err| format!("这个方案无法应用（{err}）；可以换一个方案或改变单位数"))?;
            *self = next;
        }
        // provinces: whatever is left directly with the country goes to its neighbouring unit
        if spec.provinces {
            let left: Vec<crate::topo::FaceId> = self.faces_of(e);
            for f in left {
                let mut best: Option<(EntityId, f64)> = None;
                for (_, ed) in self.topo.edges.iter() {
                    let other = if ed.left == f { ed.right } else if ed.right == f { ed.left } else { continue };
                    if let Some(o) = self.owner.get(&other).copied().filter(|o| ents.contains(o)) {
                        let len = self.topo.edge_full(ed).windows(2).map(|w| geom::geo_dist_deg(w[0], w[1])).sum::<f64>();
                        if best.is_none_or(|b| len > b.1) {
                            best = Some((o, len));
                        }
                    }
                }
                if let Some((o, _)) = best {
                    self.assign(f, Some(o))?;
                }
            }
        }
        // the new inner borders smoothed (nodes stay; a crossing keeps the old
        // line). Only a line the partition drew through a face: its two sides
        // have the same history in everything but this year's units, so
        // moving it changes nothing else (an older border between the units
        // may have parted other owners in an earlier year, or other layers)
        let set: HashSet<EntityId> = ents.iter().copied().collect();
        let inner: Vec<crate::topo::EdgeId> = self
            .topo
            .edges
            .iter()
            .filter(|(_, ed)| {
                let (a, b) = (self.owner.get(&ed.left), self.owner.get(&ed.right));
                matches!((a, b), (Some(a), Some(b)) if a != b && set.contains(a) && set.contains(b)) && self.same_past(ed.left, ed.right)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in inner {
            let full = self.topo.edge_full(&self.topo.edges[&id]);
            if full.len() < 3 {
                continue;
            }
            let s = crate::worldgen::smooth_line(&full, 2);
            let backup = self.topo.clone();
            if self.topo.replace_edge_points(id, s[1..s.len() - 1].to_vec()).is_err() {
                self.topo = backup;
            }
        }
        if !spec.provinces {
            self.apply_auto_colours(&crate::colouring::palette());
        }
        Ok(ents)
    }
}
