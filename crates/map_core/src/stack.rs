//! Layers of drawn features (the "图层" of the interface; called groups
//! here, since `layers` already means the partition layers of countries,
//! cultures and so on).
//!
//! A group is a named set of drawn features: map text, symbols, rivers
//! and routes, hatched areas. It can be hidden, locked and made partly
//! see-through as a whole, and the groups are stacked: features that are in
//! no group are drawn first, as they always were, then each group in
//! order, the last on top. A feature is in at most one group. A document
//! without groups is drawn exactly as before there were any.

use crate::doc::{Document, LineId};
use crate::features::FeatureRef;
use crate::topo::TResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct GroupId(pub u64);

fn full() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FeatureGroup {
    pub id: GroupId,
    pub name: String,
    /// Not drawn, exported or in the legend; its features cannot be picked.
    #[serde(default)]
    pub hidden: bool,
    /// Drawn, but its features cannot be picked or moved on the map.
    #[serde(default)]
    pub locked: bool,
    /// 0 (invisible) to 1 (as drawn).
    #[serde(default = "full")]
    pub opacity: f32,
}

/// One step of drawing the features: those in no group (`group` is None),
/// or those of one group, with the group's opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DrawPass {
    pub group: Option<GroupId>,
    pub opacity: f32,
}

impl Document {
    pub fn group(&self, g: GroupId) -> Option<&FeatureGroup> {
        self.groups.iter().find(|x| x.id == g)
    }

    fn group_mut(&mut self, g: GroupId) -> TResult<&mut FeatureGroup> {
        self.groups.iter_mut().find(|x| x.id == g).ok_or_else(|| "图层不存在".to_string())
    }

    /// A new, empty group on top of the others.
    pub fn add_group(&mut self, name: &str) -> GroupId {
        let id = GroupId(self.topo.alloc());
        let name = if name.trim().is_empty() { format!("图层 {}", self.groups.len() + 1) } else { name.trim().to_string() };
        self.groups.push(FeatureGroup { id, name, hidden: false, locked: false, opacity: 1.0 });
        id
    }

    pub fn rename_group(&mut self, g: GroupId, name: &str) -> TResult<()> {
        if name.trim().is_empty() {
            return Err("图层名称不能为空".into());
        }
        self.group_mut(g)?.name = name.trim().to_string();
        Ok(())
    }

    pub fn set_group_hidden(&mut self, g: GroupId, hidden: bool) -> TResult<()> {
        self.group_mut(g)?.hidden = hidden;
        Ok(())
    }

    pub fn set_group_locked(&mut self, g: GroupId, locked: bool) -> TResult<()> {
        self.group_mut(g)?.locked = locked;
        Ok(())
    }

    pub fn set_group_opacity(&mut self, g: GroupId, opacity: f32) -> TResult<()> {
        if !opacity.is_finite() {
            return Err("不透明度不是有效的数".into());
        }
        self.group_mut(g)?.opacity = opacity.clamp(0.0, 1.0);
        Ok(())
    }

    /// Remove a group. Its features stay on the map, in no group. Returns
    /// how many there were.
    pub fn remove_group(&mut self, g: GroupId) -> TResult<usize> {
        let i = self.groups.iter().position(|x| x.id == g).ok_or("图层不存在")?;
        self.groups.remove(i);
        let members: Vec<_> = self.group_of.iter().filter_map(|(id, group)| (*group == g).then_some(*id)).collect();
        for id in &members {
            self.group_of.remove(id);
        }
        Ok(members.len())
    }

    /// Move a group to place `to` in the stack (0 is the bottom).
    pub fn move_group(&mut self, g: GroupId, to: usize) -> TResult<()> {
        let i = self.groups.iter().position(|x| x.id == g).ok_or("图层不存在")?;
        let x = self.groups.remove(i);
        self.groups.insert(to.min(self.groups.len()), x);
        Ok(())
    }

