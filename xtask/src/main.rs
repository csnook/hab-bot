//! `cargo xtask`: the single entry point for building, testing, linting and running.

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const HELP: &str = "\
cargo xtask <command>

  test      run the core's unit tests and the UI's type check
  lint      cargo fmt --check, clippy with warnings denied, and the UI's type check
  build     build the UI, then the desktop app
  run       build the UI, then run the desktop app (dev mode)
  android   build a debug APK and install it on the connected device
";

fn main() {
    let result = match std::env::args().nth(1).as_deref() {
        Some("test") => test(),
        Some("lint") => lint(),
        Some("build") => build_ui().and_then(|_| tauri(&["build"])),
        Some("run") => build_ui().and_then(|_| tauri(&["dev"])),
        Some("android") => build_ui().and_then(|_| android()),
        _ => {
            print!("{HELP}");
            exit(2)
        }
    };
    if let Err(message) = result {
        eprintln!("xtask: {message}");
        exit(1);
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn run(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    println!("$ {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .map_err(|e| format!("could not start {program}: {e}"))?;
    status
        .success()
        .then_some(())
        .ok_or_else(|| format!("{program} {} failed", args.join(" ")))
}

fn ui_dir() -> PathBuf {
    root().join("app")
}

fn npm_install() -> Result<(), String> {
    let ui = ui_dir();
    if ui.join("node_modules").exists() {
        return Ok(());
    }
    run(&ui, "npm", &["install"])
}

fn build_ui() -> Result<(), String> {
    npm_install()?;
    run(&ui_dir(), "npm", &["run", "build"])
}

fn test() -> Result<(), String> {
    run(&root(), "cargo", &["test", "--workspace"])?;
    npm_install()?;
    run(&ui_dir(), "npm", &["run", "typecheck"])
}

fn lint() -> Result<(), String> {
    run(&root(), "cargo", &["fmt", "--all", "--check"])?;
    run(
        &root(),
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    npm_install()?;
    run(&ui_dir(), "npm", &["run", "typecheck"])
}

fn tauri(args: &[&str]) -> Result<(), String> {
    let mut full = vec!["tauri"];
    full.extend_from_slice(args);
    run(&ui_dir(), "npx", &full)
}

fn android() -> Result<(), String> {
    tauri(&["android", "build", "--debug", "--apk"])?;
    let apk = ui_dir().join(
        "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk",
    );
    run(&root(), "adb", &["install", "-r", apk.to_str().unwrap()])
}
