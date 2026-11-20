//! atlas-agent: run a task on a world with a language model, without the
//! application, and keep what it made apart from the world:
//!
//!   atlas-agent <world.atlas | --demo | --test-world | --blank> "<task>" [--provider ID] [--base-url URL]
//!               [--model NAME] [--effort high] [--out FOLDER] [--year Y] [--turns N]
//!
//! The key comes from the provider environment variable or --key-env. Writes into the
//! folder: the draft (草稿.atlas), previews before and after (前.png, 后.png
//! for --year, or the latest state) and the log (记录.txt).

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--providers") {
        for p in map_ai::Provider::ALL {
            println!("{}\t{}\t{}\t{}", p.id(), p.name(), p.key_env(), p.default_url());
        }
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    let (Some(src), Some(task)) = (args.get(1), args.get(2)) else {
        eprintln!("用法：atlas-agent <工程.atlas | --demo | --test-world> \"任务\" [--provider ID] [--base-url URL] [--model 名称] [--effort high] [--out 文件夹] [--year 年份]");
        std::process::exit(2);
    };
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    let id = opt("--provider").unwrap_or_else(|| "gemini".into());
    let Some(provider) = map_ai::Provider::parse(&id) else {
        eprintln!("未知提供方 {id}。用 atlas-agent --providers 列出可用提供方。");
        std::process::exit(2);
    };
    let endpoint = opt("--base-url").or_else(|| std::env::var("ATLAS_AI_BASE_URL").ok()).unwrap_or_default();
    let env = opt("--key-env").unwrap_or_else(|| provider.key_env().into());
    let key = std::env::var(&env).unwrap_or_default();
    let mut cfg = provider.configure(&opt("--model").unwrap_or_default(), &endpoint, &key, opt("--effort").as_deref());
    if let Some(n) = opt("--turns").and_then(|n| n.parse().ok()) {
        cfg.max_turns = n;
    }
    if let Err(error) = cfg.validate() {
        eprintln!("{error} 密钥环境变量：{env}");
        std::process::exit(2);
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
    let failed = result.is_err();
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
    if failed { std::process::exit(1); }
}
