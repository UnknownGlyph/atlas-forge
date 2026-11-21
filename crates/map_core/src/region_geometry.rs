//! Moves and joins entire land components without renumbering shared faces.
use super::{ArcRef, EdgeId, FaceId, FaceKind, LonLat, NodeId, TResult, Topology, BACKGROUND};
use crate::geom;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GeoOffset {
    pub lon: f64,
    pub lat: f64,
}

/// A uniform scale works in the continuous longitude/latitude frame around
/// its pivot. This keeps shared arcs and straight segments consistent.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LandTransform {
    Translate(GeoOffset),
    Rotate(GlobeRotation),
    Scale { pivot: LonLat, factor: f64 },
}

impl LandTransform {
    pub fn apply(self, point: LonLat) -> LonLat {
        match self {
            Self::Translate(offset) => offset.apply(point),
            Self::Rotate(rotation) => rotation.apply(point),
            Self::Scale { pivot, factor } => LonLat::new(
                geom::wrap_lon(pivot.lon + geom::lon_delta(pivot.lon, point.lon) * factor),
                pivot.lat + (point.lat - pivot.lat) * factor,
            ),
        }
    }

    pub fn is_identity(self) -> bool {
        match self {
            Self::Translate(offset) => offset == GeoOffset::default(),
            Self::Rotate(rotation) => rotation.0 == glam::DQuat::IDENTITY,
            Self::Scale { factor, .. } => factor == 1.0,
        }
    }

    fn validate(self) -> TResult<()> {
        match self {
            Self::Translate(offset) if !offset.lat.is_finite() || !offset.lon.is_finite() => {
                Err("经纬度偏移必须是有限数值。".into())
            }
            Self::Scale { pivot, factor }
                if !pivot.lon.is_finite()
                    || !(-90.0..=90.0).contains(&pivot.lat)
                    || !factor.is_finite()
                    || factor <= 0.0 =>
            {
                Err("缩放中心必须有效，缩放比例必须大于零。".into())
            }
            _ => Ok(()),
        }
    }
}

/// A rigid rotation of all selected coordinates on the planet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlobeRotation(glam::DQuat);

impl GlobeRotation {
    pub fn between(from: LonLat, to: LonLat) -> TResult<Self> {
        let valid = |p: LonLat| p.lon.is_finite() && (-90.0..=90.0).contains(&p.lat);
        if !valid(from) || !valid(to) {
            return Err("移动起点和终点必须位于行星表面。".into());
        }
        let from = unit(from);
        let to = unit(to);
        let axis = from.cross(to);
        let dot = from.dot(to).clamp(-1.0, 1.0);
        let angle = axis.length().atan2(dot);
        let rotation = if angle < 1e-14 {
            glam::DQuat::IDENTITY
        } else if axis.length_squared() < 1e-28 {
            // Exactly antipodal endpoints admit multiple rotations. Use a
            // fixed orthogonal axis so preview and commit take the same path.
            let reference = if from.z.abs() < 0.9 { glam::DVec3::Z } else { glam::DVec3::X };
            glam::DQuat::from_axis_angle(from.cross(reference).normalize(), angle)
        } else {
            glam::DQuat::from_axis_angle(axis.normalize(), angle)
        };
        Ok(Self(rotation))
    }

    fn apply(self, point: LonLat) -> LonLat {
        if self.0 == glam::DQuat::IDENTITY {
            return point;
        }
        let p = self.0 * unit(point);
        LonLat::new(
            geom::wrap_lon(p.y.atan2(p.x).to_degrees()),
            p.z.atan2(p.x.hypot(p.y)).to_degrees(),
        )
    }
}

fn unit(point: LonLat) -> glam::DVec3 {
    let (sin_lon, cos_lon) = point.lon.to_radians().sin_cos();
    let (sin_lat, cos_lat) = point.lat.to_radians().sin_cos();
    glam::DVec3::new(cos_lat * cos_lon, cos_lat * sin_lon, sin_lat)
}

/// Center the group in the smallest continuous longitude interval covering
/// its coordinates. Sampling a coastline more densely does not change it.
pub fn land_group_center(points: impl IntoIterator<Item = LonLat>) -> Option<LonLat> {
    let mut longitudes = Vec::new();
    let mut south = f64::INFINITY;
    let mut north = f64::NEG_INFINITY;
    for point in points {
        longitudes.push(point.lon.rem_euclid(360.0));
        south = south.min(point.lat);
        north = north.max(point.lat);
    }
    let first = *longitudes.first()?;
    longitudes.sort_unstable_by(f64::total_cmp);
    let mut west = longitudes[0];
    let mut gap = west + 360.0 - longitudes[longitudes.len() - 1];
    for pair in longitudes.windows(2) {
        let next_gap = pair[1] - pair[0];
        if next_gap > gap {
            west = pair[1];
            gap = next_gap;
        }
    }
    let lon = if longitudes.len() == 1 { first } else { west + (360.0 - gap) * 0.5 };
    Some(LonLat::new(geom::wrap_lon(lon), (south + north) * 0.5))
}

