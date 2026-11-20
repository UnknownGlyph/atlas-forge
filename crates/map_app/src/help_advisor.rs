use crate::{
    actions::{self, Act},
    view::Tool,
};
use eframe::egui;
use map_core::Document;
use std::{sync::mpsc, time::Duration};

#[derive(Default)]
pub(crate) struct Advisor {
    prompt: String,
    pub online: bool,
    turns: Vec<(String, Answer)>,
    job: Option<(String, mpsc::Receiver<Answer>)>,
}

struct Answer {
    text: String,
    tool: Option<Act>,
}

fn recommend(prompt: &str) -> Option<Act> {
    let words = prompt.to_lowercase();
    let entries: &[(Act, &[&str])] = &[
        (
            Act::LandMove,
            &[
                "整体移动",
                "移动陆块",
                "挪动大陆",
                "移动大陆",
                "两块大陆的位置",
                "move land",
            ],
        ),
        (
            Act::Tool(Tool::DrawLand),
            &[
                "画大陆",
                "画陆地",
                "新增岛",
                "画海岸",
                "添加岛",
                "draw land",
            ],
        ),
        (
            Act::Tool(Tool::DrawLake),
            &["挖湖", "画湖", "挖海湾", "carve lake"],
        ),
        (
            Act::Tool(Tool::EditVertex),
            &["编辑顶点", "拖动顶点", "调整节点", "调整海岸点"],
        ),
        (
            Act::Tool(Tool::Redraw),
            &["重画", "改画国界", "修改边界走向"],
        ),
        (Act::Tool(Tool::Split), &["切分", "切开地块", "分成两块"]),
        (
            Act::Tool(Tool::MergeFaces),
            &["合并面", "合并相邻", "合并地块"],
        ),
        (
            Act::Tool(Tool::Assign),
            &["领土画笔", "改归属", "分配陆地", "更改归属"],
        ),
        (
            Act::Tool(Tool::City),
            &["添加城市", "新增城市", "放置城市", "画城市"],
        ),
        (
            Act::Tool(Tool::River),
            &["画河", "画线路", "画路", "画运河", "draw a river"],
        ),
        (Act::Tool(Tool::Pen), &["贝塞尔", "钢笔工具", "画曲线"]),
        (
            Act::Tool(Tool::Label),
            &["地图文字", "添加文字", "调整国名", "写地名"],
        ),
        (
            Act::Tool(Tool::Symbol),
            &["添加符号", "放地图图标", "画地图符号"],
        ),
        (Act::Tool(Tool::Hatch), &["斜线区域", "画森林", "画斜线区"]),
        (
            Act::Tool(Tool::Eyedropper),
            &["吸管", "提取样式", "复制线条样式"],
        ),
        (
            Act::Tool(Tool::Measure),
            &["测量", "相距多远", "算距离", "measure"],
        ),
        (
            Act::Tool(Tool::Align),
            &["配准", "对齐底图", "参考图对齐", "reference alignment"],
        ),
        (
            Act::GeographicGuides,
            &["辅助线", "参考线", "北回归线", "guide"],
        ),
        (
            Act::Checkpoints,
            &["保存点", "恢复版本", "自动保存", "checkpoint"],
        ),
        (Act::Export, &["导出", "输出png", "打印成图", "export"]),
        (
            Act::WorldBook,
            &["世界资料", "独立画板", "关联文档", "world book"],
        ),
        (Act::Tutorial, &["教程", "如何入门", "tutorial"]),
        (Act::Diplomacy, &["外交关系", "加协约"]),
        (Act::Events, &["事件时间线", "历史事件"]),
    ];
    entries
        .iter()
        .filter_map(|(action, names)| {
            let score = names
                .iter()
                .filter(|name| words.contains(**name))
                .map(|name| name.chars().count())
                .max()?;
            Some((score, *action))
        })
        .max_by_key(|(score, _)| *score)
        .map(|(_, action)| action)
}

fn local_answer(doc: &Document, question: &str) -> Answer {
    let tool = recommend(question);
    let diagnosis = ["检查", "拓扑", "错误", "故障", "破面", "bug", "diagnose"]
        .iter()
        .any(|word| question.to_lowercase().contains(word));
    let mut paragraphs = Vec::new();
    if diagnosis {
        let mut session = map_api::Session::new(doc.clone(), "当前工程");
        let report = session.handle_mut(&serde_json::json!({
            "op":"checks.run", "context":{"layer":doc.active_layer.0, "year":doc.year}
        }));
        let result = &report["result"];
        let status = result["topology"].as_str().unwrap_or("检查未完成");
        let unowned = result["unowned_land"]["pieces"].as_u64().unwrap_or(0);
        let conflicts = result["lifespan_conflicts"].as_array().map_or(0, Vec::len);
        paragraphs.push(format!(
            "当前画板、年份与归属层的检查结果\n{}\n无归属陆地 {unowned} 块，实体存续期冲突 {conflicts} 条。\n如果是地图显示问题，请同时检查年份、图层可见性和缩放层级。",
            if status == "ok" {"共享边界与地块拓扑有效"} else {status}
        ));
    }
    if question.contains("经线") || question.to_lowercase().contains("meridian") {
        paragraphs.push("在顶部「中央经线」点击数值，输入经度后按回车。正数代表东经，负数代表西经。点「居中」可把同一条经线移到两种视图的正中央；在「视角」菜单可将其保存为工程打开视角。".into());
    }
    if question.contains("省份") || question.contains("下级") {
        paragraphs.push("先在「国家」标签中选中上级单位，再点「绘制省份」或「绘制下级地区」圈定范围，也可以用「自动划分」预览。只能划入该上级直接管辖的陆地。".into());
    }
    if let Some(entry) = tool.and_then(|act| actions::table().into_iter().find(|a| a.act == act)) {
        paragraphs.push(format!(
            "{} → {}\n{}",
            entry.menu.name(),
            entry.name,
            entry.hint
        ));
    }
    if paragraphs.is_empty() {
        paragraphs.push("可以描述想画的对象或遇到的现象，例如「怎样把两块大陆挪开」「检查共享边界」「怎样在国家下加省份」。复杂问题可启用联网回答。".into());
    }
    Answer {
        text: paragraphs.join("\n\n"),
        tool,
    }
}

