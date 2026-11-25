//! Ordered feature IDs for the canvas passes shared by the flat and globe views.
use imbl::OrdMap;
use map_core::area::AreaMark;
use map_core::doc::{Annotation, LineFeature, LineId};
use map_core::stack::{DrawPass, FeatureGroup, GroupId};
use map_core::symbol::MapSymbol;
use map_core::Document;
use std::collections::HashMap;

pub(crate) struct LayerFeatures {
    pub pass: DrawPass,
    pub areas: Vec<LineId>,
    pub rivers: Vec<LineId>,
    pub annotations: Vec<LineId>,
    pub symbols: Vec<LineId>,
}

#[derive(Default)]
pub(crate) struct DrawLayers {
    groups: Vec<FeatureGroup>,
    membership: OrdMap<LineId, GroupId>,
    rivers: OrdMap<LineId, LineFeature>,
    areas: OrdMap<LineId, AreaMark>,
    annotations: OrdMap<LineId, Annotation>,
    symbols: OrdMap<LineId, MapSymbol>,
    layers: Vec<LayerFeatures>,
}

impl DrawLayers {
    pub fn for_document(&mut self, doc: &Document) -> &[LayerFeatures] {
        if !self.layers.is_empty() && self.groups == doc.groups && self.membership.ptr_eq(&doc.group_of)
            && self.rivers.ptr_eq(&doc.rivers) && self.areas.ptr_eq(&doc.areas)
            && self.annotations.ptr_eq(&doc.annotations) && self.symbols.ptr_eq(&doc.symbols)
        {
            return &self.layers;
        }

        self.layers = doc.draw_passes().into_iter().map(|pass| LayerFeatures {
            pass, areas: vec![], rivers: vec![], annotations: vec![], symbols: vec![],
        }).collect();
        // A missing group falls back to the base pass. A hidden group has no pass.
        let mut slots: HashMap<_, _> = doc.groups.iter().map(|group| (group.id, None)).collect();
        for (slot, layer) in self.layers.iter().enumerate() {
            if let Some(group) = layer.pass.group {
                slots.insert(group, Some(slot));
            }
        }
        let slot_for = |id: &LineId| doc.group_of.get(id).and_then(|g| slots.get(g)).copied().unwrap_or(Some(0));
        for id in doc.areas.keys() {
            if let Some(slot) = slot_for(id) { self.layers[slot].areas.push(*id); }
        }
        for id in doc.rivers.keys() {
            if let Some(slot) = slot_for(id) { self.layers[slot].rivers.push(*id); }
        }
        for id in doc.annotations.keys() {
            if let Some(slot) = slot_for(id) { self.layers[slot].annotations.push(*id); }
        }
        for id in doc.symbols.keys() {
            if let Some(slot) = slot_for(id) { self.layers[slot].symbols.push(*id); }
        }
        self.groups.clone_from(&doc.groups);
        self.membership = doc.group_of.clone();
        self.rivers = doc.rivers.clone();
        self.areas = doc.areas.clone();
        self.annotations = doc.annotations.clone();
        self.symbols = doc.symbols.clone();
        &self.layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_core::{doc::Annotation, stack::GroupId, LonLat};

    #[test]
    fn passes_track_membership_and_visibility_without_losing_ungrouped_features() {
        let mut doc = Document::default();
        let a = doc.add_group("A");
        let b = doc.add_group("B");
        for id in [LineId(301), LineId(302), LineId(303)] {
            doc.annotations.insert(id, Annotation::new("label", LonLat::new(0.0, 1.0)));
        }
        doc.group_of.insert(LineId(301), b);
        doc.group_of.insert(LineId(302), GroupId(999));
        doc.group_of.insert(LineId(303), a);
        let mut cache = DrawLayers::default();
        let before = doc.clone();
        let check = |cache: &mut DrawLayers, doc: &Document| {
            let expected = doc.draw_passes();
            let layers = cache.for_document(doc);
            assert_eq!(layers.len(), expected.len());
            for (layer, pass) in layers.iter().zip(expected) {
                assert_eq!(layer.pass, pass);
                assert_eq!(layer.annotations, doc.annotations.keys().copied()
                    .filter(|id| doc.group_of_id(*id) == pass.group).collect::<Vec<_>>());
            }
        };
        check(&mut cache, &doc);
        doc.group_of.insert(LineId(301), a);
        doc.groups.reverse();
        doc.groups[0].opacity = 0.4;
        doc.groups[1].hidden = true;
        check(&mut cache, &doc);
        check(&mut cache, &before);
        check(&mut cache, &Document::default());
    }
}
