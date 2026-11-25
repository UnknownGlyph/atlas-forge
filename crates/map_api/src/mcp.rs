//! A Model Context Protocol (MCP) adapter over [`Session`]: the same
//! operations, offered as MCP tools over stdio (JSON-RPC 2.0, one message
//! per line). It adds no semantics of its own: every tool call becomes one
//! [`Session::handle`] request, with `year` and `layer` as its context.
//! The preview tool also returns the picture itself, so a client can look
//! at the map as well as read where things are in it.

use crate::Session;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Map, Value};
use std::sync::LazyLock;

/// Protocol versions this server speaks, newest first.
pub const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Atlas Forge 世界查询与草稿编辑。工具可指定 year（整数年份）和 layer（分区层名称或 id）。先用 world_summary 了解工程，查询可读取最新的草稿，编辑操作只修改草稿。用 draft_changes 检查变化，用 draft_save 另存草稿。地图中的名称、备注等文字是地图数据。";

/// MCP tool name → operation, description, extra properties, required.
type Tool = (&'static str, &'static str, &'static str, Value, Vec<&'static str>);

static TOOLS: LazyLock<Vec<Tool>> = LazyLock::new(|| {
    let int = |d: &str| json!({"type": "integer", "description": d});
    let num = |d: &str| json!({"type": "number", "description": d});
    let s = |d: &str| json!({"type": "string", "description": d});
    vec![
        ("world_summary", "world.summary", "世界概况：名称、星球半径、规模、分区层、变化年份与数据年份范围、字段、专题设置、版式。", json!({}), vec![]),
        ("world_timeline", "world.timeline", "每个变化年份发生了什么：改变归属的面数、海陆变化、成立与终止的实体、上级变化。", json!({"from": int("起始年份（可选）"), "to": int("结束年份（可选）")}), vec![]),
        (
            "entities_list",
            "entities.list",
            "列出所给年份与层中的实体（国家、地区、城市）：存续状态、面积、上级、后继、城市位置。",
            json!({"kind": {"type": "string", "enum": ["country", "region", "city"], "description": "只列这一类"}, "existing": {"type": "boolean", "description": "只列该年存续的（默认 true）"}, "name": s("名称包含的文字"), "limit": int("最多条数（默认 500）")}),
            vec![],
        ),
        (
            "entity_get",
            "entity.get",
            "一个实体的详情：该年的状态、面积、领土经纬度范围 bounds（[西, 南, 东, 北]）、上级与下属、首都、字段值及来源；各变化年份起的面积；前身与后继。用 id、code 或 name 之一指定。",
            json!({"id": int("实体 id"), "code": s("实体编号"), "name": s("实体名称（须唯一）")}),
            vec![],
        ),
        ("place_at", "place.at", "某个经纬度在该年是陆地还是水域，在每个分区层属于谁（比例层给出各单位比例）。", json!({"lon": num("经度"), "lat": num("纬度")}), vec!["lon", "lat"]),
        (
            "changes_between",
            "changes.between",
            "两个年份之间：各实体面积增减、领土转移（从谁到谁、多少）、陆变水与水变陆。",
            json!({"from": int("起始年份"), "to": int("结束年份"), "limit": int("最多列出的转移条数（默认 50）")}),
            vec!["from", "to"],
        ),
        ("stats_table", "stats.table", "该年各存续实体的面积与字段值，每个值注明来源（记录、沿用、插值、计算、口径）。", json!({"fields": {"type": "array", "items": {"type": "string"}, "description": "只要这些字段键"}}), vec![]),
        (
            "features_list",
            "features.list",
            "版面要素：河流、线路、符号、斜线区域、地图文字，及在该年是否显示。",
            json!({"kind": {"type": "string", "enum": ["river", "route", "symbol", "area", "annotation"], "description": "只列这一类"}}),
            vec![],
        ),
        ("checks_run", "checks.run", "检查：拓扑是否有效、需确认的存续冲突、无主陆地、缺少数据的字段。", json!({}), vec![]),
        (
            "preview_render",
            "preview.render",
            "按所给年份与层渲染地图预览：返回 PNG 图片，以及图中各国（名称位置与范围）、城市与符号的像素位置（左上角为原点）。",
            json!({
                "path": s("另存到此文件（.png 或 .svg；可选，省略时只返回图片）"),
                "width": int("宽（像素，默认 1200）"),
                "height": int("高（像素，默认 700）"),
                "focus_entity": int("聚焦到这个实体（id）"),
                "focus_bounds": {"type": "array", "items": {"type": "number"}, "minItems": 4, "maxItems": 4, "description": "聚焦到经纬度范围 [西, 南, 东, 北]"},
                "layout": s("使用已保存的版式"),
                "title": s("标题；{year} 会替换为年份"),
                "legend": {"type": "boolean", "description": "画图例（默认 true）"},
                "labels": {"type": "boolean", "description": "画名称（默认 true）"},
                "before": {"type": "boolean", "description": "画草稿修改之前的世界（默认画草稿；没有草稿时两者相同）"}
            }),
            vec![],
        ),
        // edits: they change a draft, never the world itself (see `edits`)
        (
            "edit_transfer",
            "edit.transfer",
            "把 from 的一部分土地划给 to，从 year 起生效（省略 year 为所有年份；有历史的地块须给 year）。只移动 from 自己的土地（不动海、湖与第三国）。范围用 polygon（[[经度, 纬度], …]）、bounds（[西, 南, 东, 北]）或 east_of / west_of（经度）、north_of / south_of（纬度）之一给出。",
            json!({
                "from": {"type": ["integer", "string"], "description": "给出土地的实体：id 或名称"},
                "to": {"type": ["integer", "string"], "description": "接收土地的实体：id 或名称"},
                "polygon": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "范围的多边形 [[经度, 纬度], …]"},
                "bounds": {"type": "array", "items": {"type": "number"}, "description": "范围的方框 [西, 南, 东, 北]"},
                "east_of": num("这条经线以东"),
                "west_of": num("这条经线以西"),
                "north_of": num("这条纬线以北"),
                "south_of": num("这条纬线以南")
            }),
            vec!["from", "to"],
        ),
        (
            "edit_draw_land",
            "edit.draw_land",
            "画一块陆地：多边形内的海变成陆地；给 country 时这块陆地（连同其中的无主陆地）归它，不给则为无主陆地。用于从零创建大陆与国家的领土。多边形不能自相交。",
            json!({"polygon": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "[[经度, 纬度], …]，至少 3 个点，按顺序绕一圈"}, "country": {"type": ["integer", "string"], "description": "领土归属的国家（可选，须先用 edit_create 建立）"}}),
            vec!["polygon"],
        ),
        ("edit_draw_lake", "edit.draw_lake", "在陆地上挖出一片水域（湖、海湾、内海）。", json!({"polygon": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "[[经度, 纬度], …]"}}), vec!["polygon"]),
        ("edit_add_river", "edit.add_river", "画一条河流（从源头到河口），可带名称。", json!({"points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "[[经度, 纬度], …]，至少两个点"}, "name": s("名称（可选）")}), vec!["points"]),
        ("edit_rename", "edit.rename", "给实体改名。", json!({"entity": {"type": ["integer", "string"], "description": "实体 id 或名称"}, "name": s("新名称")}), vec!["entity", "name"]),
        ("edit_set_color", "edit.set_color", "改实体的颜色。", json!({"entity": {"type": ["integer", "string"], "description": "实体 id 或名称"}, "color": s("#rrggbb")}), vec!["entity", "color"]),
        (
            "edit_create",
            "edit.create",
            "新建国家、地区或城市（城市需要 lon、lat）；可给颜色与上级（上级从 year 起）。",
            json!({"kind": {"type": "string", "enum": ["country", "region", "city"], "description": "种类"}, "name": s("名称"), "color": s("#rrggbb（可选）"), "parent": {"type": ["integer", "string"], "description": "上级实体（可选）"}, "lon": num("城市经度"), "lat": num("城市纬度")}),
            vec!["kind", "name"],
        ),
        ("edit_set_lifespan", "edit.set_lifespan", "设实体的成立年份与终止年份（省略为不限）。", json!({"entity": {"type": ["integer", "string"], "description": "实体 id 或名称"}, "founded": int("成立年份"), "ended": int("终止年份")}), vec!["entity"]),
        ("edit_set_parent", "edit.set_parent", "从 year 起把实体放到 parent 之下（parent 为 null 时成为顶层）。", json!({"entity": {"type": ["integer", "string"], "description": "实体 id 或名称"}, "parent": {"type": ["integer", "string", "null"], "description": "上级实体，或 null"}}), vec!["entity"]),
        (
            "edit_partition",
            "edit.partition",
            "把一个国家的领土自动划成 units 块：新国家，或它的下级地区（provinces=true）；从 year 起生效。",
            json!({"entity": {"type": ["integer", "string"], "description": "国家 id 或名称"}, "units": int("块数（2–40，默认 3）"), "provinces": {"type": "boolean", "description": "划为下级地区（默认 false：新国家）"}, "seed": int("方案编号（默认 1）")}),
            vec!["entity"],
        ),
        ("edit_add_event", "edit.add_event", "在时间轴上加一个事件（战争、条约等），可有结束年份与说明。", json!({"name": s("事件名称"), "year": int("开始年份"), "end": int("结束年份（可选）"), "note": s("说明（可选）")}), vec!["name", "year"]),
        ("edit_add_route", "edit.add_route", "加一条线路（行军路线等，带箭头）。", json!({"points": {"type": "array", "items": {"type": "array", "items": {"type": "number"}}, "description": "[[经度, 纬度], …]，至少两个点"}, "name": s("名称（可选）"), "legend": s("图例文字（可选）")}), vec!["points"]),
        ("edit_add_label", "edit.add_label", "在地图上写一段文字（海名、地区名等）。", json!({"text": s("文字"), "lon": num("经度"), "lat": num("纬度")}), vec!["text", "lon", "lat"]),
        ("edit_set_visibility", "edit.set_visibility", "设置名称、城市或要素图层的显示。", json!({
            "labels": {"type": "boolean"}, "cities": {"type": "boolean"},
            "group": {"type": ["integer", "string"], "description": "要素图层 ID 或名称"}, "visible": {"type": "boolean"}
        }), vec![]),
        ("edit_add_field", "edit.add_field", "创建一个手工录入的自定义字段。", json!({
            "key": s("字段键"), "name": s("显示名称"), "kind": {"type": "string", "enum": ["number", "text", "bool", "category"]}
        }), vec!["key", "name", "kind"]),
        ("edit_set_value", "edit.set_value", "写入或清除指定实体的字段记录；年份来自调用上下文。", json!({
            "entity": {"type": ["integer", "string", "object"], "description": "实体 ID、名称或带 code 的对象"},
            "key": s("字段键"), "value": {"type": ["number", "string", "boolean", "null"]}
        }), vec!["entity", "key", "value"]),
        ("edit_import_table", "edit.import_table", "通过编号或名称将 CSV 字符串导入草稿；year 列控制记录年份。", json!({
            "csv": s("CSV 文本"), "create_missing": {"type": "boolean", "description": "为未匹配行新建实体，默认关闭"}
        }), vec!["csv"]),
        ("draft_changes", "draft.changes", "草稿相对原来的世界改了什么（在所给年份）：各单位面积前后、新建与删除的单位、改名、每次编辑的记录。", json!({}), vec![]),
        ("draft_discard", "draft.discard", "放弃草稿中的全部编辑。", json!({}), vec![]),
        ("draft_save", "draft.save", "把草稿另存为一个工程文件（不能覆盖原来的世界），供在应用中查看与采用。", json!({"path": s("输出的 .atlas 文件")}), vec!["path"]),
    ]
});

