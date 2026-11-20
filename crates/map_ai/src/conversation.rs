use crate::{gemini_schema, shorten, Api, Config, Outcome, Step, Transport, SYSTEM};
use map_api::Session;
use serde_json::{json, Value};

struct Call {
    name: String,
    id: Option<String>,
    args: Value,
}

struct Turn {
    history: Vec<Value>,
    text: Vec<String>,
    calls: Vec<Call>,
}

struct Conversation<'a> {
    config: &'a Config,
    url: String,
    headers: Vec<(String, String)>,
    tools: Vec<Value>,
    history: Vec<Value>,
    instructions: &'a str,
}

impl<'a> Conversation<'a> {
    fn new(config: &'a Config, prompt: String, instructions: &'a str, drawing: bool) -> Self {
        let specs = map_api::mcp::tool_specs()
            .into_iter()
            .filter(|t| drawing && t["name"] != "draft_save");
        let tools = specs.map(|mut t| {
            // Files are written by the host when asked by the user. The
            // model can request a preview without giving it a filename.
            if t["name"] == "preview_render" {
                if let Some(p) = t["parameters"]["properties"].as_object_mut() {
                    p.remove("path");
                }
            }
            match config.api {
                Api::Gemini => json!({"name": t["name"], "description": t["description"], "parameters": gemini_schema(&t["parameters"])}),
                Api::OpenAi => json!({"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["parameters"]}}),
                Api::Responses => json!({"type":"function","name":t["name"],"description":t["description"],"parameters":t["parameters"],"strict":false}),
                Api::Anthropic => json!({"name":t["name"],"description":t["description"],"input_schema":t["parameters"]}),
            }
        }).collect();
        let initial = match config.api {
            Api::Gemini => json!({"role":"user","parts":[{"text":prompt}]}),
            _ => json!({"role":"user","content":prompt}),
        };
        let mut history = Vec::new();
        if config.api == Api::OpenAi {
            history.push(json!({"role":"system","content":instructions}));
        }
        history.push(initial);
        let endpoint = match config.api {
            Api::Gemini => config.endpoint(&format!(
                "/models/{}:generateContent",
                config.model.trim_start_matches("models/")
            )),
            Api::OpenAi => config.endpoint("/chat/completions"),
            Api::Responses => config.endpoint("/responses"),
            Api::Anthropic => config.endpoint("/messages"),
        };
        Self {
            config,
            url: endpoint,
            headers: config.headers(),
            tools,
            history,
            instructions,
        }
    }

    fn request(&self) -> Value {
        let model = &self.config.model;
        let mut body = match self.config.api {
            Api::Gemini => {
                json!({"systemInstruction":{"parts":[{"text":self.instructions}]},"contents":self.history,"tools":[{"functionDeclarations":self.tools}]})
            }
            Api::OpenAi => json!({"model":model,"messages":self.history,"tools":self.tools}),
            Api::Responses => {
                json!({"model":model,"instructions":self.instructions,"input":self.history,"tools":self.tools,"store":false,"include":["reasoning.encrypted_content"]})
            }
            Api::Anthropic => {
                json!({"model":model,"system":self.instructions,"messages":self.history,"tools":self.tools,"max_tokens":4096})
            }
        };
        if self.tools.is_empty() {
            body.as_object_mut().unwrap().remove("tools");
        }
        if let Some(effort) = &self.config.effort {
            match self.config.api {
                Api::Gemini => {
                    body["generationConfig"] = json!({"thinkingConfig":{"thinkingLevel":effort}})
                }
                Api::OpenAi => body["reasoning_effort"] = json!(effort),
                Api::Responses => body["reasoning"] = json!({"effort":effort}),
                Api::Anthropic => {} // the Messages preset uses Claude's default
            }
        }
        body
    }

    fn tool_reply(&self, call: &Call, text: String, error: bool) -> Value {
        match self.config.api {
            Api::Gemini => {
                let mut v = json!({"name":call.name,"response":{"ok":!error,"content":text}});
                if let Some(id) = &call.id {
                    v["id"] = json!(id);
                }
                json!({"functionResponse": v})
            }
            Api::Anthropic => {
                json!({"type":"tool_result","tool_use_id":call.id,"content":text,"is_error":error})
            }
            Api::Responses => {
                json!({"type":"function_call_output","call_id":call.id,"output":marked(text,error)})
            }
            Api::OpenAi => {
                json!({"role":"tool","tool_call_id":call.id,"content":marked(text,error)})
            }
        }
    }

    fn append_replies(&mut self, replies: Vec<Value>) {
        match self.config.api {
            Api::Gemini => self.history.push(json!({"role":"user","parts":replies})),
            Api::Anthropic => self.history.push(json!({"role":"user","content":replies})),
            _ => self.history.extend(replies),
        }
    }
}

fn marked(text: String, error: bool) -> String {
    if error {
        format!("错误：{text}")
    } else {
        text
    }
}

fn call(name: &Value, id: &Value, args: Value, api: Api) -> Result<Call, String> {
    let name = name
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or("工具调用缺少名称")?
        .to_owned();
    let id = id.as_str().filter(|s| !s.is_empty()).map(str::to_owned);
    if api != Api::Gemini && id.is_none() {
        return Err(format!("工具 {name} 缺少调用 ID。"));
    }
    Ok(Call { name, id, args })
}

fn arguments(value: &Value) -> Value {
    // Some compatibility gateways return an object directly. Invalid
    // arguments become a tool error, so a malformed call cannot edit.
    if value.is_object() {
        return value.clone();
    }
    value
        .as_str()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(Value::Null)
}

fn turn(api: Api, answer: &Value) -> Result<Turn, String> {
    let mut text = Vec::new();
    let mut calls = Vec::new();
    let mut say = |v: &Value| {
        if let Some(s) = v.as_str().filter(|s| !s.trim().is_empty()) {
            text.push(s.to_owned());
        }
    };
    let history = match api {
        Api::Gemini => {
            let msg = &answer["candidates"][0]["content"];
            for part in msg["parts"]
                .as_array()
                .ok_or("Gemini 没有给出可读取的回答。")?
            {
                if part["thought"] != true {
                    say(&part["text"]);
                }
                if let Some(f) = part.get("functionCall") {
                    calls.push(call(
                        &f["name"],
                        &f["id"],
                        f.get("args").cloned().unwrap_or_else(|| json!({})),
                        api,
                    )?);
                }
            }
            vec![msg.clone()]
        }
        Api::OpenAi => {
            let msg = &answer["choices"][0]["message"];
            if !msg.is_object() {
                return Err("Chat Completions 没有给出可读取的回答。".into());
            }
            say(&msg["content"]);
            if let Some(parts) = msg["content"].as_array() {
                for part in parts {
                    say(&part["text"]);
                }
            }
            if let Some(items) = msg["tool_calls"].as_array() {
                for item in items {
                    let f = &item["function"];
                    calls.push(call(
                        &f["name"],
                        &item["id"],
                        arguments(&f["arguments"]),
                        api,
                    )?);
                }
            }
            // Pass through provider-specific reasoning fields, used by
            // DeepSeek and other compatible models during tool replay.
            vec![msg.clone()]
        }
        Api::Anthropic => {
            let parts = answer["content"]
                .as_array()
                .ok_or("Claude 没有给出可读取的回答。")?;
            for part in parts {
                match part["type"].as_str() {
                    Some("text") => say(&part["text"]),
                    Some("tool_use") => calls.push(call(
                        &part["name"],
                        &part["id"],
                        part["input"].clone(),
                        api,
                    )?),
                    _ => {}
                }
            }
            vec![json!({"role":"assistant","content":parts})]
        }
        Api::Responses => {
            if matches!(
                answer["status"].as_str(),
                Some("incomplete" | "failed" | "cancelled")
            ) {
                return Err(format!(
                    "Responses 任务未完成：{}",
                    answer["incomplete_details"]["reason"]
                        .as_str()
                        .unwrap_or("请检查模型的响应状态")
                ));
            }
            let items = answer["output"]
                .as_array()
                .ok_or("Responses 没有给出可读取的回答。")?;
            for item in items {
                match item["type"].as_str() {
                    Some("message") => {
                        if let Some(parts) = item["content"].as_array() {
                            for part in parts {
                                if part["type"] == "output_text" {
                                    say(&part["text"]);
                                }
                                if part["type"] == "refusal" {
                                    say(&part["refusal"]);
                                }
                            }
                        }
                    }
                    Some("function_call") => calls.push(call(
                        &item["name"],
                        &item["call_id"],
                        arguments(&item["arguments"]),
                        api,
                    )?),
                    _ => {}
                }
            }
            items.clone()
        }
    };
    Ok(Turn {
        history,
        text,
        calls,
    })
}

fn execute(session: &mut Session, item: &Call) -> (String, bool) {
    if item.name == "draft_save" {
        return ("保存草稿由 Atlas Forge 或命令行调用方负责。".into(), true);
    }
    if !item.args.is_object() {
        return ("工具参数须为有效的 JSON 对象。".into(), true);
    }
    // A second enforcement point for callers that ignore the tool schema.
    if item.name == "preview_render" {
        let mut args = item.args.clone();
        args.as_object_mut().unwrap().remove("path");
        return map_api::mcp::call(session, &item.name, &args);
    }
    map_api::mcp::call(session, &item.name, &item.args)
}

pub(crate) fn run(
    session: &mut Session,
    task: &str,
    config: &Config,
    net: &dyn Transport,
    on_step: &mut dyn FnMut(Step),
) -> Result<Outcome, String> {
    config.validate()?;
    let prompt = format!(
        "当前世界：{}；年份范围与分区层请用 world_summary 查看。\n\n任务：{task}",
        session.doc().meta.name
    );
    let mut convo = Conversation::new(config, prompt, SYSTEM, true);
    let mut outcome = Outcome {
        summary: String::new(),
        turns: 0,
        calls: 0,
    };
    for _ in 0..config.max_turns {
        outcome.turns += 1;
        let (status, answer) = net.post(
            &convo.url,
            &convo.headers,
            &convo.request(),
            config.timeout_s,
        )?;
        if !(200..300).contains(&status) {
            let message = answer["error"]["message"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| answer.to_string());
            let message = if config.key.is_empty() {
                message
            } else {
                message.replace(&config.key, "***")
            };
            return Err(format!(
                "接口返回 HTTP {status}：{}",
                shorten(&message, 400)
            ));
        }
        let turn = turn(config.api, &answer)?;
        for line in turn.text {
            on_step(Step::Said(line.clone()));
            outcome.summary = line;
        }
        if turn.calls.is_empty() {
            return Ok(outcome);
        }
        convo.history.extend(turn.history);
        let mut replies = Vec::new();
        for item in turn.calls {
            on_step(Step::Call(item.name.clone(), item.args.clone()));
            outcome.calls += 1;
            let (text, error) = execute(session, &item);
            let text = shorten(&text, 12000);
            on_step(Step::Answer(text.clone(), error));
            replies.push(convo.tool_reply(&item, text, error));
        }
        convo.append_replies(replies);
    }
    outcome.summary = format!(
        "{}\n（达到 {} 轮上限，任务可能未完成）",
        outcome.summary, config.max_turns
    );
    Ok(outcome)
}

pub(crate) fn answer_help(
    prompt: String,
    config: &Config,
    net: &dyn Transport,
) -> Result<String, String> {
    const HELP: &str = "你是 Atlas Forge 的使用助手。依据给出的实际菜单、工具和检查结果回答当前问题。说明操作入口、具体步骤和检查方法。区分已确认的项目状态与可能原因。避免编造没有列出的工具或检查结果。你没有修改项目的权限，也不能代替用户执行命令。";
    config.validate()?;
    let convo = Conversation::new(config, prompt, HELP, false);
    let (status, answer) = net.post(&convo.url, &convo.headers, &convo.request(), config.timeout_s)?;
    if !(200..300).contains(&status) {
        let message = answer["error"]["message"].as_str().unwrap_or("服务端没有返回可读的错误信息");
        let masked = if config.key.is_empty() { message.to_owned() } else { message.replace(&config.key, "***") };
        return Err(format!("接口返回 HTTP {status}：{}", shorten(&masked, 280)));
    }
    let answer = turn(config.api, &answer)?;
    if !answer.calls.is_empty() {
        return Err("通用助手仅接收文字答复。".into());
    }
    let text = answer.text.join("\n\n");
    if text.trim().is_empty() {
        return Err("模型没有返回文字。".into());
    }
    Ok(text)
}

#[cfg(test)]
mod help_tests {
    use super::*;

    struct AnswerOnly;

    impl Transport for AnswerOnly {
        fn post(&self, url: &str, _: &[(String, String)], body: &Value, _: u64) -> Result<(u16, Value), String> {
            assert!(url.ends_with("/chat/completions"));
            assert!(body.get("tools").is_none(), "the guide must send no callable tools");
            assert!(body["messages"][0]["content"].as_str().unwrap().contains("使用助手"));
            Ok((200, json!({"choices":[{"message":{"role":"assistant","content":"使用整体移动陆块。"}}]})))
        }
    }

    #[test]
    fn general_assistant_uses_the_provider_without_edit_tools() {
        let cfg = crate::Provider::Compatible.configure("example", "http://127.0.0.1:8999/v1", "", None);
        assert_eq!(answer_help("怎样移动陆块？".into(), &cfg, &AnswerOnly).unwrap(), "使用整体移动陆块。");
    }
}
