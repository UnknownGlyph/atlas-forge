//! Local script batches and a versioned declarative plug-in format. Both
//! use the draft operations from Session. A batch has no file side effects.
use crate::Session;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::{
    collections::{BTreeMap, HashSet},
    path::Path,
};

pub const AUTOMATION_VERSION: u32 = 1;

pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("无法读取 {}：{e}", path.display()))?;
    serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("{}：{e}", path.display()))
}

pub fn request(session: &mut Session, req: &Value) -> Value {
    if req["op"] == "automation.capabilities" {
        return json!({
            "id":req["id"], "ok":true, "version":session.version(),
            "result":{"schema":AUTOMATION_VERSION, "transport":"jsonl","plugin_schema":1,"tools":crate::mcp::tool_specs()}
        });
    }
    session.handle_mut(req)
}

pub fn invalid_json(error: &str) -> Value {
    json!({"id":null,"ok":false,"error":{"kind":"bad_json","message":error}})
}

#[derive(Debug, Serialize)]
pub struct BatchReport {
    pub ok: bool,
    pub version: String,
    pub results: Vec<Value>,
    /// One-based request number when a request failed.
    pub failed_at: Option<usize>,
}

/// State of the draft immediately after a numbered command.
pub struct VisualStep {
    pub number: usize,
    pub operation: String,
    pub document: map_core::Document,
}

pub fn run_visual_steps(session: &mut Session, requests: &[Value]) -> (BatchReport, Vec<VisualStep>) {
    let mut steps = vec![VisualStep {
        number: 0,
        operation: String::new(),
        document: session.doc().clone(),
    }];
    let report = run_batch_observed(session, requests, |index, request, world| {
        steps.push(VisualStep {
            number: index,
            operation: request["op"].as_str().unwrap_or_default().to_owned(),
            document: world.clone(),
        });
    });
    (report, steps)
}

impl VisualStep {
    pub fn preview(&self, width: u32, height: u32, font: Option<&[u8]>) -> Result<Vec<u8>, String> {
        use map_io::scene::{build_scene, ExportOptions};
        let options = ExportOptions {
            width,
            height,
            projection: self.document.home_view.map_or(map_core::projection::Projection::EqualEarth, |view| view.projection),
            lon0: self.document.home_view.map_or(0.0, |view| view.central_meridian),
            ..ExportOptions::default()
        };
        let theme = map_core::theme::evaluate(&self.document);
        let scene = build_scene(&self.document, &theme, &options);
        map_io::png::render(&scene, font)?.encode_png().map_err(|error| error.to_string())
    }
}

pub fn run_batch(session: &mut Session, requests: &[Value]) -> BatchReport {
    run_batch_observed(session, requests, |_, _, _| {})
}

fn run_batch_observed(
    session: &mut Session,
    requests: &[Value],
    mut observe: impl FnMut(usize, &Value, &map_core::Document),
) -> BatchReport {
    let original_draft = session.draft.clone();
    let original_log = session.log.clone();
    let mut results = Vec::with_capacity(requests.len());
    for (index, req) in requests.iter().enumerate() {
        let reply = if matches!(req["op"].as_str(), Some("preview.render" | "draft.save")) {
            json!({"id":req["id"],"ok":false,"version":session.version(),"error":{
                "kind":"external_write","message":"事务内只读取或修改内存草稿。请在事务完成后另行导出或保存。"
            }})
        } else {
            request(session, req)
        };
        let failed = reply["ok"] != true;
        results.push(reply);
        if failed {
            session.draft = original_draft;
            session.log = original_log;
            return BatchReport {
                ok: false,
                version: session.version().to_owned(),
                results,
                failed_at: Some(index + 1),
            };
        }
        observe(index + 1, req, session.doc());
    }
    BatchReport {
        ok: true,
        version: session.version().to_owned(),
        results,
        failed_at: None,
    }
}