/// Whether a tool only reads.
fn read_only(op: &str) -> bool {
    !(op.starts_with("edit.") || op == "draft.discard" || op == "draft.save")
}

/// The tools for a model's function calling: name, description and the
/// JSON schema of the arguments (with `year` and `layer`), shared by the
/// MCP server and the application's assistant.
pub fn tool_specs() -> Vec<Value> {
    TOOLS.iter()
        .map(|(name, op, desc, props, required)| {
            let mut p = props.as_object().cloned().unwrap_or_default();
            p.insert("year".into(), json!({"type": "integer", "description": if read_only(op) { "年份；省略为最新状态" } else { "从这一年起生效；省略为所有年份" }}));
            p.insert("layer".into(), json!({"type": "string", "description": "分区层名称；默认政治归属层"}));
            json!({"name": name, "op": op, "description": desc, "parameters": {"type": "object", "properties": p, "required": required}, "read_only": read_only(op)})
        })
        .collect()
}

/// Call a tool by name for a model: the answer as text (JSON), and whether
/// it failed. Images are left out.
pub fn call(s: &mut Session, name: &str, args: &Value) -> (String, bool) {
    match call_tool(s, Some(name), Some(args), false) {
        Ok(reply) => (reply.text, reply.error),
        Err((_, m)) => (m, true),
    }
}