    /// Put features into a group, or (None) take them out of theirs.
    pub fn set_group(&mut self, fs: &[FeatureRef], g: Option<GroupId>) -> TResult<()> {
        if let Some(g) = g {
            self.group(g).ok_or("图层不存在")?;
        }
        if let Some(f) = fs.iter().find(|f| !self.feature_exists(**f)) {
            return Err(format!("{}不存在", f.kind_name()));
        }
        for f in fs {
            match g {
                Some(g) => self.group_of.insert(f.id(), g),
                None => self.group_of.remove(&f.id()),
            };
        }
        Ok(())
    }

    /// The group a feature (by its id) is in.
    pub fn group_of_id(&self, id: LineId) -> Option<GroupId> {
        // (a group that is gone holds nothing: its features are in no group)
        self.group_of.get(&id).copied().filter(|g| self.group(*g).is_some())
    }

    pub fn group_of_feature(&self, f: FeatureRef) -> Option<GroupId> {
        self.group_of_id(f.id())
    }

    /// The features of a group (or, with None, those in no group), by kind.
    pub fn group_members(&self, g: Option<GroupId>) -> Vec<FeatureRef> {
        self.features().into_iter().map(|f| f.what).filter(|f| self.group_of_feature(*f) == g).collect()
    }

    /// Is the feature's group hidden?
    pub fn group_hides(&self, id: LineId) -> bool {
        self.group_of_id(id).and_then(|g| self.group(g)).is_some_and(|g| g.hidden)
    }

    /// The opacity the feature's group gives it (1 in no group).
    pub fn group_opacity(&self, id: LineId) -> f32 {
        self.group_of_id(id).and_then(|g| self.group(g)).map_or(1.0, |g| g.opacity)
    }

    /// Hidden itself, or in a hidden group.
    pub fn hidden_in_effect(&self, f: FeatureRef) -> bool {
        let own = match f {
            FeatureRef::Text(id) => self.annotations.get(&id).is_some_and(|x| x.hidden),
            FeatureRef::Symbol(id) => self.symbols.get(&id).is_some_and(|x| x.hidden),
            FeatureRef::Route(id) | FeatureRef::River(id) => self.rivers.get(&id).is_some_and(|x| x.hidden),
            FeatureRef::Area(id) => self.areas.get(&id).is_some_and(|x| x.hidden),
        };
        own || self.group_hides(f.id())
    }

    /// Locked itself, or in a locked group.
    pub fn locked_in_effect(&self, f: FeatureRef) -> bool {
        self.is_locked(f) || self.group_of_feature(f).and_then(|g| self.group(g)).is_some_and(|g| g.locked)
    }

    /// Can the feature be picked on the map: there, shown and not locked.
    pub fn pickable(&self, f: FeatureRef) -> bool {
        self.feature_exists(f) && !self.hidden_in_effect(f) && !self.locked_in_effect(f)
    }

    /// The steps the features are drawn in: those in no group, then each
    /// group that is shown, from the bottom of the stack up.
    pub fn draw_passes(&self) -> Vec<DrawPass> {
        std::iter::once(DrawPass { group: None, opacity: 1.0 }).chain(self.groups.iter().filter(|g| !g.hidden).map(|g| DrawPass { group: Some(g.id), opacity: g.opacity })).collect()
    }

    /// The ids of every drawn feature.
    pub fn feature_ids(&self) -> std::collections::BTreeSet<LineId> {
        self.annotations.keys().chain(self.symbols.keys()).chain(self.rivers.keys()).chain(self.areas.keys()).copied().collect()
    }

    /// Forget the groups of features that are gone (they are not removed
    /// one by one where features are deleted).
    pub fn prune_groups(&mut self) {
        let gone: Vec<LineId> = self
            .group_of
            .iter()
            .filter(|(id, g)| self.group(**g).is_none() || !(self.annotations.contains_key(*id) || self.symbols.contains_key(*id) || self.rivers.contains_key(*id) || self.areas.contains_key(*id)))
            .map(|(id, _)| *id)
            .collect();
        for id in gone {
            self.group_of.remove(&id);
        }
    }
}
