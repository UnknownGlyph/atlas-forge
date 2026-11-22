//! Puts the program's icon into atlas.exe on Windows (Explorer, the taskbar,
//! shortcuts). The window's own icon is set at start-up from the same art
//! (`main.rs`), so a machine without the resource compiler still builds and
//! runs; it only shows the default file icon in Explorer.
use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    let icon = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("../../assets/icon/atlas-forge.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", icon.display());
    let windows = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !windows || !msvc {
        return;
    }
    let Some(compiler) = resource_compiler() else {
        println!("cargo:warning=rc.exe not found: atlas.exe keeps the default file icon");
        return;
    };
    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let script = out.join("atlas.rc");
    let res = out.join("atlas.res");
    let icon = fs::canonicalize(&icon).unwrap_or(icon);
    let path = icon.display().to_string();
    let path = path.trim_start_matches(r"\\?\").replace('\\', "\\\\");
    if fs::write(&script, format!("1 ICON \"{path}\"\n")).is_err() {
        return;
    }
    let made = Command::new(&compiler)
        .arg("/nologo")
        .arg("/fo")
        .arg(&res)
        .arg(&script)
        .status()
        .is_ok_and(|status| status.success());
    if made {
        println!("cargo:rustc-link-arg-bin=atlas={}", res.display());
    } else {
        println!("cargo:warning=rc.exe failed: atlas.exe keeps the default file icon");
    }
}

/// `rc.exe` on the path (a Visual Studio prompt), else the newest one in
/// the Windows SDK.
fn resource_compiler() -> Option<PathBuf> {
    let on_path = Command::new("rc.exe")
        .arg("/?")
        .output()
        .is_ok_and(|output| output.status.success() || !output.stdout.is_empty());
    if on_path {
        return Some(PathBuf::from("rc.exe"));
    }
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86") => "x86",
        _ => "x64",
    };
    let mut found: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(env::var_os)
        .map(|base| PathBuf::from(base).join("Windows Kits").join("10").join("bin"))
        .filter_map(|bin| fs::read_dir(bin).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|entry| entry.path().join(arch).join("rc.exe"))
        .filter(|candidate| candidate.is_file())
        .collect();
    // versions are folder names such as 10.0.22621.0
    found.sort_by_key(|candidate| {
        candidate
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.file_name())
            .map(|name| {
                name.to_string_lossy()
                    .split('.')
                    .map(|part| part.parse::<u32>().unwrap_or(0))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    });
    found.pop()
}
