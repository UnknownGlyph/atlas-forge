use super::*;
impl ClimateUi {
    pub(super) fn planet_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &mut Project,
        run: &mut Option<Run>,
        external: &mut Option<FileAction>,
    ) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("climate-presets")
                .selected_text("载入行星模板")
                .show_ui(ui, |ui| {
                    for (i, label) in PRESETS.iter().enumerate() {
                        if ui.button(*label).clicked() {
                            p.settings = preset(i);
                            changed = true;
                            ui.close();
                        }
                    }
                });
            if ui.button("读取自定义模板").clicked() {
                *external = Some(FileAction::Load);
            }
            if ui.button("保存为模板").clicked() {
                *external = Some(FileAction::Save);
            }
        });
        let s = &mut p.settings;
        changed |= ui
            .checkbox(
                &mut s.planet.linked,
                "关联重力、密度、光度、辐照、公转和大气密度",
            )
            .changed();
        ui.horizontal(|ui| {
            changed |= ui
                .checkbox(&mut s.planet.locked_rotation, "潮汐锁定")
                .changed();
            changed |= ui.checkbox(&mut s.planet.retrograde, "逆向自转").changed();
        });
        egui::ComboBox::from_id_salt("climate-gas")
            .selected_text(s.planet.gas.name())
            .show_ui(ui, |ui| {
                for gas in [Gas::Nitrogen, Gas::CarbonDioxide, Gas::Hydrogen, Gas::Argon] {
                    changed |= ui
                        .selectable_value(&mut s.planet.gas, gas, gas.name())
                        .changed();
                }
            });
        ui.weak("手动值可直接编辑；固定值需先解锁。只允许自动参数随约束或目标搜索改变。");
        use Param::*;
        let groups: &[(&str, &[Param])] = &[
            (
                "恒星与轨道",
                &[
                    StarMass, StarRadius, StarTemp, Lum, Distance, Flux, Period, Ecc,
                ],
            ),
            (
                "行星与自转",
                &[Radius, Mass, Density, Gravity, Spin, Tilt, Substellar],
            ),
            (
                "大气与地表",
                &[
                    Pressure, AirDensity, CO2, Methane, Greenhouse, Transport, Albedo, Vapour,
                    Ocean,
                ],
            ),
        ];
        for &(label, fields) in groups {
            ui.collapsing(label, |ui| {
                if label == "恒星与轨道" {
                    let t = s.planet[StarTemp];
                    ui.horizontal_wrapped(|ui| {
                        ui.label(format!("辐射光谱峰值约 {:.0} nm", 2.898e6 / t));
                        egui::ComboBox::from_id_salt("stellar-spectrum")
                            .selected_text("恒星光谱模板")
                            .show_ui(ui, |ui| {
                                for (name, t) in [
                                    ("M 红矮星", 3200.),
                                    ("K 橙星", 4600.),
                                    ("G 黄星", 5772.),
                                    ("F 黄白星", 6800.),
                                    ("A 白星", 9000.),
                                    ("B 蓝白星", 16000.),
                                ] {
                                    if ui
                                        .add_enabled(
                                            s.planet.controls[StarTemp as usize] != Control::Fixed,
                                            egui::Button::new(name),
                                        )
                                        .clicked()
                                    {
                                        s.planet[StarTemp] = t;
                                        changed = true;
                                        ui.close();
                                    }
                                }
                            });
                    });
                }
                egui::Grid::new(("climate-params", label)).show(ui, |ui| {
                    for &f in fields {
                        let (title, _, lo, hi, _) = f.spec();
                        ui.label(title);
                        let role = s.planet.controls[f as usize];
                        let speed = (s.planet[f].abs() * 0.012).clamp(0.001, 10.);
                        if ui
                            .add_enabled(
                                role != Control::Fixed,
                                egui::DragValue::new(&mut s.planet[f])
                                    .range(lo..=hi)
                                    .speed(speed)
                                    .max_decimals(6),
                            )
                            .changed()
                        {
                            s.planet.controls[f as usize] = Control::Manual;
                            changed = true;
                        }
                        egui::ComboBox::from_id_salt(("climate-role", f as usize))
                            .width(58.)
                            .selected_text(control_name(role))
                            .show_ui(ui, |ui| {
                                for role in [Control::Fixed, Control::Manual, Control::Auto] {
                                    changed |= ui
                                        .selectable_value(
                                            &mut s.planet.controls[f as usize],
                                            role,
                                            control_name(role),
                                        )
                                        .changed();
                                }
                            });
                        ui.end_row();
                    }
                });
            });
        }
        if changed {
            match s.planet.resolve() {
                Ok(planet) => {
                    s.planet = planet;
                    self.notice.clear();
                }
                Err(e) => self.notice = e,
            }
        }
        if let Err(e) = s.planet.resolve() {
            ui.colored_label(Color32::LIGHT_RED, e);
        }
        ui.collapsing("反向搜索气候目标", |ui| {
            for (label, slot, default, lo, hi) in [
                ("区域年均温 °C", &mut s.goals.temperature, 15., -180., 400.),
                ("年降水 mm", &mut s.goals.rain, 990., 0., 100000.),
                ("冰雪覆盖比例", &mut s.goals.ice, 0.1, 0., 1.),
                ("干旱及半干旱比例", &mut s.goals.dry, 0.18, 0., 1.),
                ("平均年温差 °C", &mut s.goals.season, 12., 0., 300.),
            ] {
                ui.horizontal(|ui| {
                    let mut on = slot.is_some();
                    if ui.checkbox(&mut on, label).changed() {
                        *slot = on.then_some(default);
                        changed = true;
                    }
                    if let Some(v) = slot {
                        changed |= numeric(ui, v, lo..=hi, if hi == 1. { 0.005 } else { 0.2 });
                    }
                });
            }
            if ui
                .add_enabled(
                    self.job.is_none(),
                    egui::Button::new("搜索参数并生成参考图"),
                )
                .clicked()
            {
                *run = Some(Run::Fit);
            }
            ui.weak("在所选区域比较模拟值与目标；气候比例目标是创作参考值。");
        });
        changed
    }
    pub(super) fn ground_tab(&mut self, ui: &mut egui::Ui, p: &mut Project) -> bool {
        let mut changed = false;
        self.position(ui);
        ui.add(egui::Slider::new(&mut self.radius, 0.0..=90.).text("笔刷角半径 °"));
        ui.collapsing("添加地表资料 / 绘制高原", |ui| {
            ui.label("拓扑与已有高程提供底图。此处可单独指定地表与海拔。");
            egui::ComboBox::from_id_salt("climate-bg")
                .selected_text(format!("未绘区：{}", p.background.name()))
                .show_ui(ui, |ui| {
                    for s in [Surface::Unknown, Surface::Water, Surface::Land] {
                        changed |= ui
                            .selectable_value(&mut p.background, s, s.name())
                            .changed();
                    }
                });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.set_surface, "地表");
                egui::ComboBox::from_id_salt("climate-brush")
                    .selected_text(self.surface.name())
                    .show_ui(ui, |ui| {
                        for s in Surface::ALL {
                            ui.selectable_value(&mut self.surface, s, s.name());
                        }
                    });
            });
            ui.horizontal(|ui| {
                ui.checkbox(&mut self.set_height, "海拔 m");
                ui.add(
                    egui::DragValue::new(&mut self.height)
                        .speed(20.)
                        .range(-12000.0..=30000.),
                );
            });
            ui.horizontal(|ui| {
                if ui.button("写入所选地区").clicked() {
                    for i in footprint(self.point, self.radius) {
                        let mut e = p.surface_edits.get(&i).copied().unwrap_or_default();
                        if self.set_surface {
                            e.surface = Some(self.surface);
                        }
                        if self.set_height {
                            e.height = Some(self.height);
                        }
                        p.surface_edits.insert(i, e);
                    }
                    changed = true;
                }
                if ui.button("恢复地图地表").clicked() {
                    for i in footprint(self.point, self.radius) {
                        p.surface_edits.remove(&i);
                    }
                    changed = true;
                }
            });
        });
        ui.separator();
        ui.label(format!(
            "人工修订　{} · {}",
            p.field.name(),
            p.period.name()
        ));
        if p.selected.is_some() {
            ui.label("正在查看留存方案。切回当前结果后可修订。");
            return changed;
        }
        if p.current.is_none() {
            ui.label("运行模拟后可修订或固定参考值。");
            return changed;
        }
        ui.horizontal_wrapped(|ui| {
            if matches!(p.field, Field::Vegetation | Field::Zone) {
                egui::ComboBox::from_id_salt("climate-category")
                    .selected_text(p.field.format(self.manual))
                    .show_ui(ui, |ui| {
                        for k in 0..=p.field.scale(p.period).1 as usize {
                            ui.selectable_value(
                                &mut self.manual,
                                k as f32,
                                p.field.format(k as f32),
                            );
                        }
                    });
            } else {
                let bounds = match p.field {
                    Field::Humidity | Field::Ice => 0.0..=100.,
                    Field::Wind => 0.0..=360.,
                    Field::Temperature => -273.15..=1000.,
                    _ => 0.0..=10000000.,
                };
                ui.add(
                    egui::DragValue::new(&mut self.manual)
                        .speed(0.3)
                        .range(bounds),
                );
            }
            ui.checkbox(&mut self.lock, "重算时固定此图层");
        });
        let mut op = None;
        ui.horizontal_wrapped(|ui| {
            for (k, label) in ["写入人工值", "固定当前值", "固定所有气候量", "恢复自然值"]
                .iter()
                .enumerate()
            {
                if ui.button(*label).clicked() {
                    op = Some(k);
                }
            }
        });
        if let Some(k) = op {
            let cells = footprint(self.point, self.radius);
            let s = Arc::make_mut(p.current.as_mut().unwrap());
            match k {
                0 => s.paint(&cells, p.field, p.period, Some(self.manual), self.lock),
                1 => s.paint(&cells, p.field, p.period, None, true),
                2 => {
                    for f in Field::ALL {
                        s.paint(&cells, f, Period::Annual, None, true);
                    }
                }
                _ => {
                    for i in cells {
                        s.edits.remove(&(i, p.field));
                    }
                }
            }
            changed = true;
        }
        ui.weak("固定保存所选网格的逐月值。未固定的人工修订在下次模拟时更新。");
        changed
    }
    pub(super) fn records_tab(
        &mut self,
        ui: &mut egui::Ui,
        p: &mut Project,
        clear: &mut bool,
        external: &mut Option<FileAction>,
    ) -> bool {
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label("方案名称");
            ui.text_edit_singleline(&mut self.name);
        });
        if ui
            .add_enabled(
                p.current.is_some() && !self.name.trim().is_empty(),
                egui::Button::new("留存当前结果和参数"),
            )
            .clicked()
        {
            p.saved.push((
                self.name.trim().to_string(),
                p.current.as_ref().unwrap().clone(),
            ));
            self.name = format!("方案 {}", p.saved.len() + 1);
            changed = true;
        }
        changed |= ui
            .selectable_value(&mut p.selected, None, "当前结果")
            .changed();
        for (i, (name, _)) in p.saved.iter().enumerate() {
            changed |= ui
                .selectable_value(&mut p.selected, Some(i), format!("{}　{name}", i + 1))
                .changed();
        }
        if let Some((i, (_, s))) = p.selected.and_then(|i| p.saved.get(i).map(|s| (i, s))) {
            if let (Some(current), Some(shown)) = (&p.current, &self.display) {
                if Arc::ptr_eq(&shown.source, s) && current.region == s.region {
                    if self
                        .comparison
                        .as_ref()
                        .is_none_or(|(old, _)| !Arc::ptr_eq(old, current))
                    {
                        self.comparison = Some((current.clone(), current.summary(s.region)));
                    }
                    let a = self.comparison.as_ref().unwrap().1;
                    let b = shown.stats;
                    ui.label(format!(
                        "相对当前结果　均温 {:+.1} °C　年降水 {:+.0} mm",
                        b[0] - a[0],
                        b[1] - a[1]
                    ));
                } else if current.region != s.region {
                    ui.label("两个方案的模拟范围不同。");
                }
            }
            if ui.button("复用此方案参数").clicked() {
                p.settings = s.settings.clone();
                p.region = s.region;
                changed = true;
            }
            if ui.button("移除此留存方案").clicked() {
                p.saved.remove(i);
                p.selected = None;
                changed = true;
            }
        }
        ui.separator();
        ui.horizontal_wrapped(|ui| {
            let ready = p.shown().is_some();
            if ui
                .add_enabled(ready, egui::Button::new("导出当前图层 PNG"))
                .clicked()
            {
                *external = Some(FileAction::Png);
            }
            if ui
                .add_enabled(ready, egui::Button::new("导出逐月数据 CSV"))
                .clicked()
            {
                *external = Some(FileAction::Csv);
            }
            if ui
                .add_enabled(
                    p.current.is_some() || self.job.is_some(),
                    egui::Button::new("清除当前结果"),
                )
                .clicked()
            {
                *clear = true;
            }
        });
        ui.weak("保存 .atlas 工程时一并保存结果、固定区域和留存方案。");
        changed
    }
}
fn control_name(role: Control) -> &'static str {
    match role {
        Control::Manual => "手动",
        Control::Auto => "自动",
        Control::Fixed => "固定",
    }
}
impl App {
    pub(in crate::climate_ui) fn climate_file(
        &mut self,
        action: FileAction,
        p: &mut Project,
    ) -> Result<bool, String> {
        if matches!(action, FileAction::Load) {
            let Some(path) = self
                .file_dialog()
                .add_filter("气候模板", &["json"])
                .pick_file()
            else {
                return Ok(false);
            };
            let s: Settings =
                serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            s.planet.resolve()?;
            p.settings = s;
            return Ok(true);
        }
        let (ext, name) = match action {
            FileAction::Save => ("json", "行星模板"),
            FileAction::Png => ("png", "气候参考"),
            _ => ("csv", "逐月气候"),
        };
        let Some(path) = self
            .file_dialog()
            .add_filter(ext, &[ext])
            .set_file_name(format!("{name}.{ext}"))
            .save_file()
        else {
            return Ok(false);
        };
        let bytes = if matches!(action, FileAction::Save) {
            serde_json::to_vec_pretty(&p.settings).map_err(|e| e.to_string())?
        } else {
            let s = p.shown().ok_or("没有气候结果")?;
            if matches!(action, FileAction::Png) {
                use image::ImageEncoder;
                let (w, h, rgba) = s.raster(p.field, p.period);
                let mut data = Vec::new();
                image::codecs::png::PngEncoder::new(&mut data)
                    .write_image(&rgba, w as u32, h as u32, image::ExtendedColorType::Rgba8)
                    .map_err(|e| e.to_string())?;
                data
            } else {
                s.csv().into_bytes()
            }
        };
        map_io::recovery::write_atomic(&path, &bytes).map_err(|e| e.to_string())?;
        self.climate_ui.notice = format!("已保存 {}", path.display());
        Ok(false)
    }
}