impl Advisor {
    pub(crate) fn start(&mut self, doc: Document, config: Option<map_ai::Config>) {
        let question = self.prompt.trim().to_owned();
        if question.is_empty() || self.job.is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        self.prompt.clear();
        self.job = Some((question.clone(), rx));
        let catalog = actions::table()
            .iter()
            .filter(|a| !a.hint.is_empty())
            .map(|a| format!("{} → {}：{}", a.menu.name(), a.name, a.hint))
            .collect::<Vec<_>>()
            .join("\n");
        let history = self
            .turns
            .iter()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .map(|(question, answer)| {
                format!(
                    "用户：{question}\n助手：{}",
                    answer.text.chars().take(1500).collect::<String>()
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        std::thread::spawn(move || {
            let mut answer = local_answer(&doc, &question);
            if let Some(config) = config {
                let prompt = format!("软件实际菜单和操作：\n{catalog}\n\n近期对话（历史答复仅供参考）：\n{history}\n\n本地检查与建议：\n{}\n\n用户问题：{question}", answer.text);
                match map_ai::answer_help(prompt, &config, map_ai::default_transport().as_ref()) {
                    Ok(help) => answer.text = help,
                    Err(error) => answer.text.push_str(&format!("\n\n联网答复：{error}")),
                }
            }
            let _ = tx.send(answer);
        });
    }

    pub(crate) fn body(&mut self, ui: &mut egui::Ui) -> (bool, Option<Act>) {
        if let Some((_, job)) = &self.job {
            match job.try_recv() {
                Ok(answer) => {
                    if let Some((question, _)) = self.job.take() {
                        if self.turns.len() == 32 {
                            self.turns.remove(0);
                        }
                        self.turns.push((question, answer));
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(100))
                }
                Err(mpsc::TryRecvError::Disconnected) => self.job = None,
            }
        }
        let busy = self.job.is_some();
        ui.label(crate::i18n::tr("用自然语言询问操作、查找工具或检查当前地图。"));
        let mut selected = None;
        if !self.turns.is_empty() || busy {
            egui::ScrollArea::vertical()
                .id_salt("guidance-transcript")
                .stick_to_bottom(true)
                .max_height((ui.available_height() - 156.0).max(120.0))
                .show(ui, |ui| {
                    for (question, answer) in &self.turns {
                        ui.strong(question);
                        ui.label(&answer.text);
                        if let Some(a) = answer
                            .tool
                            .and_then(|act| actions::table().into_iter().find(|a| a.act == act))
                        {
                            if ui.button(crate::trf!("使用「{}」", a.name)).clicked() {
                                selected = Some(a.act);
                            }
                        }
                        ui.separator();
                    }
                    if let Some((question, _)) = &self.job {
                        ui.strong(question);
                        ui.spinner();
                    }
                });
        }
        let mut ask = false;
        ui.add_enabled_ui(!busy, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.prompt)
                    .desired_rows(3)
                    .desired_width(f32::INFINITY)
                    .hint_text(crate::i18n::tr("例如：如何移动整个大陆？怎样给国家添加省份？检查当前拓扑。")),
            );
            ui.horizontal_wrapped(|ui| {
                ask = ui
                    .add_enabled(!self.prompt.trim().is_empty(), egui::Button::new(crate::i18n::tr("提问")))
                    .clicked();
                ui.checkbox(&mut self.online, crate::i18n::tr("联网回答"));
                ui.weak(if self.online {
                    crate::i18n::tr("将问题和必要的操作上下文发送给上面所选模型。")
                } else {
                    crate::i18n::tr("本地帮助和地图检查，无需密钥。")
                });
            });
        });
        (ask, selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_tool_and_checks_the_active_document() {
        let doc = Document::default();
        let answer = local_answer(&doc, "怎样整体移动两块大陆，先检查一下拓扑？");
        assert_eq!(answer.tool, Some(Act::LandMove));
        assert!(answer.text.contains("共享边界与地块拓扑有效"));
        assert_eq!(
            local_answer(&doc, "加一根北回归线").tool,
            Some(Act::GeographicGuides)
        );
        let answer = local_answer(&doc, "怎样在国家下加省份");
        assert!(answer.tool.is_none());
        assert!(answer.text.contains("绘制省份"));
    }
}
