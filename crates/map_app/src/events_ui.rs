//! The event timeline in the interface (`map_core::events`): a window that
//! lists the events in order (in the world's calendar) and edits the one
//! picked — name, years, notes, the local view where it happened — and
//! markers on the timeline that show each event's years and jump to them.
//! The chronicle is saved as a Markdown file.

use crate::app::App;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Stroke, Vec2};
use map_core::events::Event;

const MARK: Color32 = Color32::from_rgb(220, 140, 30);

impl App {
    /// Markers of the events over the timeline's slider (`rail`: the
    /// slider's rectangle, covering years `lo`..`hi`).
    pub fn event_markers(&mut self, ui: &mut egui::Ui, rail: Rect, lo: i32, hi: i32) {
        if self.doc.events.is_empty() || hi <= lo {
            return;
        }
        let hr = rail.height() / 2.5;
        let x_of = |y: i32| rail.left() + hr + (y - lo) as f32 / (hi - lo) as f32 * (rail.width() - 2.0 * hr);
        let painter = ui.painter_at(rail.expand2(Vec2::new(4.0, 6.0)));
        let mut jump = None;
        for e in self.doc.events_in_order().into_iter().cloned().collect::<Vec<Event>>() {
            if e.last() < lo || e.year > hi {
                continue;
            }
            let (x0, x1) = (x_of(e.year.max(lo)), x_of(e.last().min(hi)));
            let top = rail.top() + 1.0;
            if x1 > x0 + 1.0 {
                painter.line_segment([Pos2::new(x0, top + 1.0), Pos2::new(x1, top + 1.0)], Stroke::new(3.0, MARK.gamma_multiply(0.7)));
            }
            let tip = Pos2::new(x0, top + 5.0);
            painter.add(egui::Shape::convex_polygon(vec![tip, Pos2::new(x0 - 4.0, top - 2.0), Pos2::new(x0 + 4.0, top - 2.0)], MARK, Stroke::new(0.8, Color32::from_gray(40))));
            let hit = Rect::from_min_max(Pos2::new(x0 - 5.0, top - 4.0), Pos2::new(x1.max(x0) + 5.0, top + 6.0));
            let r = ui.interact(hit, ui.scope_id().with(("event-mark", e.id)), egui::Sense::click());
            let when = match e.end {
                Some(z) if z > e.year => format!("{}—{}", self.doc.calendar.show(e.year), self.doc.calendar.show(z)),
                _ => self.doc.calendar.show(e.year),
            };
            let r = r.on_hover_text(format!("{}　{when}{}", e.name, if e.note.trim().is_empty() { String::new() } else { format!("\n{}", e.note.trim()) }));
            if r.clicked() {
                jump = Some(e);
            }
        }
        if let Some(e) = jump {
            self.set_year(Some(e.year));
            self.event_sel = Some(e.id);
            self.events_open = true;
        }
    }

