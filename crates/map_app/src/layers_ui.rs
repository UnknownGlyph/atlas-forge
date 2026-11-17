//! The layers panel (top of the 图层 tab): the stack of layers of drawn
//! features, the top one first. A layer is shown or hidden, locked and
//! made see-through as a whole, moved up and down the stack, renamed and
//! removed here. One layer can be the layer in hand: what is drawn or
//! pasted from then on goes into it. The model is `map_core::stack`.

use crate::app::App;
use crate::panels::Tab;
use crate::view::Tool;
use eframe::egui::{self, RichText};
use map_core::doc::LineId;
use map_core::features::FeatureRef;
use map_core::stack::GroupId;
use std::collections::{BTreeSet, HashMap};

const OPACITY: &str = "图层不透明度";

enum Op {
    New,
    FromSelection,
    Hide(GroupId, bool),
    Lock(GroupId, bool),
    Opacity(GroupId, f32),
    OpacityDone,
    Move(GroupId, usize),
    Remove(GroupId),
    Rename(GroupId, String),
    InHand(Option<GroupId>),
    TakeSelection(GroupId),
    SelectMembers(GroupId),
}

impl App {
    /// After an edit: drawn features that were not there before go into
    /// the layer in hand (unless they already have one, as copies made
    /// beside their originals do).
    pub fn adopt_new_features(&mut self, had: &BTreeSet<LineId>) {
        let Some(g) = self.active_group.filter(|g| self.doc.group(*g).is_some()) else { return };
        for id in self.doc.feature_ids().difference(had) {
            self.doc.group_of.entry(*id).or_insert(g);
        }
    }

    /// Features that can no longer be picked (their layer was hidden or
    /// locked) leave the selection.
    fn drop_unpickable(&mut self) {
        let sel = self.selection();
        let keep: Vec<FeatureRef> = sel.iter().copied().filter(|f| self.doc.pickable(*f)).collect();
        if keep.len() != sel.len() {
            self.set_selection(keep);
            self.selected_sym = self.selected_sym.filter(|id| self.doc.pickable(FeatureRef::Symbol(*id)));
            self.selected_ann = self.selected_ann.filter(|id| self.doc.pickable(FeatureRef::Text(*id)));
        }
    }

    /// Ctrl+G: a new layer on top with the selected features in it (an
    /// empty one when nothing is selected); it becomes the layer in hand.
    pub fn group_selection(&mut self) {
        let fs = self.selection();
        let label = if fs.is_empty() { "新建图层" } else { "所选建为图层" };
        let made = self.edit(label, |d| {
            let g = d.add_group("");
            d.set_group(&fs, Some(g))?;
            Ok(g)
        });
        if let Some(g) = made {
            self.active_group = Some(g);
            self.tab = Tab::Features;
            let name = self.doc.group(g).map(|x| x.name.clone()).unwrap_or_default();
            self.set_status(format!("已新建“{name}”{}；它是当前图层，新画的要素会进入它", if fs.is_empty() { String::new() } else { format!("，移入 {} 个要素", fs.len()) }), false);
            self.group_rename = Some((g, name, true));
        }
    }

    /// Ctrl+Shift+G: the selected features leave their layers.
    pub fn ungroup_selection(&mut self) {
        let fs = self.selection();
        if self.edit("移出图层", |d| d.set_group(&fs, None)).is_some() {
            self.set_status(format!("{} 个要素已移出图层（回到最下面，按原来的顺序绘制）", fs.len()), false);
        }
    }

    /// Whether any selected feature is in a layer.
    pub fn selection_in_layers(&self) -> bool {
        self.selection().iter().any(|f| self.doc.group_of_feature(*f).is_some())
    }

