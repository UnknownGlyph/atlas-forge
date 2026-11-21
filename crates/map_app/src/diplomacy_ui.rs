use crate::app::App;
use crate::view::ViewGeom;
use eframe::egui;
use map_core::{
    diplomacy::{Agreement, Kind},
    doc::{EntityKind, POLITICAL},
    world_book::{Node, WorldBook},
};

#[derive(Default)]
pub(crate) struct DiplomacyUi {
    pub open: bool,
    pub lines: bool,
    edit: Option<(Option<u64>, Agreement)>,
    delete: Option<u64>,
    search: String,
    all_years: bool,
}

fn date(ui: &mut egui::Ui, value: &mut Option<i32>, label: &str, hint: i32) {
    let mut enabled = value.is_some();
    if ui.checkbox(&mut enabled, label).changed() {
        *value = enabled.then_some(hint);
    }
    if let Some(year) = value {
        ui.add(egui::DragValue::new(year).range(-1_000_000..=1_000_000));
    }
}

impl App {
    pub(crate) fn request_diplomacy_delete(&mut self) {
        if let Some((Some(id), _)) = &self.diplomacy_ui.edit {
            self.diplomacy_ui.delete = Some(*id);
        }
    }

    pub(crate) fn diplomacy_window(&mut self, ctx: &egui::Context) {
        if !self.diplomacy_ui.open {
            return;
        }
        let doc = &self.doc;
        let state = &mut self.diplomacy_ui;
        let mut open = true;
        let mut save = None;
        let mut remove = None;
        let mut related_event = None;
        egui::Window::new(crate::i18n::tr("外交关系"))
            .id(egui::Id::unique("atlas-diplomacy"))
            .open(&mut open)
            .default_width(585.0)
            .scroll(true)
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.button(crate::i18n::tr("新建关系或协约")).clicked() {
                        let mut record = Agreement {
                            from: doc.year,
                            ..Default::default()
                        };
                        if let Some(id) = self
                            .selected
                            .and_then(|id| doc.entities.get(&id).map(|ent| (id, ent)))
                            .filter(|(_, ent)| {
                                ent.layer == POLITICAL
                                    && ent.kind == EntityKind::Country
                                    && ent.parent.is_none()
                            })
                            .map(|p| p.0)
                        {
                            record.parties.insert(id);
                        }
                        state.edit = Some((None, record));
                        state.search.clear();
                    }
                    ui.checkbox(&mut state.all_years, crate::i18n::tr("浏览所有年份"));
                    ui.checkbox(&mut state.lines, crate::i18n::tr("地图连线"));
                });
                let year = if state.all_years { None } else { doc.year };
                if let Some(d) = &doc.diplomacy {
                    egui::ScrollArea::vertical()
                        .max_height(185.0)
                        .id_salt("diplomatic-list")
                        .show(ui, |ui| {
                            for (id, record) in d.records.iter().filter(|(_, r)| r.visible(year)) {
                                ui.group(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        let [r, g, b] = record.kind.color();
                                        ui.colored_label(
                                            egui::Color32::from_rgb(r, g, b),
                                            record.kind.name(),
                                        );
                                        ui.strong(&record.name);
                                        if ui.small_button(crate::i18n::tr("编辑")).clicked() {
                                            state.edit = Some((Some(*id), record.clone()));
                                            state.search.clear();
                                        }
                                        if record.event.is_some()
                                            && ui.small_button(crate::i18n::tr("关联事件")).clicked()
                                        {
                                            related_event = record.event;
                                        }
                                        if ui.small_button(crate::i18n::tr("删除…")).clicked() {
                                            state.delete = Some(*id);
                                        }
                                    });
                                    ui.label(
                                        record
                                            .parties
                                            .iter()
                                            .map(|&party| doc.diplomatic_name(party))
                                            .collect::<Vec<_>>()
                                            .join("、"),
                                    );
                                    if record.from.is_some() || record.until.is_some() {
                                        let fmt = |year: Option<i32>| {
                                            year.map_or("不限".into(), |y| doc.calendar.show(y))
                                        };
                                        ui.weak(crate::trf!(
                                            "{} 至 {}",
                                            fmt(record.from),
                                            fmt(record.until)
                                        ));
                                    }
                                    if !record.note.is_empty() {
                                        ui.label(&record.note);
                                    }
                                    if state.delete == Some(*id) {
                                        ui.horizontal(|ui| {
                                            if ui.button(crate::i18n::tr("确认删除关系")).clicked() {
                                                remove = Some(*id);
                                                state.delete = None;
                                            }
                                            if ui.button(crate::i18n::tr("取消")).clicked() {
                                                state.delete = None;
                                            }
                                        });
                                    }
                                });
                            }
                        });
                }
                let mut cancel = false;
                if let Some((id, record)) = &mut state.edit {
                    ui.separator();
                    ui.label(crate::i18n::tr("名称"));
                    ui.text_edit_singleline(&mut record.name);
                    ui.horizontal_wrapped(|ui| {
                        egui::ComboBox::from_id_salt("diplomatic-kind")
                            .selected_text(record.kind.name())
                            .show_ui(ui, |ui| {
                                for kind in Kind::ALL {
                                    ui.selectable_value(&mut record.kind, kind, kind.name());
                                }
                            });
                        date(ui, &mut record.from, "起始年", doc.year.unwrap_or(0));
                        date(ui, &mut record.until, "结束年", doc.year.unwrap_or(0));
                    });
                    ui.horizontal(|ui| {
                        ui.label(crate::trf!("参与国家（已选 {} 个）", record.parties.len()));
                        ui.add(egui::TextEdit::singleline(&mut state.search).hint_text(crate::i18n::tr("搜索国家")));
                    });
                    let search = state.search.to_lowercase();
                    egui::ScrollArea::vertical()
                        .max_height(150.0)
                        .id_salt("diplomatic-parties")
                        .show(ui, |ui| {
                            for (id, e) in doc.entities.iter() {
                                if !record.parties.contains(id)
                                    && !(e.kind == EntityKind::Country
                                        && e.layer == POLITICAL
                                        && e.parent.is_none())
                                {
                                    continue;
                                }
                                if !e.name.to_lowercase().contains(&search) {
                                    continue;
                                }
                                let mut enabled = record.parties.contains(id);
                                if ui.checkbox(&mut enabled, &e.name).changed() {
                                    if enabled {
                                        record.parties.insert(*id);
                                    } else {
                                        record.parties.remove(id);
                                    }
                                }
                            }
                        });
                    egui::ComboBox::from_id_salt("diplomatic-event")
                        .selected_text(
                            record
                                .event
                                .and_then(|id| doc.event(id))
                                .map_or("不关联事件", |e| e.name.as_str()),
                        )
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut record.event, None, crate::i18n::tr("不关联事件"));
                            for event in doc.events_in_order() {
                                ui.selectable_value(&mut record.event, Some(event.id), &event.name);
                            }
                        });
                    ui.label(crate::i18n::tr("说明或条约内容"));
                    ui.add(
                        egui::TextEdit::multiline(&mut record.note)
                            .desired_rows(3)
                            .desired_width(480.),
                    );
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("保存关系")).clicked() {
                            save = Some((*id, record.clone()));
                        }
                        if ui.button(crate::i18n::tr("取消编辑")).clicked() {
                            cancel = true;
                        }
                    });
                }
                if cancel {
                    state.edit = None;
                }
            });
        self.diplomacy_ui.open = open;
        if let Some((id, record)) = save {
            if self
                .edit("保存外交关系", |doc| doc.save_agreement(id, record))
                .is_some()
            {
                self.diplomacy_ui.edit = None;
            }
        }
        if let Some(id) = remove {
            if self
                .edit("删除外交关系", |doc| doc.remove_agreement(id))
                .is_some()
            {
                self.diplomacy_ui.edit = None;
            }
        }
        if let Some(event) = related_event {
            self.book_context(Node::Event(WorldBook::current(&self.doc), event));
        }
    }

    pub(crate) fn draw_diplomatic_lines(&self, painter: &egui::Painter, vg: &ViewGeom) {
        if !self.diplomacy_ui.lines || self.doc.active_layer != POLITICAL {
            return;
        }
        let Some(diplomacy) = self.doc.diplomacy.as_ref() else {
            return;
        };
        for agreement in diplomacy
            .records
            .values()
            .filter(|a| a.visible(self.doc.year))
        {
            let mut points = agreement.parties.iter().filter_map(|id| {
                let path = self.labels.iter().find(|(unit, _)| unit == id)?.1.middle();
                vg.to_screen(path)
            });
            let Some(first) = points.next() else {
                continue;
            };
            let [r, g, b] = agreement.kind.color();
            let color = egui::Color32::from_rgb(r, g, b);
            for end in points {
                if (end.x - first.x).abs() > vg.rect.width() * 0.6 {
                    continue;
                }
                painter.extend(egui::Shape::dashed_line(
                    &[first, end],
                    egui::Stroke::new(2.1, color),
                    6.0,
                    4.0,
                ));
                painter.circle_filled(end, 3.5, color);
            }
            painter.circle_filled(first, 3.5, color);
        }
    }
}