    pub fn events_window(&mut self, ctx: &egui::Context) {
        if !self.events_open {
            return;
        }
        let mut open = true;
        let mut change: Option<Event> = None;
        let mut remove = None;
        let mut add = false;
        let mut save = false;
        let cal = self.doc.calendar.clone();
        egui::Window::new("事件时间线").open(&mut open).default_width(340.0).show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button("+ 在当前年份新建事件").clicked() {
                    add = true;
                }
                if ui.add_enabled(!self.doc.events.is_empty(), egui::Button::new("导出编年表…")).on_hover_text("按年份列出事件、说明与期间的领土变化（Markdown 文件）").clicked() {
                    save = true;
                }
            });
            if self.doc.events.is_empty() {
                ui.weak("还没有事件。事件给一年或一段年份起名（战争、条约、继承），可附说明并关联局部视图；在时间轴上显示为标记，单击即跳到那一年。");
                return;
            }
            egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                for e in self.doc.events_in_order() {
                    let when = match e.end {
                        Some(z) if z > e.year => format!("{}—{}", cal.show(e.year), cal.show(z)),
                        _ => cal.show(e.year),
                    };
                    let r = ui.selectable_label(self.event_sel == Some(e.id), format!("{when}　{}", e.name));
                    if r.clicked() {
                        self.event_sel = Some(e.id);
                        self.event_text = None;
                    }
                }
            });
            let Some(e) = self.event_sel.and_then(|id| self.doc.event(id)).cloned() else { return };
            ui.separator();
            let (mut name, mut note) = self.event_text.clone().filter(|t| t.0 == e.id).map(|t| (t.1, t.2)).unwrap_or((e.name.clone(), e.note.clone()));
            let rn = ui.add(egui::TextEdit::singleline(&mut name).hint_text("名称").desired_width(300.0));
            ui.horizontal(|ui| {
                let (mut y, mut last) = (e.year, e.last());
                let mut ranged = e.end.is_some();
                ui.label("年份");
                let ry = ui.add(egui::DragValue::new(&mut y).speed(0.2));
                if ui.checkbox(&mut ranged, "至").changed() {
                    change = Some(Event { end: ranged.then_some(y + 1), ..e.clone() });
                }
                if ranged {
                    let rl = ui.add(egui::DragValue::new(&mut last).speed(0.2));
                    if rl.changed() && !rl.dragged() || rl.drag_stopped() {
                        change = Some(Event { end: Some(last.max(y + 1)), ..e.clone() });
                    }
                }
                if ry.changed() && !ry.dragged() || ry.drag_stopped() {
                    change = Some(Event { year: y, end: e.end.map(|z| z.max(y + 1)), ..e.clone() });
                }
                if cal.on {
                    ui.weak(cal.show(y));
                }
            });
            ui.horizontal(|ui| {
                ui.label("地区");
                let shown = e.focus.and_then(|f| self.doc.focus(f)).map_or("（整个地图）".to_string(), |f| f.name.clone());
                egui::ComboBox::from_id_salt("event-focus").selected_text(shown).show_ui(ui, |ui| {
                    if ui.selectable_label(e.focus.is_none(), "（整个地图）").clicked() {
                        change = Some(Event { focus: None, ..e.clone() });
                    }
                    for f in &self.doc.focuses {
                        if ui.selectable_label(e.focus == Some(f.id), &f.name).clicked() {
                            change = Some(Event { focus: Some(f.id), ..e.clone() });
                        }
                    }
                });
            });
            let rt = ui.add(egui::TextEdit::multiline(&mut note).hint_text("说明（可选）").desired_rows(3).desired_width(300.0));
            self.event_text = Some((e.id, name.clone(), note.clone()));
            if (rn.lost_focus() || rt.lost_focus()) && (name.trim() != e.name || note != e.note) && !name.trim().is_empty() {
                change = Some(Event { name: name.trim().to_string(), note: note.clone(), ..e.clone() });
                self.event_text = None;
            }
            // what changed during it
            let faces: Vec<map_core::FaceId> = match e.focus.and_then(|f| self.doc.focus(f)) {
                Some(f) => self.doc.faces_in(f),
                None => self.doc.topo.faces.keys().copied().filter(|f| *f != map_core::BACKGROUND).collect(),
            };
            let rep = self.doc.change_report(&faces, e.year - 1, e.last());
            let moved: f64 = rep.transfers.iter().map(|t| t.2).sum();
            if rep.transfers.is_empty() {
                ui.weak("期间领土没有变化");
            } else {
                ui.label(RichText::new(format!("期间易手 {:.0} km²", moved)).weak());
                for (a, b, k) in rep.transfers.iter().take(4) {
                    ui.weak(format!("{} → {}：{:.0} km²", self.entity_name(*a), self.entity_name(*b), k));
                }
            }
            ui.horizontal(|ui| {
                if ui.button("跳到这一年").clicked() {
                    self.set_year(Some(e.year));
                }
                if let Some(f) = e.focus.filter(|f| self.doc.focus(*f).is_some()) {
                    if ui.button("进入局部视图").clicked() {
                        self.enter_focus(f, true);
                    }
                }
                if ui.button("删除").clicked() {
                    remove = Some(e.id);
                }
            });
        });
        if add {
            let y = self.doc.year.unwrap_or(self.timeline_last_year);
            if let Some(id) = self.edit("新建事件", |d| Ok(d.add_event("", y, None))) {
                // in the local view being looked at, by default
                if let Some(f) = self.focus {
                    let mut e = self.doc.event(id).unwrap().clone();
                    e.focus = Some(f);
                    let _ = self.doc.update_event(e);
                }
                self.event_sel = Some(id);
                self.event_text = None;
                if self.doc.year.is_none() {
                    self.set_year(Some(y));
                }
            }
        }
        if let Some(e) = change {
            self.edit("修改事件", move |d| d.update_event(e));
        }
        if let Some(id) = remove {
            self.edit("删除事件", move |d| d.remove_event(id));
            self.event_sel = None;
        }
        if save {
            if let Some(path) = self.file_dialog().add_filter("Markdown", &["md"]).set_file_name(format!("{} 编年表.md", self.doc.meta.name)).save_file() {
                match std::fs::write(&path, self.doc.chronicle_markdown()) {
                    Ok(()) => self.set_status(format!("已导出编年表 {}", path.display()), false),
                    Err(e) => self.set_status(format!("导出编年表失败：{e}"), true),
                }
            }
        }
        if !open {
            self.events_open = false;
        }
    }
}
