//! The assistant in the interface (stage D, `map_ai`): a window where a
//! task is given to a language model (Gemini or DeepSeek), which works on a
//! copy of the map through the same tools as the MCP server — edits change
//! a draft only. The steps show as they happen; at the end the window shows
//! the edits and the map before and after (the selected year), and the
//! draft is applied as one undo step ("AI（模型）：任务") or dropped. The key
//! comes from GEMINI_API_KEY / DEEPSEEK_API_KEY or is typed in (kept in
//! memory only, never saved).

use crate::app::App;
use eframe::egui::{self, Color32, RichText};
use map_core::Document;
use std::sync::mpsc;

enum Msg {
    Step(String),
    Done(Result<Box<Finished>, String>),
}

struct Finished {
    draft: Option<Document>,
    edits: Vec<String>,
    summary: String,
    pictures: Vec<egui::ColorImage>,
}

/// The window's state.
pub struct Assistant {
    pub open: bool,
    /// 0: Gemini, 1: DeepSeek.
    provider: usize,
    model: [String; 2],
    effort: [String; 2],
    key: String,
    task: String,
    log: Vec<String>,
    job: Option<mpsc::Receiver<Msg>>,
    /// The history's generation when the task started (the map it worked on).
    base: u64,
    done: Option<Finished>,
    textures: Vec<egui::TextureHandle>,
    error: Option<String>,
}

impl Default for Assistant {
    fn default() -> Self {
        Assistant {
            open: false,
            provider: 0,
            model: ["gemini-4-hydrogen".into(), "deepseek-flash".into()],
            effort: ["high".into(), String::new()],
            key: String::new(),
            task: String::new(),
            log: vec![],
            job: None,
            base: 0,
            done: None,
            textures: vec![],
            error: None,
        }
    }
}

const ENV: [&str; 2] = ["GEMINI_API_KEY", "DEEPSEEK_API_KEY"];

