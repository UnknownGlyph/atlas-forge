//! The 要素 tab: every drawn feature listed by kind, with a filter; a
//! click selects one (switching to its tool for text and symbols), 定位
//! centres the map on it. Routes, rivers and areas are edited right here.

use crate::app::App;
use crate::view::Tool;
use eframe::egui::{self, RichText};
use map_core::features::{matches, FeatureRef};
use map_core::geom::LonLat;

impl App {
    pub fn features_tab(&mut self, ui: &mut egui::Ui) {
        // several picked: the group's panel; one: its own editor
        // each part in a scope of its own, there or not: the widgets after
        // it keep their ids, so the keyboard focus does not jump from a
        // field being typed in to some button when a part appears
        ui.push_id("selection", |ui| {
            if self.picked.len() >= 2 {
                self.selection_panel(ui);
            }
        });
        ui.push_id("line", |ui| self.river_panel(ui));
        ui.push_id("area", |ui| self.area_panel(ui));
        ui.separator();
        ui.push_id("layers", |ui| self.layers_section(ui));
        ui.label(RichText::new("要素").strong());
        ui.horizontal(|ui| {
            ui.label("筛选");
            ui.add(egui::TextEdit::singleline(&mut self.feature_filter).hint_text("名称、种类、年份或图层"));
            if self.active_group.is_some() {
                ui.checkbox(&mut self.layer_only, "只列当前图层");
            }
        });
        let all = self.doc.features();
        if all.is_empty() {
            ui.weak("还没有绘制要素：地图文字（标注 T）、符号（Y）、河流与线路（R）、斜线区域（H）都会列在这里。");
            return;
        }
        ui.weak("单击选中；Ctrl 或 Shift 单击多选；右键有更多操作。");
        let groups = self.doc.groups.clone();
        let layer_name = |g: Option<map_core::stack::GroupId>| g.and_then(|g| groups.iter().find(|x| x.id == g)).map(|x| x.name.as_str());
        let only = self.active_group.filter(|_| self.layer_only);
        let q = self.feature_filter.trim().to_string();
        let shown: Vec<_> = all
            .into_iter()
            .filter(|f| {
                let g = self.doc.group_of_feature(f.what);
                (only.is_none() || g == only) && (matches(f, &q) || (!q.is_empty() && layer_name(g).is_some_and(|n| n.contains(&q))))
            })
            .collect();
        // a feature moved to a layer (or out of them) from its menu
        let mut regroup: Option<(FeatureRef, Option<map_core::stack::GroupId>)> = None;
        if shown.is_empty() {
            ui.weak("没有符合筛选的要素。");
        }
        let selected: std::collections::BTreeSet<FeatureRef> = self.selection().into_iter().collect();
        let mut heading = "";
        let mut chosen: Option<(FeatureRef, Option<LonLat>)> = None;
        // Ctrl or Shift click: into or out of the selection
        let mut toggled: Option<FeatureRef> = None;
        let mut delete: Option<FeatureRef> = None;
        // (which, hide?) from the eye boxes and the per-kind buttons; (which, lock?)
        let mut visibility: Vec<(FeatureRef, bool)> = Vec::new();
        let mut locks: Vec<(FeatureRef, bool)> = Vec::new();
        for f in &shown {
            let kind = f.what.kind_name();
            if kind != heading {
                heading = kind;
                let of_kind: Vec<FeatureRef> = shown.iter().filter(|g| g.what.kind_name() == kind).map(|g| g.what).collect();
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{kind}（{}）", of_kind.len())).strong());
                    if ui.small_button("全部显示").clicked() {
                        visibility.extend(of_kind.iter().map(|w| (*w, false)));
                    }
                    if ui.small_button("全部隐藏").clicked() {
                        visibility.extend(of_kind.iter().map(|w| (*w, true)));
                    }
                    let all_locked = of_kind.iter().all(|w| self.doc.is_locked(*w));
                    if ui.small_button(if all_locked { "全部解锁" } else { "全部锁定" }).on_hover_text("锁定的要素仍然显示，但在地图上不会被选中或移动").clicked() {
                        locks.extend(of_kind.iter().map(|w| (*w, !all_locked)));
                    }
                });
            }
            let locked = self.doc.is_locked(f.what);
            ui.horizontal(|ui| {
                let mut on = !f.hidden;
                if ui.checkbox(&mut on, "").on_hover_text("显示：取消后不绘制、不导出、不进图例，但不删除").changed() {
                    visibility.push((f.what, !on));
                }
                let lock_text = if locked { RichText::new("锁") } else { RichText::new("锁").weak() };
                if ui.selectable_label(locked, lock_text).on_hover_text(if locked { "已锁定：仍然显示，但在地图上不会被选中或移动。单击解锁" } else { "单击锁定：仍然显示，但在地图上不会被选中或移动" }).clicked() {
                    locks.push((f.what, !locked));
                }
                let name = if f.name.is_empty() { "（未命名）".to_string() } else { f.name.clone() };
                let label = if f.detail.is_empty() { name } else { format!("{name} · {}", f.detail) };
                let text = if f.hidden { RichText::new(label).weak() } else { RichText::new(label) };
                let r = ui.selectable_label(selected.contains(&f.what), text);
                if r.clicked() {
                    // as the button went down (a quick click may have let the key go by now)
                    let m = self.press_mods;
                    if m.shift || m.command {
                        toggled = Some(f.what);
                    } else {
                        chosen = Some((f.what, None));
                    }
                }
                r.context_menu(|ui| {
                    if ui.button("定位").clicked() {
                        chosen = Some((f.what, Some(f.at)));
                    }
                    if ui.button(if f.hidden { "显示" } else { "隐藏" }).clicked() {
                        visibility.push((f.what, !f.hidden));
                    }
                    if ui.button(if locked { "解锁" } else { "锁定" }).clicked() {
                        locks.push((f.what, !locked));
                    }
                    if !groups.is_empty() {
                        let cur = self.doc.group_of_feature(f.what);
                        ui.menu_button("移到图层", |ui| {
                            for g in groups.iter().rev() {
                                if ui.add_enabled(cur != Some(g.id), egui::Button::new(&g.name)).clicked() {
                                    regroup = Some((f.what, Some(g.id)));
                                }
                            }
                            if cur.is_some() {
                                ui.separator();
                                if ui.button("（移出图层）").clicked() {
                                    regroup = Some((f.what, None));
                                }
                            }
                        });
                    }
                    ui.separator();
                    if ui.button("删除").clicked() {
                        delete = Some(f.what);
                    }
                });
                if !f.years.is_empty() {
                    ui.weak(&f.years);
                }
                if let Some(n) = layer_name(self.doc.group_of_feature(f.what)) {
                    ui.weak(format!("〔{n}〕"));
                }
                if ui.small_button("定位").on_hover_text("选中并把地图中心移到这里").clicked() {
                    chosen = Some((f.what, Some(f.at)));
                }
            });
        }
        if !visibility.is_empty() {
            let hide = visibility.iter().all(|v| v.1);
            let label = match (visibility.len(), hide) {
                (1, true) => "隐藏要素",
                (1, false) => "显示要素",
                (_, true) => "隐藏一类要素",
                (_, false) => "显示一类要素",
            };
            self.edit(label, |d| {
                for (w, h) in &visibility {
                    d.set_hidden(*w, *h)?;
                }
                Ok(())
            });
        }
        if !locks.is_empty() {
            let lock = locks.iter().all(|l| l.1);
            self.edit(if lock { "锁定要素" } else { "解锁要素" }, |d| locks.iter().try_for_each(|(w, l)| d.set_locked(*w, *l)));
            if lock {
                // what is locked is no longer selected
                let keep: Vec<FeatureRef> = self.selection().into_iter().filter(|f| !self.doc.is_locked(*f)).collect();
                if keep.len() != self.selection().len() {
                    self.set_selection(keep);
                }
            }
        }
        if let Some((what, g)) = regroup {
            self.edit(if g.is_some() { "移入图层" } else { "移出图层" }, |d| d.set_group(&[what], g));
        }
        if let Some(what) = delete {
            self.edit("删除要素", |d| {
                d.delete_features(&[what]);
                Ok(())
            });
        }
        if let Some(what) = toggled {
            // with the select tool, as a Shift-click on the map would
            let mut sel = self.selection();
            match sel.iter().position(|f| *f == what) {
                Some(i) => {
                    sel.remove(i);
                }
                None => sel.push(what),
            }
            self.set_tool(Tool::Select);
            let n = sel.len();
            self.set_selection(sel);
            self.set_status(format!("已选择 {n} 个要素"), false);
        }
        if let Some((what, at)) = chosen {
            self.picked.clear();
            self.select_feature(what);
            if let Some(p) = at {
                self.centre_on(p);
            }
        }
    }

    /// Select a drawn feature: text and symbols with their tools (whose
    /// panels edit them), lines and areas in place.
    pub fn select_feature(&mut self, what: FeatureRef) {
        self.selected = None;
        self.selected_face = None;
        self.selected_river = None;
        self.selected_area = None;
        match what {
            FeatureRef::Text(id) => {
                self.set_tool(Tool::Label);
                self.selected_ann = Some(id);
            }
            FeatureRef::Symbol(id) => {
                self.set_tool(Tool::Symbol);
                self.selected_sym = Some(id);
            }
            FeatureRef::Route(id) | FeatureRef::River(id) => self.selected_river = Some(id),
            FeatureRef::Area(id) => self.selected_area = Some(id),
        }
    }

    /// Centre both views on a point, keeping their zoom.
    pub fn centre_on(&mut self, p: LonLat) {
        self.cam2d.centre_on(p);
        self.globe.center_lon = p.lon;
        self.globe.center_lat = p.lat.clamp(-89.0, 89.0);
        self.nav_touched = true;
    }
}
