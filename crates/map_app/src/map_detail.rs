//! Scale-dependent visibility for the two editing views.
use crate::app::App;
use crate::view::{Tool, ViewGeom, ViewKind};
use eframe::egui::{self, Color32, Rect, Vec2};
use map_core::{Document, EntityId, EntityKind, FaceId, LonLat, BACKGROUND};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Copy)]
pub(crate) struct Detail(f64);

impl Detail {
    const THRESHOLDS: [f64; 4] = [1.8, 3.5, 6.0, 10.0];

    fn in_view(view: &ViewGeom, enabled: bool) -> Self {
        if !enabled {
            return Self(f64::INFINITY);
        }
        let size = view.rect.size();
        let relative = match view.kind {
            ViewKind::Flat => {
                let bounds = view.cam.proj.bounds();
                let fitted = (f64::from(size.x) / (bounds[2] - bounds[0]))
                    .min(f64::from(size.y) / (bounds[3] - bounds[1]))
                    * 0.95;
                view.cam.zoom / fitted
            }
            ViewKind::Globe => {
                (map_render::camera::GlobeCamera::default().dist - 1.0) / (view.globe.dist - 1.0)
            }
        };
        Self(relative)
    }

    pub fn visible(self, rank: usize) -> bool {
        self.opacity(rank) >= 0.5
    }

    pub fn opacity(self, rank: usize) -> f32 {
        if rank == 0 {
            return 1.0;
        }
        let threshold = Self::THRESHOLDS[(rank - 1).min(Self::THRESHOLDS.len() - 1)];
        ((self.0 / threshold - 1.0) / 0.25).clamp(0.0, 1.0) as f32
    }

    pub fn owner(self, doc: &Document, leaf: EntityId) -> EntityId {
        let lineage: Vec<_> = doc.ancestors(leaf).collect();
        let rank = lineage.len().saturating_sub(1);
        if rank == 0 || self.visible(rank.min(3)) {
            return leaf;
        }
        let visible = (1..=rank)
            .rev()
            .find(|depth| self.visible((*depth).min(3)))
            .unwrap_or(0);
        lineage[rank - visible]
    }
}

struct Region {
    id: EntityId,
    at: LonLat,
    rank: usize,
    size: f64,
}

#[derive(Default)]
pub(crate) struct DetailCache {
    source: Option<Document>,
    regions: Vec<Region>,
    capitals: HashMap<EntityId, usize>,
    pub areas: Arc<HashMap<EntityId, f64>>,
}

impl DetailCache {
    pub fn update(&mut self, doc: &Document) {
        if self.source.as_ref().is_some_and(|old| {
            old.owner.ptr_eq(&doc.owner)
                && old.entities.ptr_eq(&doc.entities)
                && old.topo.faces.ptr_eq(&doc.topo.faces)
                && old.year == doc.year
                && old.active_layer == doc.active_layer
                && old.radius() == doc.radius()
                && (!doc.active_is_shares() || old.layers.ptr_eq(&doc.layers))
        }) {
            return;
        }
        self.capitals.clear();
        let mut areas = doc.areas_by_entity();
        if !doc.active_is_shares() {
            let direct: Vec<_> = areas.iter().map(|(&id, &area)| (id, area)).collect();
            for (id, area) in direct {
                for parent in doc.ancestors(id).skip(1) {
                    *areas.entry(parent).or_default() += area;
                }
            }
        }
        self.areas = Arc::new(areas);
        for (id, e) in doc.entities.iter().filter(|(_, e)| {
            e.kind == EntityKind::Country && e.layer == doc.active_layer && e.exists_in(doc.year)
        }) {
            if let Some(city) = e.capital {
                let rank = doc.ancestors(*id).count().saturating_sub(1).min(3);
                self.capitals
                    .entry(city)
                    .and_modify(|r| *r = (*r).min(rank))
                    .or_insert(rank);
            }
        }
        let mut largest = HashMap::<EntityId, (FaceId, f64)>::new();
        let mut ancestry = HashMap::<EntityId, Vec<EntityId>>::new();
        for (face, owner) in doc.owner.iter() {
            if *face == BACKGROUND {
                continue;
            }
            let units = ancestry.entry(*owner).or_insert_with(|| {
                doc.ancestors(*owner)
                    .filter(|id| doc.entities.get(id).is_some_and(|e| e.parent.is_some()))
                    .collect()
            });
            if units.is_empty() {
                continue;
            }
            let Some(f) = doc.topo.faces.get(face) else {
                continue;
            };
            let bbox = f.bbox;
            let extent = (bbox[2] - bbox[0])
                * (bbox[3] - bbox[1])
                * ((bbox[3] + bbox[1]) * 0.5).to_radians().cos().max(0.05);
            for unit in units {
                let entry = largest.entry(*unit).or_insert((*face, extent));
                if extent > entry.1 {
                    *entry = (*face, extent);
                }
            }
        }
        self.regions = largest
            .into_iter()
            .filter_map(|(id, (face, size))| {
                let (outer, holes) = doc.topo.face_planar(face);
                let ([lon, lat], _) = map_core::geom::interior_point(&outer, &holes)?;
                Some(Region {
                    id,
                    at: LonLat::new(map_core::geom::wrap_lon(lon), lat),
                    rank: doc.ancestors(id).count().saturating_sub(1).min(3),
                    size,
                })
            })
            .collect();
        self.regions.sort_by(|a, b| {
            a.rank
                .cmp(&b.rank)
                .then(b.size.total_cmp(&a.size))
                .then(a.id.cmp(&b.id))
        });
        self.source = Some(doc.clone());
    }