impl App {
    fn start_assistant(&mut self) {
        let a = &mut self.ai;
        let key = if a.key.trim().is_empty() { std::env::var(ENV[a.provider]).unwrap_or_default() } else { a.key.trim().to_string() };
        if key.is_empty() {
            a.error = Some(format!("没有密钥：在下面填入，或设置环境变量 {}", ENV[a.provider]));
            return;
        }
        let effort = Some(a.effort[a.provider].trim()).filter(|e| !e.is_empty());
        let cfg = if a.provider == 0 { map_ai::Config::gemini(a.model[0].trim(), &key, effort) } else { map_ai::Config::deepseek(a.model[1].trim(), &key, effort) };
        let (tx, rx) = mpsc::channel();
        let (doc, task, year) = (self.doc.clone(), a.task.trim().to_string(), self.doc.year);
        a.log = vec![format!("模型：{}　任务：{task}", cfg.label())];
        a.done = None;
        a.textures.clear();
        a.error = None;
        a.base = self.history.generation();
        a.job = Some(rx);
        std::thread::spawn(move || {
            let mut session = map_api::Session::new(doc, "当前地图");
            let net = map_ai::default_transport();
            let step_tx = tx.clone();
            let run = map_ai::run(&mut session, &task, &cfg, net.as_ref(), &mut |s| {
                let line = match s {
                    map_ai::Step::Said(t) => format!("模型：{t}"),
                    map_ai::Step::Call(n, a) => format!("→ {n} {}", a.to_string().chars().take(160).collect::<String>()),
                    map_ai::Step::Answer(t, err) => format!("{} {}", if err { "✗" } else { "←" }, t.chars().take(160).collect::<String>().replace('\n', " ")),
                };
                let _ = step_tx.send(Msg::Step(line));
            });
            let finished = run.map(|o| {
                // the map before and after, in the year shown
                let mut pictures = Vec::new();
                if session.draft().is_some() {
                    for before in [true, false] {
                        let path = std::env::temp_dir().join(format!("atlas_ai_preview_{}_{before}.png", std::process::id()));
                        let args = serde_json::json!({"path": path.display().to_string(), "before": before, "width": 640, "height": 340, "year": year, "legend": false});
                        let (_, err) = map_api::mcp::call(&mut session, "preview_render", &args);
                        if let (false, Ok(bytes)) = (err, std::fs::read(&path)) {
                            if let Ok(img) = image::load_from_memory(&bytes) {
                                let img = img.to_rgba8();
                                pictures.push(egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw()));
                            }
                        }
                        let _ = std::fs::remove_file(&path);
                    }
                }
                Box::new(Finished { draft: session.draft().cloned(), edits: session.edit_log().to_vec(), summary: o.summary, pictures })
            });
            let _ = tx.send(Msg::Done(finished));
        });
    }

    pub fn assistant_window(&mut self, ctx: &egui::Context) {
        if !self.ai.open {
            return;
        }
        // what the running task sent
        if let Some(rx) = &self.ai.job {
            loop {
                match rx.try_recv() {
                    Ok(Msg::Step(s)) => self.ai.log.push(s),
                    Ok(Msg::Done(r)) => {
                        match r {
                            Ok(f) => {
                                self.ai.textures = f.pictures.iter().enumerate().map(|(i, p)| ctx.load_texture(format!("ai-preview-{i}"), p.clone(), egui::TextureOptions::LINEAR)).collect();
                                self.ai.done = Some(*f);
                            }
                            Err(e) => self.ai.error = Some(e),
                        }
                        self.ai.job = None;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => {
                        ctx.request_repaint_after(std::time::Duration::from_millis(150));
                        break;
                    }
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.ai.job = None;
                        break;
                    }
                }
            }
        }
        let mut open = true;
        let (mut run, mut apply, mut drop) = (false, false, false);
        egui::Window::new("AI 助手").open(&mut open).default_size([620.0, 560.0]).resizable(true).show(ctx, |ui| {
            let a = &mut self.ai;
            let busy = a.job.is_some();
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal(|ui| {
                    ui.radio_value(&mut a.provider, 0, "Gemini");
                    ui.radio_value(&mut a.provider, 1, "DeepSeek");
                    ui.label("模型");
                    ui.add(egui::TextEdit::singleline(&mut a.model[a.provider]).desired_width(150.0));
                    ui.label("思考强度");
                    ui.add(egui::TextEdit::singleline(&mut a.effort[a.provider]).desired_width(60.0).hint_text("默认"));
                });
                ui.horizontal(|ui| {
                    ui.label("密钥");
                    ui.add(egui::TextEdit::singleline(&mut a.key).password(true).desired_width(260.0).hint_text(format!("留空则用环境变量 {}", ENV[a.provider])))
                        .on_hover_text("只保存在内存中，不写入文件。");
                });
                ui.label("任务");
                ui.add(egui::TextEdit::multiline(&mut a.task).desired_rows(3).desired_width(f32::INFINITY).hint_text("例如：从 1500 年起，把甲国东半部划给东边的邻国，并在时间轴上记一笔"));
                ui.horizontal(|ui| {
                    if ui.add_enabled(!a.task.trim().is_empty(), egui::Button::new("开始")).clicked() {
                        run = true;
                    }
                    ui.weak("AI 只修改一份草稿；看过前后对比后再决定是否采用。地图里的文字只作为数据交给模型。");
                });
            });
            if busy {
                ui.horizontal(|ui| {
                    ui.add(egui::Spinner::new());
                    ui.label(format!("进行中…（{} 步）", a.log.len().saturating_sub(1)));
                });
            }
            if let Some(e) = &a.error {
                ui.colored_label(Color32::from_rgb(220, 90, 60), e);
            }
            ui.separator();
            egui::ScrollArea::vertical().max_height(200.0).stick_to_bottom(true).show(ui, |ui| {
                for l in &a.log {
                    ui.label(RichText::new(l).small());
                }
            });
            if let Some(f) = &a.done {
                ui.separator();
                if !f.summary.trim().is_empty() {
                    ui.label(RichText::new(f.summary.trim()).strong());
                }
                match &f.draft {
                    None => {
                        ui.weak("没有修改。");
                    }
                    Some(_) => {
                        ui.label(format!("草稿中的 {} 处修改：", f.edits.len()));
                        for e in &f.edits {
                            ui.label(format!("· {e}"));
                        }
                        if a.textures.len() == 2 {
                            ui.horizontal(|ui| {
                                for (t, name) in a.textures.iter().zip(["之前", "之后"]) {
                                    ui.vertical(|ui| {
                                        ui.weak(name);
                                        ui.image((t.id(), egui::vec2(290.0, 154.0)));
                                    });
                                }
                            });
                        }
                        ui.horizontal(|ui| {
                            if ui.button("应用到地图").on_hover_text("作为一步撤销；Ctrl+Z 可撤回").clicked() {
                                apply = true;
                            }
                            if ui.button("放弃").clicked() {
                                drop = true;
                            }
                        });
                    }
                }
            }
        });
        if run {
            self.start_assistant();
        }
        if apply {
            if self.history.generation() != self.ai.base {
                self.ai.error = Some("AI 开始之后地图又被修改过，草稿与现在的地图不一致；请重新运行任务。".into());
            } else if let Some(draft) = self.ai.done.as_mut().and_then(|f| f.draft.take()) {
                let label = format!("AI（{}）：{}", self.ai.model[self.ai.provider].trim(), self.ai.task.trim().chars().take(30).collect::<String>());
                // the draft keeps the map's year and layer as they were
                if self.edit(&label, move |d| {
                    *d = draft;
                    Ok(())
                })
                .is_some()
                {
                    self.set_status(format!("已采用 AI 的修改：{label}"), false);
                    self.ai.done = None;
                    self.ai.textures.clear();
                }
            }
        }
        if drop {
            self.ai.done = None;
            self.ai.textures.clear();
        }
        if !open {
            self.ai.open = false;
        }
    }
}
