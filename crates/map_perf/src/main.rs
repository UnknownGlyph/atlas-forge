use std::{
    env, io,
    path::PathBuf,
    process::{Command, ExitCode},
};

fn run() -> io::Result<()> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    match args.as_slice() {
        [mode, log] | [mode, log, _] if mode == "report" => {
            let trace = PathBuf::from(log);
            let report = args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| map_perf::report::default_path(&trace));
            map_perf::report::write(&trace, &report)?;
            println!("{}", report.display());
        }
        [mode, log] | [mode, log, _] if mode == "record" => {
            let trace = env::current_dir()?.join(log);
            let report = map_perf::report::default_path(&trace);
            if trace == report || trace.exists() || report.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "choose a new .jsonl path with no matching .html report",
                ));
            }
            if !trace.parent().is_some_and(|dir| dir.is_dir()) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    "the recording directory does not exist",
                ));
            }
            let suffix = env::consts::EXE_SUFFIX;
            let app = env::current_exe()?.with_file_name(format!("atlas{suffix}"));
            let mut launch = Command::new(app);
            launch.env("ATLAS_PERF_LOG", &trace);
            if let Some(map) = args.get(2) {
                launch.arg(map);
            }
            let status = launch.status()?;
            if !status.success() {
                return Err(io::Error::other(format!(
                    "Atlas Forge exited with {status}. Run report to recover available samples."
                )));
            }
            if !trace.is_file() {
                return Err(io::Error::other(
                    "Atlas Forge did not write a recording to the requested path",
                ));
            }
            println!("{}", trace.display());
            if !report.is_file() {
                map_perf::report::write(&trace, &report)?;
            }
            println!("{}", report.display());
        }
        [] | [_] => {
            println!("atlas-perf record <trace.jsonl> [world.atlas]\natlas-perf report <trace.jsonl> [report.html]");
            if !args.is_empty() && args[0] != "--help" && args[0] != "-h" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "expected record or report and a trace path",
                ));
            }
        }
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid arguments (see --help)",
            ))
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