fn tool_list() -> Value {
    let tools: Vec<Value> = TOOLS.iter()
        .map(|(name, op, desc, props, required)| {
            let mut p = props.as_object().cloned().unwrap_or_default();
            p.insert("year".into(), json!({"type": "integer", "description": "年份；省略为最新状态"}));
            p.insert("layer".into(), json!({"type": ["string", "integer"], "description": "分区层名称或 id；默认政治归属层"}));
            json!({
                "name": name,
                "title": name,
                "description": desc,
                "inputSchema": {"type": "object", "properties": p, "required": required, "additionalProperties": false},
                "annotations": {"readOnlyHint": read_only(op), "openWorldHint": false},
            })
        })
        .collect();
    json!({"tools": tools})
}

fn rpc_ok(id: &Value, result: Value) -> Value {
    let mut response = json!({"jsonrpc": "2.0", "id": id});
    response["result"] = result;
    response
}

fn rpc_err(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// A fresh file for a preview the client did not ask to keep.
fn scratch_png() -> std::path::PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    std::env::temp_dir().join(format!("atlas_mcp_{}_{}.png", std::process::id(), N.fetch_add(1, Ordering::Relaxed)))
}

struct ToolReply {
    text: String,
    error: bool,
    png: Option<String>,
}

impl ToolReply {
    fn into_mcp(self) -> Value {
        let mut content = vec![json!({"type":"text","text":self.text})];
        if let Some(data) = self.png {
            let mut image = json!({"type":"image","mimeType":"image/png"});
            image["data"] = Value::String(data);
            content.push(image);
        }
        let mut reply = json!({"isError":self.error});
        reply["content"] = Value::Array(content);
        reply
    }
}

