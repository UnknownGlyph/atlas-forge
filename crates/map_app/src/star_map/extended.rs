use super::{StarChart, StarMap};
use crate::i18n::tr;
use eframe::egui;
use glam::DVec3;
use map_core::{
    star_chart::{Body, BodyKey, Gravity, Star},
    world_book::BookId,
    Document,
};
use std::{
    collections::BTreeMap,
    sync::{mpsc, Arc},
};

pub(super) struct Driver {
    chart: StarChart,
    star: Star,
    home: Body,
    pub(super) simulation: Gravity,
}

type PendingModel = mpsc::Receiver<(BookId, Result<map_io::model::Model, String>)>;

#[derive(Default)]
pub(super) struct Models {
    ready: BTreeMap<BookId, Arc<map_io::model::Model>>,
    pending: BTreeMap<BookId, PendingModel>,
    failed: BTreeMap<BookId, String>,
    inputs: BTreeMap<BookId, map_core::underlay::Blob>,
}

impl Models {
    pub(super) fn update(&mut self, doc: &Document, chart: &StarChart, ctx: &egui::Context) {
        let desired: std::collections::BTreeSet<_> = chart.model_ids().collect();
        let book = doc.book.as_deref();
        let evicted: Vec<_> = self
            .inputs
            .iter()
            .filter(|(id, old)| {
                !desired.contains(*id)
                    || book
                        .and_then(|b| b.assets.get(*id))
                        .is_none_or(|asset| !Arc::ptr_eq(&asset.bytes.0, &old.0))
            })
            .map(|(&k, _)| k)
            .collect();
        for id in evicted {
            self.ready.remove(&id);
            self.pending.remove(&id);
            self.failed.remove(&id);
            self.inputs.remove(&id);
        }
        let done: Vec<_> = self
            .pending
            .iter()
            .filter_map(|(&id, job)| match job.try_recv() {
                Ok(pair) => Some(pair),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some((id, Err(tr("未能载入模型。").into())))
                }
                _ => None,
            })
            .collect();
        for (id, result) in done {
            self.pending.remove(&id);
            match result {
                Ok(model) => {
                    self.ready.insert(id, Arc::new(model));
                }
                Err(error) => {
                    self.failed.insert(id, error);
                }
            }
        }
        if let Some(book) = &doc.book {
            for id in chart.model_ids() {
                if self.ready.contains_key(&id)
                    || self.pending.contains_key(&id)
                    || self.failed.contains_key(&id)
                {
                    continue;
                }
                if self.pending.len() >= 2 {
                    break;
                }
                if let Some(asset) = book.assets.get(&id).cloned() {
                    self.inputs.insert(id, asset.bytes.clone());
                    let (tx, rx) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = tx.send((id, map_io::model::decode(&asset)));
                    });
                    self.pending.insert(id, rx);
                }
            }
        }
        if !self.pending.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(90));
        }
    }

    pub(super) fn get(&self, id: Option<BookId>) -> Option<&map_io::model::Model> {
        self.ready.get(&id?).map(Arc::as_ref)
    }
}

#[derive(Clone, Copy)]
pub(super) enum AssetCommand {
    Import(BodyKey),
    Inspect(BookId),
}

pub(super) fn model_row(
    ui: &mut egui::Ui,
    key: BodyKey,
    id: &mut Option<BookId>,
    doc: &Document,
    action: &mut Option<AssetCommand>,
) {
    let book = doc.book.as_deref();
    ui.horizontal_wrapped(|ui| {
        let selected = id
            .and_then(|id| book?.assets.get(&id).map(|a| a.name.as_str()))
            .unwrap_or(tr("无模型"));
        egui::ComboBox::from_id_salt("astronomy-model")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                ui.selectable_value(id, None, tr("无模型"));
                if let Some(book) = book {
                    for (&asset, data) in book
                        .assets
                        .iter()
                        .filter(|(_, a)| map_io::model::is_model(a))
                    {
                        ui.selectable_value(id, Some(asset), &data.name);
                    }
                }
            });
        if ui.button(tr("导入 3D 模型")).clicked() {
            *action = Some(AssetCommand::Import(key));
        }
        if let Some(id) = *id {
            if ui.button(tr("打开 3D 查看器")).clicked() {
                *action = Some(AssetCommand::Inspect(id));
            }
        }
    });
}

impl StarMap {
    pub(super) fn tick_gravity(&mut self, doc: &Document, time_delta: f64, ui: &egui::Ui) {
        let Some((_, chart)) = &self.draft else {
            return;
        };
        if !chart.gravity {
            self.driver = None;
            self.gravity_error = None;
            self.actual_step = time_delta;
            self.days += time_delta;
            return;
        }
        let (star, home) = chart.resolved(doc);
        let reset = self.driver.as_ref().is_none_or(|d| {
            &d.chart != chart
                || d.star != star
                || d.home != home
                || (d.simulation.days() - self.days).abs() > 1e-8
        });
        if reset {
            self.driver = None;
            match Gravity::new(chart, doc, self.days) {
                Ok(simulation) => {
                    self.driver = Some(Driver {
                        chart: chart.clone(),
                        star,
                        home,
                        simulation,
                    });
                    self.gravity_error = None;
                }
                Err(error) => {
                    self.gravity_error = Some(error);
                    self.playing = false;
                }
            }
        }
        let Some(driver) = &mut self.driver else {
            return;
        };
        self.actual_step = 0.0;
        if self.playing && time_delta > 0.0 {
            match driver.simulation.advance(time_delta, 95) {
                Ok(span) => {
                    self.actual_step = span;
                }
                Err(error) => {
                    self.gravity_error = Some(error);
                    self.playing = false;
                }
            }
            self.days = driver.simulation.days();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    pub(super) fn position(&self, key: BodyKey, analytical: impl FnOnce() -> DVec3) -> DVec3 {
        self.driver
            .as_ref()
            .and_then(|d| d.simulation.position(key))
            .unwrap_or_else(analytical)
    }
}
