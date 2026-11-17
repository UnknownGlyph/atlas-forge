//! atlas-agent: run a task on a world with a language model, without the
//! application, and keep what it made apart from the world:
//!
//!   atlas-agent <world.atlas | --demo | --test-world | --blank> "<task>" [--provider gemini|deepseek]
//!               [--model NAME] [--effort high] [--out FOLDER] [--year Y] [--turns N]
//!
//! The key comes from GEMINI_API_KEY or DEEPSEEK_API_KEY. Writes into the
//! folder: the draft (草稿.atlas), previews before and after (前.png, 后.png
//! for --year, or the latest state) and the log (记录.txt).

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (Some(src), Some(task)) = (args.get(1), args.get(2)) else {
        eprintln!("用法：atlas-agent <工程.atlas | --demo | --test-world> \"任务\" [--provider gemini|deepseek] [--model 名称] [--effort high] [--out 文件夹] [--year 年份]");
        std::process::exit(2);
    };
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let provider = opt("--provider").unwrap_or_else(|| "gemini".into());
    let effort = opt("--effort");
    let cfg = match provider.as_str() {
        "gemini" => map_ai::Config::gemini(&opt("--model").unwrap_or_else(|| "gemini-4-hydrogen".into()), &std::env::var("GEMINI_API_KEY").unwrap_or_default(), effort.as_deref().or(Some("high"))),
        "deepseek" => map_ai::Config::deepseek(&opt("--model").unwrap_or_else(|| "deepseek-flash".into()), &std::env::var("DEEPSEEK_API_KEY").unwrap_or_default(), effort.as_deref()),
        p => {
            eprintln!("不认识的提供方：{p}");
            std::process::exit(2);
        }
    };
    let mut cfg = cfg;
    if let Some(n) = opt("--turns").and_then(|n| n.parse().ok()) {
        cfg.max_turns = n;
    }
    let out_dir = std::path::PathBuf::from(opt("--out").unwrap_or_else(|| "atlas-agent-out".into()));
    let _ = std::fs::create_dir_all(&out_dir);
    let mut session = match map_api::Session::from_arg(src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let mut log = vec![format!("模型：{}\n任务：{task}", cfg.label())];
    let t = std::time::Instant::now();
    let net = map_ai::default_transport();
    let result = map_ai::run(&mut session, task, &cfg, net.as_ref(), &mut |step| {
        let line = match step {
            map_ai::Step::Said(t) => format!("【模型】{t}"),
            map_ai::Step::Call(n, a) => format!("【调用】{n} {a}"),
            map_ai::Step::Answer(t, err) => format!("【{}】{}", if err { "失败" } else { "结果" }, t.chars().take(600).collect::<String>()),
        };
        println!("{line}");
        log.push(line);
    });
    match result {
        Ok(o) => {
            log.push(format!("用时 {:.0} s，{} 轮，{} 次调用", t.elapsed().as_secs_f64(), o.turns, o.calls));
            println!("{}", log.last().unwrap());
        }
        Err(e) => {
            eprintln!("失败：{e}");
            log.push(format!("失败：{e}"));
        }
    }
    let year = opt("--year");
    let ctx = |before: bool| serde_json::json!({"path": out_dir.join(if before { "前.png" } else { "后.png" }).display().to_string(), "before": before, "width": 1400, "height": 760, "year": year.as_ref().and_then(|y| y.parse::<i64>().ok())});
    if session.draft().is_some() {
        for before in [true, false] {
            let (text, err) = map_api::mcp::call(&mut session, "preview_render", &ctx(before));
            if err {
                eprintln!("预览失败：{text}");
            }
        }
        let (text, _) = map_api::mcp::call(&mut session, "draft_changes", &serde_json::json!({"year": year.as_ref().and_then(|y| y.parse::<i64>().ok())}));
        log.push(format!("草稿的修改：{text}"));
        let (text, err) = map_api::mcp::call(&mut session, "draft_save", &serde_json::json!({"path": out_dir.join("草稿.atlas").display().to_string()}));
        if err {
            eprintln!("保存草稿失败：{text}");
        }
    } else {
        log.push("没有修改".into());
    }
    let _ = std::fs::write(out_dir.join("记录.txt"), log.join("\n"));
    println!("输出在 {}", out_dir.display());
}