    pub fn city_rank(&self, city: EntityId) -> usize {
        self.capitals.get(&city).copied().unwrap_or(3)
    }
}

impl App {
    pub(crate) fn map_detail(&self, vg: &ViewGeom) -> Detail {
        Detail::in_view(vg, self.prefs.progressive_detail && self.doc.paper.is_none())
    }

    pub(crate) fn city_shown(&self, id: EntityId, detail: Detail) -> bool {
        self.doc.style.show_cities
            && (self.tool == Tool::City
                || self.selected == Some(id)
                || detail.visible(self.detail_cache.city_rank(id)))
    }

    pub(crate) fn line_shown(
        &self,
        id: map_core::doc::LineId,
        line: &map_core::doc::LineFeature,
        detail: Detail,
    ) -> bool {
        line.route.is_none_or(|s| s.arrow || s.broad)
            || matches!(self.tool, Tool::River | Tool::Pen)
            || self.selected_river == Some(id)
            || self
                .picked
                .contains(&map_core::features::FeatureRef::Route(id))
            || detail.visible(2)
    }

    pub(crate) fn draw_region_labels(
        &self,
        painter: &egui::Painter,
        vg: &ViewGeom,
        taken: &mut Vec<Rect>,
    ) {
        let detail = self.map_detail(vg);
        if !self.doc.style.show_labels || detail.opacity(1) == 0.0 {
            return;
        }
        let [r, g, b, a] = self.doc.style.label;
        let ink = Color32::from_rgba_unmultiplied(r, g, b, a);
        let halo = map_core::doc::halo_for(self.doc.style.label)[0];
        for region in &self.detail_cache.regions {
            let opacity = detail.opacity(region.rank);
            if opacity == 0.0 {
                continue;
            }
            let Some(entity) = self.doc.entities.get(&region.id) else {
                continue;
            };
            if entity.label_hidden || (self.tool == Tool::Label && entity.label.is_some()) {
                continue;
            }
            let Some(at) = vg
                .to_screen(entity.label.map_or(region.at, |label| label.at))
                .filter(|point| vg.rect.contains(*point))
            else {
                continue;
            };
            let tier = map_core::label_type::Tier::administrative(region.rank);
            let style = self.doc.style.label_type(tier);
            let scale = entity.label.map_or(1.0, |p| p.scale).clamp(0.55, 1.8);
            let font = egui::FontId::new(style.size * scale, self.font_family(style.font, style.bold));
            let text =
                painter.layout_no_wrap(entity.name.clone(), font, ink.gamma_multiply(opacity));
            let area = Rect::from_center_size(at, text.size()).expand(2.0);
            if !vg.rect.contains_rect(area) || taken.iter().any(|other| other.intersects(area)) {
                continue;
            }
            let pos = at - text.size() * 0.5;
            let shade = Color32::from_rgba_unmultiplied(halo, halo, halo, (180.0 * opacity) as u8);
            for offset in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                painter.add(
                    egui::epaint::TextShape::new(pos + offset, text.clone(), ink)
                        .with_override_text_color(shade),
                );
            }
            painter.galley(pos, text, ink);
            taken.push(area);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zoom_reveals_and_selects_successive_jurisdictions() {
        let mut doc = Document::default();
        let country = doc.add_entity(EntityKind::Country, "A");
        let province = doc.add_subdivision(country, "B").unwrap();
        let district = doc.add_subdivision(province, "C").unwrap();
        let town = doc.add_subdivision(district, "D").unwrap();
        for (zoom, expected) in [
            (1.0, country),
            (2.2, province),
            (4.2, district),
            (7.2, town),
        ] {
            assert_eq!(Detail(zoom).owner(&doc, town), expected);
        }
        assert_eq!(Detail(f64::INFINITY).owner(&doc, town), town);
        assert!(Detail(1.9).opacity(1) > 0.0 && !Detail(1.9).visible(1));
    }
}
