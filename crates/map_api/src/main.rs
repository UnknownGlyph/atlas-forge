//! atlas-query: read-only queries of an Atlas Forge world over JSON lines.
//!
//!   atlas-query <world.atlas>          one request per stdin line, one answer per stdout line
//!   atlas-query --demo | --test-world  the sample or the test world instead of a file
//!   atlas-query <world> --request '{"op": "world.summary"}'   a single request
//!
//! See `{"op": "capabilities"}` for the operations. Nothing here changes
//! the world or reaches the network.

use std::io::{BufRead, Write};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(src) = args.first() else {
        eprintln!("用法：atlas-query <工程.atlas | --demo | --test-world> [--request '<JSON>']");
        std::process::exit(2);
    };
    let session = match map_api::Session::from_arg(src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if let Some(i) = args.iter().position(|a| a == "--request") {
        let Some(req) = args.get(i + 1) else {
            eprintln!("--request 后需要一个 JSON 请求");
            std::process::exit(2);
        };
        println!("{}", session.handle_text(req));
        return;
    }
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let answer = session.handle_text(&line);
        if writeln!(out, "{answer}").and_then(|_| out.flush()).is_err() {
            break;
        }
    }
}
