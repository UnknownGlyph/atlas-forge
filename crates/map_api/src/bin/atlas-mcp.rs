//! atlas-mcp: an MCP server (stdio) offering queries of one Atlas Forge
//! world as tools, and edits that change a draft only (never the file
//! opened); `draft_save` writes the draft to another file, to be looked at
//! and taken in the application.
//!
//!   atlas-mcp <world.atlas | --demo | --test-world>
//!
//! JSON-RPC messages, one per line, on stdin and stdout; diagnostics go to
//! stderr. Nothing here changes the file opened or reaches the network.

use std::io::{BufRead, Write};

fn main() {
    let Some(src) = std::env::args().nth(1) else {
        eprintln!("用法：atlas-mcp <工程.atlas | --demo | --test-world>");
        std::process::exit(2);
    };
    let mut session = match map_api::Session::from_arg(&src) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    eprintln!("atlas-mcp：{}（版本 {}）", session.doc().meta.name, session.version());
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        if let Some(answer) = map_api::mcp::handle_line(&mut session, &line) {
            if writeln!(out, "{answer}").and_then(|_| out.flush()).is_err() {
                break;
            }
        }
    }
}