impl GeoOffset {
    pub fn between(from: LonLat, to: LonLat) -> Self {
        Self {
            lon: geom::lon_delta(from.lon, to.lon),
            lat: to.lat - from.lat,
        }
    }

    pub fn apply(self, point: LonLat) -> LonLat {
        LonLat::new(geom::wrap_lon(point.lon + self.lon), point.lat + self.lat)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct LandBlock {
    pub faces: BTreeSet<FaceId>,
    pub edges: BTreeSet<EdgeId>,
    nodes: BTreeSet<NodeId>,
}

impl Topology {
    /// Include every face and boundary carrying a shared node, including
    /// subdivisions and enclosed lakes. Open ocean never bridges components.
    pub fn land_block(&self, seeds: &[FaceId]) -> TResult<LandBlock> {
        if self.faces[&BACKGROUND].kind != FaceKind::Water {
            return Err("当前地图的陆地背景没有可整体移动的封闭外边界。".into());
        }
        if seeds.is_empty() {
            return Err("请先选中一个有封闭边界的陆地面。".into());
        }
        if seeds.iter().any(|id| {
            *id == BACKGROUND || !self.faces.get(id).is_some_and(|f| f.kind == FaceKind::Land)
        }) {
            return Err("请从陆地内开始选择。开阔海域不能作为整体移动。".into());
        }
        let mut by_node = BTreeMap::<NodeId, Vec<EdgeId>>::new();
        for (&id, e) in &self.edges {
            by_node.entry(e.a).or_default().push(id);
            if e.b != e.a {
                by_node.entry(e.b).or_default().push(id);
            }
        }
        let mut block = LandBlock {
            faces: BTreeSet::new(),
            edges: BTreeSet::new(),
            nodes: BTreeSet::new(),
        };
        let mut remaining: VecDeque<_> = seeds.iter().copied().collect();
        while let Some(id) = remaining.pop_front() {
            if id == BACKGROUND || !block.faces.insert(id) {
                continue;
            }
            let face = &self.faces[&id];
            for ring in std::iter::once(&face.outer).chain(&face.holes) {
                for r in ring {
                    let edge = &self.edges[&r.edge];
                    for node in [edge.a, edge.b] {
                        if block.nodes.insert(node) {
                            for &eid in &by_node[&node] {
                                block.edges.insert(eid);
                                let e = &self.edges[&eid];
                                remaining.extend([e.left, e.right]);
                            }
                        }
                    }
                }
            }
        }
        Ok(block)
    }

    pub fn block_coastlines(&self, block: &LandBlock) -> Vec<Vec<LonLat>> {
        self.faces[&BACKGROUND]
            .holes
            .iter()
            .filter(|ring| ring.iter().any(|r| block.edges.contains(&r.edge)))
            .map(|ring| self.ring_coords(ring))
            .collect()
    }

    /// Build the proposed state separately, so every failed check leaves the
    /// map, its indexes and its revisions exactly as they were.
    pub fn translated_block(&self, block: &LandBlock, offset: GeoOffset) -> TResult<Self> {
        self.transformed_block(block, LandTransform::Translate(offset))
    }

    pub(crate) fn transformed_block(
        &self,
        block: &LandBlock,
        transform: LandTransform,
    ) -> TResult<Self> {
        transform.validate()?;
        if transform.is_identity() {
            return Ok(self.clone());
        }
        let polar = self.faces[&BACKGROUND]
            .holes
            .iter()
            .any(|ring| ring.iter().any(|r| block.edges.contains(&r.edge)) && self.is_polar(ring));
        if polar {
            match transform {
                LandTransform::Translate(offset) if offset.lat != 0.0 => {
                    return Err(
                        "所选陆块包围极点。南北平移会改变极区的闭合关系，可以只沿经度移动。".into(),
                    )
                }
                LandTransform::Scale { .. } | LandTransform::Rotate(_) => {
                    return Err("所选陆块包含环绕极点的边界，请在移动工具中选择仅沿经度偏移。".into())
                }
                _ => {}
            }
        }
        let outside = |point: LonLat| {
            let y = transform.apply(point).lat;
            !(-90.0..=90.0).contains(&y) || (y.abs() >= 90.0 && point.lat.abs() < 90.0)
        };
        if block
            .nodes
            .iter()
            .any(|id| self.nodes.get(id).is_none_or(|n| outside(n.pos)))
            || block.edges.iter().any(|id| {
                self.edges
                    .get(id)
                    .is_none_or(|e| e.pts.iter().copied().any(outside))
            })
        {
            return Err("变换会让选中陆块越过极点或失去原边界。请减小偏移量或缩放比例。".into());
        }
        if let LandTransform::Scale { pivot, factor } = transform {
            for &id in &block.edges {
                let path = self.edge_full(&self.edges[&id]);
                if path.windows(2).any(|pair| {
                    let a = geom::lon_delta(pivot.lon, pair[0].lon);
                    let b = geom::lon_delta(pivot.lon, pair[1].lon);
                    (b - a - geom::lon_delta(pair[0].lon, pair[1].lon)).abs() > 1e-7
                        || (a * factor).abs() >= 180.0
                        || (b * factor).abs() >= 180.0
                        || ((b - a) * factor).abs() >= 180.0
                }) {
                    return Err(
                        "缩放轮廓跨过了中心对侧的接缝。请减小缩放比例或重新选择中心。".into(),
                    );
                }
            }
        }
        let mut next = self.clone();
        for id in &block.nodes {
            let node = next.nodes.get_mut(id).unwrap();
            node.pos = transform.apply(node.pos);
        }
        for &id in &block.edges {
            let mut edge = next.edges[&id].clone();
            for point in &mut edge.pts {
                *point = transform.apply(*point);
            }
            next.put_edge(id, edge);
        }
        if matches!(transform, LandTransform::Rotate(_))
            && next.faces[&BACKGROUND].holes.iter().any(|ring| {
                ring.iter().any(|edge| block.edges.contains(&edge.edge)) && next.is_polar(ring)
            })
        {
            return Err("移动会让选中陆块包围极点并改变海岸闭合方式，请调整移动方向。".into());
        }
        next.touch(BACKGROUND);
        for &id in &block.faces {
            next.touch(id);
        }
        next.check_separate_faces(&block.faces, &block.edges)?;
        Ok(next)
    }

    /// Add already built geography in the open ocean. New IDs start past
    /// this project's allocator. Only the background face is shared.
    pub fn append_disjoint(&mut self, source: Topology) -> TResult<()> {
        if source.faces[&BACKGROUND].kind != FaceKind::Water
            || self.faces[&BACKGROUND].kind != FaceKind::Water
        {
            return Err("只能将闭合陆块并入开放海域。".into());
        }
        let lowest = self.next_id();
        if source.nodes.keys().any(|n| n.0 < lowest)
            || source.edges.keys().any(|e| e.0 < lowest)
            || source
                .faces
                .keys()
                .any(|f| *f != BACKGROUND && f.0 < lowest)
        {
            return Err("新地理对象的编号与工程已有对象冲突。".into());
        }
        let mut next = self.clone();
        next.nodes
            .extend(source.nodes.iter().map(|(&id, value)| (id, value.clone())));
        next.edges
            .extend(source.edges.iter().map(|(&id, value)| (id, value.clone())));
        next.faces.extend(
            source
                .faces
                .iter()
                .filter(|(&id, _)| id != BACKGROUND)
                .map(|(&id, value)| (id, value.clone())),
        );
        next.faces
            .get_mut(&BACKGROUND)
            .unwrap()
            .holes
            .extend(source.faces[&BACKGROUND].holes.clone());
        next.reserve_ids(source.next_id());
        next.rebuild_index();
        next.touch(BACKGROUND);
        let faces = source
            .faces
            .keys()
            .copied()
            .filter(|&f| f != BACKGROUND)
            .collect();
        let edges = source.edges.keys().copied().collect();
        next.check_separate_faces(&faces, &edges)?;
        *self = next;
        Ok(())
    }

    fn check_separate_faces(
        &self,
        faces: &BTreeSet<FaceId>,
        edges: &BTreeSet<EdgeId>,
    ) -> TResult<()> {
        // Validate both sides of every changed edge, including the ocean rings.
        let mut checked = faces.clone();
        checked.insert(BACKGROUND);
        self.validate_local(&checked, edges)
            .map_err(|err| format!("此位置会破坏共享边界或与其他地理要素相交。{err}"))?;
        // The generic crossing check permits shared endpoints. A newly
        // touching endpoint from an independent block has a different ID,
        // so reject it before either block can be edited as if joined.
        for &id in edges {
            let edge = &self.edges[&id];
            if edge.left != BACKGROUND && edge.right != BACKGROUND {
                continue;
            }
            let here = self.edge_full(edge);
            for (_, other) in self.edges_near(edge.bbox).filter(|(id, e)| {
                !edges.contains(id) && super::bbox_overlap_wrapped(&edge.bbox, &e.bbox)
            }) {
                let there = self.edge_full(other);
                if here
                    .iter()
                    .any(|&a| there.iter().any(|&b| geom::geo_dist_deg(a, b) < 1e-9))
                {
                    return Err("移动会让原本独立的边界在同一顶点接触。请留一点间距。".into());
                }
            }
        }
        let sample = |r: ArcRef| self.node_pos(self.ref_start(r));
        for (&id, face) in self.faces.iter().filter(|(&id, _)| id != BACKGROUND) {
            let Some(&first) = face.outer.first() else {
                continue;
            };
            let p = sample(first);
            let bbox = [p.lon, p.lat, p.lon, p.lat];
            for other in self.faces_in_bbox(bbox) {
                if other != BACKGROUND
                    && faces.contains(&id) != faces.contains(&other)
                    && self.face_contains(other, p)
                {
                    return Err("移动后的陆块会覆盖或包围另一块已有陆地或封闭水域。".into());
                }
            }
        }
        Ok(())
    }
}
