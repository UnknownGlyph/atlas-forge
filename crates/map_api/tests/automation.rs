use map_api::{automation, Session};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

#[test]
fn writes_visibility_and_typed_values_in_the_draft_year() {
    use map_core::doc::{EntityKind, Value as FieldValue};

    let mut doc = map_core::Document::default();
    let id = doc.add_entity(EntityKind::Country, "五国测试");
    let code = doc.entities[&id].code.clone();
    let group = doc.add_group("洲名索引");
    let mut session = Session::new(doc, "test");
    let batch = automation::run_batch(
        &mut session,
        &[
            json!({"op":"edit.set_visibility","args":{"labels":true,"cities":false,"group":"洲名索引","visible":false}}),
            json!({"op":"edit.add_field","args":{"key":"population","name":"人口","kind":"number"}}),
            json!({"op":"edit.set_value","context":{"year":2897},"args":{"entity":{"code":code},"key":"population","value":1800}}),
            json!({"op":"edit.import_table","args":{"csv":format!("code,year,income\n{code},2897,340\nUNKNOWN,2897,20\n")}}),
        ],
    );
    assert!(batch.ok, "{batch:?}");
    assert_eq!(
        batch.results[3]["result"]["result"]["unmatched"],
        json!(["UNKNOWN"])
    );
    assert_eq!(batch.results[3]["result"]["result"]["values_set"], 1);
    assert_eq!(session.original().entities.len(), 1);
    assert!(!session.original().group(group).unwrap().hidden);
    let mut draft = session.doc().clone();
    draft.set_year(Some(2897));
    assert!(
        draft.style.show_labels && !draft.style.show_cities && draft.group(group).unwrap().hidden
    );
    assert_eq!(
        draft.raw_value(id, draft.field_by_key("population").unwrap()),
        Some(FieldValue::Num(1800.0))
    );
    assert_eq!(
        draft.raw_value(id, draft.field_by_key("income").unwrap()),
        Some(FieldValue::Num(340.0))
    );
    let bad = automation::request(
        &mut session,
        &json!({
            "op":"edit.set_value","context":{"year":2897},"args":{"entity":id.0,"key":"population","value":"few"}
        }),
    );
    assert_eq!(bad["ok"], false);
    assert_eq!(session.edit_log().len(), 4);
}

#[test]
fn batch_failure_restores_existing_draft_and_log() {
    let mut session = Session::from_arg("--blank").unwrap();
    let city =
        |name| json!({"op":"edit.create","args":{"kind":"city","name":name,"lon":12.0,"lat":20.0}});
    assert_eq!(
        automation::request(&mut session, &city("旧草稿"))["ok"],
        true
    );
    let report = automation::run_batch(
        &mut session,
        &[
            city("应回滚"),
            json!({"op":"edit.add_event","args":{"name":"I","year":9223372036854775807_i64}}),
        ],
    );
    assert_eq!(report.failed_at, Some(2));
    assert_eq!(session.doc().entities.len(), 1);
    assert_eq!(session.edit_log().len(), 1);
    let report = automation::run_batch(
        &mut session,
        &[json!({"op":"draft.save","args":{"path":"not-written.atlas"}})],
    );
    assert_eq!(report.results[0]["error"]["kind"], "external_write");
}

#[test]
fn plug_in_applies_typed_arguments_and_queries_the_draft() {
    let example = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/examples/create-city.atlas-plugin.json");
    let plugin = automation::Plugin::load(&example).unwrap();
    let command = plugin.command("create-city").unwrap();
    let bad = json!({"lat":"25"});
    assert!(command.expand(bad.as_object().unwrap()).is_err());
    let values = json!({"name":"海岬","lat":31.5});
    let requests = command.expand(values.as_object().unwrap()).unwrap();
    let mut session = Session::from_arg("--blank").unwrap();
    let report = automation::run_batch(&mut session, &requests);
    assert!(report.ok, "{report:?}");
    assert_eq!(session.doc().entities.values().next().unwrap().name, "海岬");
    assert!(report.results[1].to_string().contains("海岬"));
}

#[test]
fn stdio_keeps_a_session_and_batch_output_is_atomic() {
    let executable = env!("CARGO_BIN_EXE_atlas-script");
    let mut process = Command::new(executable)
        .arg("--blank")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = process.stdin.take().unwrap();
    writeln!(stdin, "bad json").unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"op":"edit.create","args":{"kind":"city","name":"港湾","lon":12,"lat":20}})
    )
    .unwrap();
    writeln!(
        stdin,
        "{}",
        json!({"id":"inspect","op":"entities.list","args":{"kind":"city"}})
    )
    .unwrap();
    drop(stdin);
    let out = process.wait_with_output().unwrap();
    assert!(out.status.success());
    let replies: Vec<Value> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 3);
    assert_eq!(replies[0]["error"]["kind"], "bad_json");
    assert_eq!(replies[2]["id"], "inspect");
    assert!(replies[2].to_string().contains("港湾"));

    let path =
        std::env::temp_dir().join(format!("atlas-script-atomic-{}.atlas", std::process::id()));
    let create = json!({"op":"edit.create","args":{"kind":"city","name":"港湾","lon":12,"lat":20}});
    let out = Command::new(executable)
        .arg("--blank")
        .arg("--request")
        .arg(create.to_string())
        .arg("--out")
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let original = std::fs::read(&path).unwrap();
    assert!(map_io::project::load(&path)
        .unwrap()
        .entities
        .values()
        .any(|e| e.name == "港湾"));
    let failed = Command::new(executable)
        .arg("--blank")
        .arg("--request")
        .arg(r#"{"op":"unknown"}"#)
        .arg("--out")
        .arg(&path)
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(std::fs::read(&path).unwrap(), original);
    std::fs::remove_file(path).unwrap();
}
