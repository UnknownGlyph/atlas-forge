//! Offline runner for JSONL, transactional JSON scripts and plug-in commands.
use map_api::{automation, Session};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    io::{self, BufRead, Write},
    path::Path,
};

const HELP: &str =
    "atlas-script <world.atlas | --blank | --demo | --test-world> [mode] [--out NEW.atlas]
  (no mode)                                  read JSONL from stdin
  --request JSON                             run one JSON request
  --file PLAN.json                            run a transactional JSON script
  --steps NEW_DIR                             export one preview per step with --file
  --plugin FILE --command ID [--params JSON | --params-file FILE]
                                             run a declarative plug-in command
With --out, only successful batches save a draft. Outputs and errors are JSON.";
const FLAGS: &[&str] = &[
    "--out",
    "--request",
    "--file",
    "--plugin",
    "--command",
    "--params",
    "--params-file",
    "--steps",
];

fn options(args: &[String]) -> Result<BTreeMap<&str, &str>, String> {
    if !args.len().is_multiple_of(2) {
        return Err("每个选项需要一个值。".into());
    }
    let mut opts = BTreeMap::new();
    for pair in args.as_chunks::<2>().0 {
        if !FLAGS.contains(&pair[0].as_str())
            || opts.insert(pair[0].as_str(), pair[1].as_str()).is_some()
        {
            return Err(format!("未知或重复选项 {}。", pair[0]));
        }
    }
    let has = |flag: &str| opts.contains_key(flag);
    if ["--request", "--file", "--plugin"]
        .into_iter()
        .filter(|k| has(k))
        .count()
        > 1
        || has("--plugin") != has("--command")
        || (!has("--plugin") && (has("--params") || has("--params-file")))
        || (has("--params") && has("--params-file"))
        || (has("--steps") && !has("--file"))
    {
        return Err("请选择一种运行方式，并在插件模式下指定命令。".into());
    }
    Ok(opts)
}

fn print(report: &Value) -> Result<(), String> {
    let mut out = io::stdout().lock();
    writeln!(out, "{report}")
        .and_then(|_| out.flush())
        .map_err(|e| e.to_string())
}

fn line(line: &str) -> Result<Value, Value> {
    serde_json::from_str(line.trim_start_matches('\u{feff}'))
        .map_err(|error| automation::invalid_json(&error.to_string()))
}

fn finish_batch(
    session: &mut Session,
    requests: &[Value],
    out: Option<&str>,
    preview_dir: Option<&str>,
) -> Result<bool, String> {
    let (batch, steps) = if preview_dir.is_some() {
        automation::run_visual_steps(session, requests)
    } else {
        (automation::run_batch(session, requests), Vec::new())
    };
    let mut report = serde_json::to_value(&batch).map_err(|e| e.to_string())?;
    if let Some(directory) = preview_dir {
        let manifest = write_previews(Path::new(directory), &steps, &batch)?;
        report["steps"] = json!(manifest.display().to_string());
    }
    if let (true, Some(path)) = (batch.ok, out) {
        let saved = session.handle_mut(&json!({"op":"draft.save","args":{"path":path}}));
        if saved["ok"] == true {
            report["saved"] = saved["result"]["saved"].clone();
        } else {
            report["ok"] = json!(false);
            report["error"] = saved["error"].clone();
        }
    }
    print(&report)?;
    Ok(report["ok"] == true)
}

fn write_previews(
    directory: &Path,
    steps: &[automation::VisualStep],
    report: &automation::BatchReport,
) -> Result<std::path::PathBuf, String> {
    std::fs::create_dir(directory).map_err(|error| format!("{}：{error}", directory.display()))?;
    let font = map_io::png::find_system_font();
    let mut frames = Vec::with_capacity(steps.len());
    for step in steps {
        let file = format!("step-{:04}.png", step.number);
        let bytes = step.preview(1000, 560, font.as_deref())?;
        map_io::recovery::write_atomic(&directory.join(&file), &bytes).map_err(|e| e.to_string())?;
        frames.push(json!({"number":step.number, "operation":step.operation, "image":file}));
    }
    let manifest = directory.join("steps.json");
    let bytes = serde_json::to_vec_pretty(&json!({"steps": frames, "ok":report.ok, "failed_at":report.failed_at}))
        .map_err(|e| e.to_string())?;
    map_io::recovery::write_atomic(&manifest, &bytes).map_err(|e| e.to_string())?;
    Ok(manifest)
}

fn execute(src: &str, opts: BTreeMap<&str, &str>) -> Result<bool, String> {
    let mut session = Session::from_arg(src)?;
    let opts = |key| opts.get(key).copied();
    let out = opts("--out");
    if let Some(path) = opts("--plugin") {
        let plugin = automation::Plugin::load(Path::new(path))?;
        let command = plugin.command(opts("--command").unwrap())?;
        let parameters: Map<String, Value> = match (opts("--params"), opts("--params-file")) {
            (Some(input), _) => serde_json::from_str(input).map_err(|e| e.to_string())?,
            (_, Some(path)) => automation::read_json(Path::new(path))?,
            _ => Map::new(),
        };
        return finish_batch(&mut session, &command.expand(&parameters)?, out, None);
    }
    if let Some(path) = opts("--file") {
        return finish_batch(
            &mut session,
            &automation::load_script(Path::new(path))?,
            out,
            opts("--steps"),
        );
    }
    if let Some(input) = opts("--request") {
        let req = match line(input) {
            Ok(req) => req,
            Err(error) => {
                print(&error)?;
                return Ok(false);
            }
        };
        if out.is_some() {
            return finish_batch(&mut session, &[req], out, None);
        }
        let result = automation::request(&mut session, &req);
        print(&result)?;
        return Ok(result["ok"] == true);
    }
    let mut pending = Vec::new();
    for input in io::stdin().lock().lines() {
        let input = input.map_err(|e| e.to_string())?;
        if input.trim().is_empty() {
            continue;
        }
        match (out.is_some(), line(&input)) {
            (true, Ok(req)) => pending.push(req),
            (true, Err(error)) => {
                print(&error)?;
                return Ok(false);
            }
            (false, Ok(req)) => print(&automation::request(&mut session, &req))?,
            (false, Err(error)) => print(&error)?,
        }
    }
    if out.is_some() {
        return finish_batch(&mut session, &pending, out, None);
    }
    Ok(true)
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if matches!(args.first().map(String::as_str), Some("-h" | "--help")) {
        println!("{HELP}");
        return;
    }
    let Some((src, rest)) = args.split_first() else {
        eprintln!("{HELP}");
        std::process::exit(2);
    };
    let opts = options(rest).unwrap_or_else(|error| {
        eprintln!("{error}\n{HELP}");
        std::process::exit(2)
    });
    match execute(src, opts) {
        Ok(true) => {}
        Ok(false) => std::process::exit(1),
        Err(error) => {
            let _ = print(&json!({"ok":false,"error":{"kind":"failed","message":error}}));
            std::process::exit(1);
        }
    }
}
