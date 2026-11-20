//! The MCP server as a client sees it: the real `atlas-mcp` binary driven
//! over stdio through a whole session.

use serde_json::{json, Value};
use std::io::Write;
use std::process::{Command, Stdio};

/// Send `msgs` (one per line) to `atlas-mcp --demo`; the answers by id.
fn session(msgs: &[Value], raw: &[&str]) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_atlas-mcp"))
        .arg("--demo")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    {
        let stdin = child.stdin.as_mut().unwrap();
        for m in msgs {
            writeln!(stdin, "{m}").unwrap();
        }
        for r in raw {
            writeln!(stdin, "{r}").unwrap();
        }
    }
    drop(child.stdin.take());
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    String::from_utf8(out.stdout).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

fn by_id<'a>(answers: &'a [Value], id: i64) -> &'a Value {
    answers.iter().find(|a| a["id"] == id).unwrap_or_else(|| panic!("no answer for {id}: {answers:?}"))
}

#[test]
fn a_whole_client_session() {
    let call = |id: i64, name: &str, args: Value| json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": name, "arguments": args}});
    let msgs = vec![
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26", "capabilities": {}, "clientInfo": {"name": "test", "version": "0"}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        call(3, "world_summary", json!({})),
        call(4, "entities_list", json!({"kind": "country", "year": 1900})),
        call(5, "preview_render", json!({"year": 1900, "width": 480, "height": 280})),
        call(6, "place_at", json!({"lat": 10})),
        call(7, "no_such_tool", json!({})),
        json!({"jsonrpc": "2.0", "id": 8, "method": "resources/list"}),
        json!({"jsonrpc": "2.0", "id": 9, "method": "ping"}),
        call(10, "entities_list", json!({"layer": "不存在的层"})),
    ];
    let a = session(&msgs, &["{not json"]);
    // one answer per request; none for the notification
    assert_eq!(a.len(), 11, "{a:?}");

    let init = &by_id(&a, 1)["result"];
    assert_eq!(init["protocolVersion"], "2025-03-26", "the client's version when supported");
    assert_eq!(init["serverInfo"]["name"], "atlas-forge");
    assert!(init["capabilities"]["tools"].is_object());

    let tools = by_id(&a, 2)["result"]["tools"].as_array().unwrap().clone();
    // ten that read; the edits (on a draft) and the draft's own tools
    assert_eq!(tools.len(), 30);
    for name in ["edit_set_visibility", "edit_add_field", "edit_set_value", "edit_import_table"] {
        assert!(tools.iter().any(|tool| tool["name"] == name), "{name}");
    }
    for t in &tools {
        let name = t["name"].as_str().unwrap();
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'), "{name}");
        let writes = name.starts_with("edit_") || name == "draft_discard" || name == "draft_save";
        assert_eq!(t["annotations"]["readOnlyHint"], !writes, "{name}");
        assert_eq!(t["inputSchema"]["type"], "object");
        assert!(t["inputSchema"]["properties"]["year"].is_object() && t["inputSchema"]["properties"]["layer"].is_object());
    }

    let text = |id: i64| -> Value {
        let r = &by_id(&a, id)["result"];
        serde_json::from_str(r["content"][0]["text"].as_str().unwrap()).unwrap()
    };
    assert_eq!(by_id(&a, 3)["result"]["isError"], false);
    assert!(text(3)["result"]["name"].as_str().unwrap().contains("示例世界"));
    let list = text(4);
    assert_eq!(list["context"]["year"], 1900);
    assert!(list["result"]["count"].as_u64().unwrap() >= 8);

    // the preview: positions as text, and the picture itself
    let p = &by_id(&a, 5)["result"]["content"];
    assert_eq!(p[1]["type"], "image");
    assert_eq!(p[1]["mimeType"], "image/png");
    assert!(p[1]["data"].as_str().unwrap().starts_with("iVBORw0KGgo"), "PNG signature in base64");
    let objs = &text(5)["result"]["objects"];
    assert!(objs.as_array().unwrap().iter().any(|o| o["kind"] == "country"));
    // the scratch picture is not left behind
    assert!(!std::path::Path::new(text(5)["result"]["path"].as_str().unwrap()).exists());

    // an operation's failure is a tool result marked as an error
    assert_eq!(by_id(&a, 6)["result"]["isError"], true);
    assert!(by_id(&a, 6)["result"]["content"][0]["text"].as_str().unwrap().contains("bad_request"));
    assert_eq!(by_id(&a, 10)["result"]["isError"], true);
    assert!(by_id(&a, 10)["result"]["content"][0]["text"].as_str().unwrap().contains("not_found"));
    // protocol errors are JSON-RPC errors
    assert_eq!(by_id(&a, 7)["error"]["code"], -32602);
    assert_eq!(by_id(&a, 8)["error"]["code"], -32601);
    assert_eq!(by_id(&a, 9)["result"], json!({}));
    assert!(a.iter().any(|x| x["id"].is_null() && x["error"]["code"] == -32700));
}

#[test]
fn unknown_protocol_versions_get_the_newest() {
    let a = session(&[json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "1999-01-01"}})], &[]);
    assert_eq!(a[0]["result"]["protocolVersion"], map_api::mcp::PROTOCOLS[0]);
}
