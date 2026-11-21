use crate::{
    geom::LonLat,
    topo::{FaceId, GeoOffset, LandBlock, LandTransform, TResult, BACKGROUND},
    world_book::{Node, WorldBook},
    Document,
};

impl Document {
    /// Shift a selection of isolated, connected geographic blocks. IDs and
    /// ownership records remain in place. Coordinates move across every year.
    pub fn translate_land(&mut self, seeds: &[FaceId], offset: GeoOffset) -> TResult<LandBlock> {
        self.transform_land(seeds, LandTransform::Translate(offset))
    }

    pub fn scale_land(&mut self, seed: FaceId, pivot: LonLat, factor: f64) -> TResult<LandBlock> {
        self.transform_land(&[seed], LandTransform::Scale { pivot, factor })
    }

    pub fn transform_land(&mut self, seeds: &[FaceId], transform: LandTransform) -> TResult<LandBlock> {
        if self.paper.is_some() {
            return Err("纸面画板没有地理陆块。可以在选择工具中移动画板要素。".into());
        }
        let block = self.topo.land_block(seeds)?;
        if transform.is_identity() {
            return Ok(block);
        }
        let within = |p: LonLat| block.faces.contains(&self.topo.face_at(p));
        if self.terrain.is_some() {
            return Err("此地图有固定在行星坐标上的高程栅格，变换陆地会使高程与海岸错位。".into());
        }
        if self
            .underlays
            .iter()
            .any(|image| image.points.iter().any(|point| within(point.ll)))
        {
            return Err("选中陆块包含参考图的配准点，移动会破坏既有配准。".into());
        }
        let topo = self.topo.transformed_block(&block, transform)?;
        let footprint = |index: usize| {
            let center = crate::climate::center(index);
            let dx = 180.0 / crate::climate::W as f64;
            let dy = 90.0 / crate::climate::H as f64;
            let bounds = [
                center.lon - dx,
                center.lat - dy,
                center.lon + dx,
                center.lat + dy,
            ];
            [&self.topo, &topo].iter().any(|topology| {
                topology
                    .faces_in_bbox(bounds)
                    .iter()
                    .any(|id| block.faces.contains(id))
            })
        };
        if self.climate.as_ref().is_some_and(|p| {
            p.surface_edits.keys().copied().any(footprint)
                || p.scenarios()
                    .any(|s| s.edits.keys().any(|(index, _)| footprint(*index)))
        }) {
            return Err(
                "工程有固定经纬网格的气候地表修订或固定值，需要先处理这些修订以免错位。".into(),
            );
        }
        let moved = |p: LonLat| transform.apply(p);
        let check = |p: LonLat| (-89.999..=89.999).contains(&moved(p).lat);
        let mut next = self.clone();
        next.topo = topo;
        for item in self.features() {
            let points = self.feature_points(item.what);
            if !points.iter().copied().any(within) {
                continue;
            }
            if self.is_locked(item.what) {
                return Err(format!(
                    "选中陆块上的{}“{}”已锁定。",
                    item.what.kind_name(),
                    item.name
                ));
            }
            if points.iter().any(|&p| {
                let face = self.topo.face_at(p);
                face != BACKGROUND && !block.faces.contains(&face)
            }) {
                return Err(format!(
                    "{}“{}”连接了未选中的陆块。请一起选择连接两端。",
                    item.what.kind_name(),
                    item.name
                ));
            }
            if !points.iter().copied().all(check) {
                return Err(format!(
                    "移动会使{}“{}”越过极点。",
                    item.what.kind_name(),
                    item.name
                ));
            }
            next.map_feature(item.what, &moved)?;
        }
        for (&id, entity) in &self.entities {
            let position = entity.pos.filter(|p| within(*p));
            let label = entity.label.filter(|l| within(l.at));
            if position.is_none() && label.is_none() {
                continue;
            }
            let edit = next.entities.get_mut(&id).unwrap();
            if let Some(p) = position {
                edit.pos = Some(moved(p));
            }
            if let Some(mut l) = label {
                l.at = moved(l.at);
                edit.label = Some(l);
            }
        }
        if let Some(old_book) = &self.book {
            let board = WorldBook::current(self);
            let book = WorldBook::edit(&mut next);
            for (&id, place) in &old_book.places {
                if place.board == board && within(place.point) {
                    book.places.get_mut(&id).unwrap().point = moved(place.point);
                }
            }
            for (&node, &p) in &old_book.anchors {
                let on_block = match node {
                    Node::Face(_, id) => block.faces.contains(&FaceId(id)),
                    Node::Edge(_, id) => block.edges.contains(&crate::topo::EdgeId(id)),
                    _ => within(p),
                };
                if node.board() == Some(board) && on_block {
                    book.anchors.insert(node, moved(p));
                }
            }
        }
        *self = next;
        Ok(block)
    }
}