/// Load a JSON array or a versioned script document.
pub fn load_script(path: &Path) -> Result<Vec<Value>, String> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Script {
        schema: u32,
        requests: Vec<Value>,
    }
    let data: Value = read_json(path)?;
    if let Value::Array(values) = data {
        return Ok(values);
    }
    let plan: Script = serde_json::from_value(data).map_err(|e| e.to_string())?;
    schema(plan.schema)?;
    Ok(plan.requests)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plugin {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub commands: Vec<PluginCommand>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginCommand {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub parameters: BTreeMap<String, Parameter>,
    pub requests: Vec<Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub label: String,
    #[serde(rename = "type")]
    pub kind: ParameterKind,
    #[serde(default)]
    pub default: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParameterKind {
    String,
    Integer,
    Number,
    Boolean,
    Json,
}
impl ParameterKind {
    fn valid(self, value: &Value) -> bool {
        match self {
            Self::String => value.is_string(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Number => value.is_number(),
            Self::Boolean => value.is_boolean(),
            Self::Json => true,
        }
    }
    pub fn parse(self, text: &str) -> Result<Value, String> {
        if matches!(self, Self::String) {
            return Ok(json!(text));
        }
        let value = serde_json::from_str::<Value>(text).map_err(|e| e.to_string())?;
        if self.valid(&value) {
            Ok(value)
        } else {
            Err("参数类型不符".into())
        }
    }
}

fn schema(version: u32) -> Result<(), String> {
    if version == AUTOMATION_VERSION {
        Ok(())
    } else {
        Err(format!("不支持的自动化格式版本 {version}。"))
    }
}

fn identifier(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'-' | b'_'))
}

impl Plugin {
    pub fn load(path: &Path) -> Result<Self, String> {
        let plugin: Self = read_json(path)?;
        plugin.validate()?;
        Ok(plugin)
    }

    fn validate(&self) -> Result<(), String> {
        schema(self.schema)?;
        if !identifier(&self.id) || self.name.trim().is_empty() || self.version.trim().is_empty() {
            return Err("插件需要有效的 id、名称和版本。".into());
        }
        if self.commands.is_empty() {
            return Err("插件没有命令。".into());
        }
        let mut ids = HashSet::new();
        for command in &self.commands {
            if !identifier(&command.id)
                || command.label.trim().is_empty()
                || !ids.insert(&command.id)
            {
                return Err(format!("插件命令标识无效或重复：{}", command.id));
            }
            if command.requests.is_empty() {
                return Err(format!("命令 {} 没有地图请求。", command.id));
            }
            for (name, spec) in &command.parameters {
                if !identifier(name)
                    || spec.label.trim().is_empty()
                    || spec
                        .default
                        .as_ref()
                        .is_some_and(|value| !spec.kind.valid(value))
                {
                    return Err(format!("命令 {} 的参数定义无效：{name}", command.id));
                }
            }
            for req in &command.requests {
                let op = req["op"]
                    .as_str()
                    .ok_or_else(|| format!("命令 {} 的 op 须为固定字符串。", command.id))?;
                if op.is_empty() || matches!(op, "preview.render" | "draft.save") {
                    return Err(format!("命令 {} 不支持操作 {op}。", command.id));
                }
                visit_params(req, &|name| {
                    if command.parameters.contains_key(name) {
                        Ok(Value::Null)
                    } else {
                        Err(format!("未定义的插件参数 {name}。"))
                    }
                })?;
            }
        }
        Ok(())
    }

    pub fn command(&self, id: &str) -> Result<&PluginCommand, String> {
        self.commands
            .iter()
            .find(|c| c.id == id)
            .ok_or_else(|| format!("插件没有命令 {id}。"))
    }
}

impl PluginCommand {
    pub fn expand(&self, input: &Map<String, Value>) -> Result<Vec<Value>, String> {
        if let Some(name) = input.keys().find(|k| !self.parameters.contains_key(*k)) {
            return Err(format!("未知插件参数 {name}。"));
        }
        let mut values = BTreeMap::new();
        for (name, param) in &self.parameters {
            let value = input
                .get(name)
                .or(param.default.as_ref())
                .ok_or_else(|| format!("缺少插件参数 {name}。"))?;
            if !param.kind.valid(value) {
                return Err(format!("插件参数 {name} 的类型不符。"));
            }
            values.insert(name.as_str(), value);
        }
        self.requests
            .iter()
            .map(|req| {
                visit_params(req, &|name| {
                    values
                        .get(name)
                        .map(|v| (*v).clone())
                        .ok_or_else(|| format!("未定义的插件参数 {name}。"))
                })
            })
            .collect()
    }
}

fn visit_params(
    value: &Value,
    parameter: &impl Fn(&str) -> Result<Value, String>,
) -> Result<Value, String> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| visit_params(item, parameter))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        Value::Object(map) if map.contains_key("$param") => {
            let name = map["$param"]
                .as_str()
                .filter(|_| map.len() == 1)
                .ok_or("参数引用须形如 {\"$param\":\"名称\"}")?;
            parameter(name)
        }
        Value::Object(map) => map
            .iter()
            .map(|(key, item)| Ok((key.clone(), visit_params(item, parameter)?)))
            .collect::<Result<Map<_, _>, _>>()
            .map(Value::Object),
        _ => Ok(value.clone()),
    }
}
