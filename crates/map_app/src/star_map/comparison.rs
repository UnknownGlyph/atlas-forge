//! Compare one trajectory with the same system after removing a chosen body.
use crate::i18n::tr;
use eframe::egui;
use map_core::star_chart::{BodyKey, Gravity, Perturbation};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};

struct Experiment {
    name: String,
    start: f64,
    elapsed: f64,
    rows: Vec<Perturbation>,
    stop: Option<String>,
}

struct Job {
    result: mpsc::Receiver<Experiment>,
    cancel: Arc<AtomicBool>,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(super) struct GravityComparison {
    duration: f64,
    job: Option<Job>,
    result: Option<Experiment>,
}

impl Default for GravityComparison {
    fn default() -> Self {
        Self {
            duration: 365.0,
            job: None,
            result: None,
        }
    }
}

impl GravityComparison {
    fn start(&mut self, sim: &Gravity, selected: BodyKey, ctx: &egui::Context) {
        let Ok(reference) = sim.without(selected) else {
            return;
        };
        let Some(selected) = sim.states().iter().find(|b| b.key == selected) else {
            return;
        };
        let name = selected.name.clone();
        let actual = sim.clone();
        let duration = self.duration;
        let wake = ctx.clone();
        let (send, result) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.job = Some(Job { result, cancel });
        self.result = None;
        crate::perf_ui::spawn("比较引力摄动", move || {
            if let Some(result) = compare(name, actual, reference, duration, &worker_cancel) {
                let _ = send.send(result);
            }
            wake.request_repaint();
        });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, selected: BodyKey, sim: Option<&Gravity>) {
        if let Some(job) = &self.job {
            match job.result.try_recv() {
                Ok(result) => {
                    self.result = Some(result);
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.job = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        ui.separator();
        ui.strong(tr("引力模拟"));
        if let Some(sim) = sim {
            if let Some(orbit) = sim.orbit_reading(selected) {
                ui.label(format!(
                    "{} {:.0} km · {:.3} km/s",
                    tr("当前距离"),
                    orbit.distance_km,
                    orbit.speed_km_s
                ));
                ui.label(format!(
                    "{} {:.5} · {} {:.0} km",
                    tr("瞬时偏心率"),
                    orbit.eccentricity,
                    tr("近心距"),
                    orbit.periapsis_km
                ));
                if let Some(apoapsis) = orbit.apoapsis_km {
                    ui.label(format!(
                        "{} {:.0} km · {} {:.2} d",
                        tr("远心距"),
                        apoapsis,
                        tr("瞬时公转周期"),
                        orbit.period_days.unwrap_or_default()
                    ));
                } else {
                    ui.weak(tr("瞬时轨道无远心点"));
                }
            }
        }
        if self.job.is_some() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("正在计算摄动对照…"));
                if ui.button(tr("取消")).clicked() {
                    self.job = None;
                }
            });
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
        } else if let Some(sim) = sim.filter(|_| selected.can_compare_without()) {
            super::number(ui, "对照时长（地球日）", &mut self.duration, 1.0..=5000.0);
            if ui
                .button(tr("对照有无所选天体"))
                .on_hover_text(tr("从当前状态开始，比较保留和移除所选天体后的轨道差异。"))
                .clicked()
            {
                self.start(sim, selected, ui.ctx());
            }
        } else {
            ui.weak(tr("选中额外行星、伴星或卫星可做摄动对照。"));
        }
        if let Some(result) = &self.result {
            ui.separator();
            ui.strong(format!("{} · {}", tr("移除对照"), result.name));
            ui.weak(format!(
                "{} {:.2} d  +  {:.2} d",
                tr("起算时刻"),
                result.start,
                result.elapsed
            ));
            if let Some(stop) = &result.stop {
                ui.colored_label(ui.visuals().warn_fg_color, stop);
            }
            for row in &result.rows {
                ui.label(&row.name);
                ui.weak(format!(
                    "{} {:.2} km · Δe {:+.6} · {} {:+.2} km",
                    tr("位置偏移"),
                    row.displacement_km,
                    row.eccentricity_change,
                    tr("近心距变化"),
                    row.periapsis_change_km
                ));
            }
        }
    }
}

fn compare(
    name: String,
    mut actual: Gravity,
    mut reference: Gravity,
    duration: f64,
    cancel: &AtomicBool,
) -> Option<Experiment> {
    let start = actual.days();
    let mut elapsed = 0.0;
    let mut stop = None;
    let mut remaining_work = 384;
    while duration - elapsed > 1e-8 && remaining_work > 0 {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        remaining_work -= 1;
        // Finish both runs at the same epoch. Keep the last common state if
        // either branch encounters a collision or reaches its step budget.
        let (mut a, mut b) = (actual.clone(), reference.clone());
        let chunk = (duration - elapsed).min(15.0);
        let attempt = a
            .advance(chunk, 384)
            .and_then(|done| b.advance(done, 384).map(|baseline| (done, baseline)));
        match attempt {
            Ok((done, baseline)) if done > 0.0 && (done - baseline).abs() < 1e-8 => {
                (actual, reference) = (a, b);
                elapsed += done;
            }
            Ok((done, baseline)) if baseline > 0.0 && done > baseline => {
                let mut a = actual.clone();
                match a.advance(baseline, 384) {
                    Ok(now) if (now - baseline).abs() < 1e-8 => {
                        (actual, reference) = (a, b);
                        elapsed += baseline;
                    }
                    _ => break,
                }
            }
            Ok(_) => break,
            Err(error) => {
                stop = Some(error);
                break;
            }
        }
    }
    if stop.is_none() && duration - elapsed > 1e-8 {
        stop = Some(tr("达到对照计算步数上限，已显示最后共同历元的结果。").to_owned());
    }
    Some(Experiment {
        name,
        start,
        elapsed,
        rows: actual.differences(&reference),
        stop,
    })
}