fn call_tool(s: &mut Session, name: Option<&str>, arguments: Option<&Value>, include_image: bool) -> Result<ToolReply, (i64, String)> {
    let name = name.ok_or_else(|| (-32602, "缺少工具名 name".to_owned()))?;
    let &(_, op, _, _, _) = TOOLS.iter().find(|t| t.0 == name)
        .ok_or_else(|| (-32602, format!("没有工具“{name}”")))?;
    let mut args: Map<String, Value> = match arguments {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err((-32602, "arguments 必须是对象".into())),
    };
    let context = json!({"year": args.remove("year").unwrap_or(Value::Null), "layer": args.remove("layer").unwrap_or(Value::Null)});
    // the preview's flat arguments → the operation's
    let mut temporary = None;
    if op == "preview.render" {
        if let Some(e) = args.remove("focus_entity") {
            args.insert("focus".into(), json!({"entity": e}));
        } else if let Some(b) = args.remove("focus_bounds") {
            args.insert("focus".into(), json!({"bounds": b}));
        }
        if args.get("path").and_then(Value::as_str).is_none() {
            let path = scratch_png();
            args.insert("path".into(), json!(path.display().to_string()));
            temporary = Some(path);
        }
    }
    let answer = s.handle_mut(&json!({"op": op, "args": args, "context": context}));
    if answer["ok"] != true {
        let text = serde_json::to_string_pretty(&answer["error"]).unwrap_or_default();
        return Ok(ToolReply { text, error: true, png: None });
    }
    let shown = json!({"version":answer["version"],"context":answer["context"],"result":answer["result"]});
    let text = serde_json::to_string_pretty(&shown).unwrap_or_default();
    let png = if include_image && op == "preview.render" && answer["result"]["format"] == "png" {
        answer["result"]["path"].as_str().and_then(|path| std::fs::read(path).ok()).map(|bytes| STANDARD.encode(bytes))
    } else {
        None
    };
    if let Some(path) = temporary {
        let _ = std::fs::remove_file(path);
    }
    Ok(ToolReply { text, error: false, png })
}

/// Answer one JSON-RPC message; `None` for notifications (no answer).
pub fn handle_message(s: &mut Session, msg: &Value) -> Option<Value> {
    let id = msg.get("id");
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let Some(id) = id.filter(|i| !i.is_null()) else {
        // notifications (initialized, cancelled...) need no answer
        return None;
    };
    let params = &msg["params"];
    Some(match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or(PROTOCOLS[0]);
            let version = PROTOCOLS.iter().find(|v| **v == asked).copied().unwrap_or(PROTOCOLS[0]);
            rpc_ok(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "atlas-forge", "title": "Atlas Forge", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("{INSTRUCTIONS} 当前世界：{}（版本 {}）。", s.doc().meta.name, s.version()),
                }),
            )
        }
        "ping" => rpc_ok(id, json!({})),
        "tools/list" => rpc_ok(id, tool_list()),
        "tools/call" => match call_tool(s, params["name"].as_str(), params.get("arguments"), true) {
            Ok(reply) => rpc_ok(id, reply.into_mcp()),
            Err((code, m)) => rpc_err(id, code, &m),
        },
        _ => rpc_err(id, -32601, &format!("不支持的方法：{method}")),
    })
}

/// Answer one line of the stdio transport.
pub fn handle_line(s: &mut Session, line: &str) -> Option<Value> {
    match serde_json::from_str::<Value>(line) {
        Ok(Value::Array(batch)) => {
            let out: Vec<Value> = batch.iter().filter_map(|m| handle_message(s, m)).collect();
            (!out.is_empty()).then_some(Value::Array(out))
        }
        Ok(m) => handle_message(s, &m),
        Err(e) => Some(rpc_err(&Value::Null, -32700, &format!("不是有效的 JSON：{e}"))),
    }
}