    pub fn layers_section(&mut self, ui: &mut egui::Ui) {
        if self.active_group.is_some_and(|g| self.doc.group(g).is_none()) {
            self.active_group = None;
        }
        let groups = self.doc.groups.clone();
        let mut count: HashMap<Option<GroupId>, usize> = HashMap::new();
        for f in self.doc.features() {
            *count.entry(self.doc.group_of_feature(f.what)).or_default() += 1;
        }
        let sel = self.selection();
        let mut rename = self.group_rename.take();
        let mut ops: Vec<Op> = Vec::new();
        ui.horizontal(|ui| {
            ui.label(RichText::new("图层").strong());
            if ui.small_button("＋ 新建").on_hover_text("在最上面新建一个空图层，并设为当前图层：之后新画、粘贴的要素进入它").clicked() {
                ops.push(Op::New);
            }
            if ui.add_enabled(!sel.is_empty(), egui::Button::new("所选建为图层").small()).on_hover_text("Ctrl+G：新建图层，把所选要素移入").clicked() {
                ops.push(Op::FromSelection);
            }
        });
        if groups.is_empty() {
            ui.weak("图层把地图上的文字、符号、线和区域分成几组：整组显示、隐藏、锁定、调不透明度，上面的图层盖住下面的。现在还没有图层，全部要素按原来的顺序绘制。");
        }
        let n = groups.len();
        // the top of the stack first
        for (i, g) in groups.iter().enumerate().rev() {
            // (ids by layer, not by place in the list)
            ui.push_id(g.id.0, |ui| ui.horizontal(|ui| {
                let mut on = !g.hidden;
                if ui.checkbox(&mut on, "").on_hover_text("显示整个图层；取消后其中的要素不绘制、不导出、不进图例").changed() {
                    ops.push(Op::Hide(g.id, !on));
                }
                let lock = if g.locked { RichText::new("锁") } else { RichText::new("锁").weak() };
                if ui.selectable_label(g.locked, lock).on_hover_text(if g.locked { "已锁定：其中的要素不会在地图上被选中或移动。单击解锁" } else { "单击锁定整个图层" }).clicked() {
                    ops.push(Op::Lock(g.id, !g.locked));
                }
                match &mut rename {
                    Some((id, text, focus)) if *id == g.id => {
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(120.0));
                        if *focus {
                            r.request_focus();
                            *focus = false;
                        } else if r.lost_focus() {
                            ops.push(Op::Rename(g.id, text.clone()));
                        }
                    }
                    _ => {
                        let in_hand = self.active_group == Some(g.id);
                        let name = if g.hidden { RichText::new(&g.name).weak() } else { RichText::new(&g.name) };
                        let r = ui.selectable_label(in_hand, name).on_hover_text("单击设为当前图层（新画、粘贴的要素进入它），再单击取消；双击改名；右键有更多操作");
                        if r.double_clicked() {
                            rename = Some((g.id, g.name.clone(), true));
                        } else if r.clicked() {
                            ops.push(Op::InHand((!in_hand).then_some(g.id)));
                        }
                        r.context_menu(|ui| {
                            if ui.button("改名").clicked() {
                                rename = Some((g.id, g.name.clone(), true));
                            }
                            if ui.add_enabled(!sel.is_empty(), egui::Button::new(format!("所选要素移入（{}）", sel.len()))).clicked() {
                                ops.push(Op::TakeSelection(g.id));
                            }
                            if ui.button("选中其中的要素").clicked() {
                                ops.push(Op::SelectMembers(g.id));
                            }
                            ui.separator();
                            if ui.add_enabled(i + 1 < n, egui::Button::new("移到最上")).clicked() {
                                ops.push(Op::Move(g.id, n - 1));
                            }
                            if ui.add_enabled(i > 0, egui::Button::new("移到最下")).clicked() {
                                ops.push(Op::Move(g.id, 0));
                            }
                            ui.separator();
                            if ui.button("删除图层（要素保留）").clicked() {
                                ops.push(Op::Remove(g.id));
                            }
                        });
                    }
                }
                ui.weak(format!("{} 个", count.get(&Some(g.id)).copied().unwrap_or(0)));
                let mut pct = (g.opacity * 100.0).round();
                let r = ui.add(egui::DragValue::new(&mut pct).range(0.0..=100.0).speed(1.0).suffix("%")).on_hover_text("不透明度：整层要素一起变淡，导出同样生效");
                if r.changed() {
                    ops.push(Op::Opacity(g.id, pct / 100.0));
                }
                if r.drag_stopped() || r.lost_focus() {
                    ops.push(Op::OpacityDone);
                }
                if ui.add_enabled(i + 1 < n, egui::Button::new("▲").small()).on_hover_text("上移一层：盖住原来在它上面的那一层").clicked() {
                    ops.push(Op::Move(g.id, i + 1));
                }
                if ui.add_enabled(i > 0, egui::Button::new("▼").small()).on_hover_text("下移一层").clicked() {
                    ops.push(Op::Move(g.id, i - 1));
                }
            }));
        }
        // what lies under the layers
        if !groups.is_empty() {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.weak(format!("未分组的要素 {} 个：在所有图层之下，按原来的顺序绘制", count.get(&None).copied().unwrap_or(0)));
            });
        }
        ui.horizontal_wrapped(|ui| {
            ui.weak("底图");
            ui.checkbox(&mut self.doc.style.show_labels, "国家名称");
            ui.checkbox(&mut self.doc.style.show_cities, "城市");
            ui.checkbox(&mut self.graticule, "经纬网");
        });
        self.group_rename = rename;
        for op in ops {
            match op {
                Op::New => {
                    if let Some(g) = self.edit("新建图层", |d| Ok(d.add_group(""))) {
                        self.active_group = Some(g);
                        let name = self.doc.group(g).map(|x| x.name.clone()).unwrap_or_default();
                        self.set_status(format!("已新建“{name}”：它是当前图层，新画的要素会进入它"), false);
                        self.group_rename = Some((g, name, true));
                    }
                }
                Op::FromSelection => self.group_selection(),
                Op::Hide(g, hide) => {
                    self.edit(if hide { "隐藏图层" } else { "显示图层" }, |d| d.set_group_hidden(g, hide));
                    self.drop_unpickable();
                }
                Op::Lock(g, lock) => {
                    self.edit(if lock { "锁定图层" } else { "解锁图层" }, |d| d.set_group_locked(g, lock));
                    self.drop_unpickable();
                }
                // one undo step for a whole drag of the number
                Op::Opacity(g, v) => {
                    if self.pending.is_none() {
                        self.begin_pending(OPACITY);
                    }
                    if self.doc.set_group_opacity(g, v).is_ok() {
                        if let Some(p) = &mut self.pending {
                            p.changed = true;
                        }
                    }
                }
                Op::OpacityDone => {
                    if self.pending.as_ref().is_some_and(|p| p.label == OPACITY) {
                        self.end_pending();
                    }
                }
                Op::Move(g, to) => {
                    self.edit("调整图层顺序", |d| d.move_group(g, to));
                }
                Op::Remove(g) => {
                    if let Some(n) = self.edit("删除图层", |d| d.remove_group(g)) {
                        self.set_status(format!("图层已删除，其中的 {n} 个要素保留在地图上（Ctrl+Z 撤销）"), false);
                    }
                    self.group_rename = None;
                }
                Op::Rename(g, name) => {
                    self.group_rename = None;
                    let name = name.trim().to_string();
                    if !name.is_empty() && self.doc.group(g).is_some_and(|x| x.name != name) {
                        self.edit("图层改名", |d| d.rename_group(g, &name));
                    }
                }
                Op::InHand(g) => {
                    self.active_group = g;
                    match g.and_then(|g| self.doc.group(g)) {
                        Some(x) => self.set_status(format!("当前图层：{}。新画、粘贴的要素进入它", x.name), false),
                        None => self.set_status("没有当前图层：新画的要素不进入任何图层", false),
                    }
                }
                Op::TakeSelection(g) => {
                    let fs = self.selection();
                    if self.edit("移入图层", |d| d.set_group(&fs, Some(g))).is_some() {
                        self.drop_unpickable();
                    }
                }
                Op::SelectMembers(g) => {
                    let fs: Vec<FeatureRef> = self.doc.group_members(Some(g)).into_iter().filter(|f| self.doc.pickable(*f)).collect();
                    let n = fs.len();
                    self.set_tool(Tool::Select);
                    self.set_selection(fs);
                    self.set_status(format!("已选择图层里的 {n} 个要素（隐藏或锁定的不选）"), false);
                }
            }
        }
        ui.separator();
    }
}
