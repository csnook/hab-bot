//! `cargo xtask`: the single entry point for building, testing, linting and
//! running the app.

use std::path::{Path, PathBuf};
use std::process::{exit, Command};

const HELP: &str = "\
cargo xtask <command>

Commands:
  build   Build the UI, then the desktop app (release with --release)
  test    Run the Rust tests and the UI tests
  lint    Check formatting, run clippy (warnings are errors) and type-check the UI
  fmt     Format the Rust code
  check   lint, then test (what CI runs)
  run     Build the UI, then run the desktop app
  server  Run the sync server (arguments after `server` go to it, see --help)
  ui      Install the UI's packages if needed and build it to ui/dist

Extra arguments after `build` or `run` are passed to cargo.
Build and test cover the server too, as part of the workspace.";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask lives in the workspace")
        .to_path_buf()
}

fn step(program: &str, args: &[&str], dir: &Path) {
    eprintln!("==> {program} {}", args.join(" "));
    let status = Command::new(program)
        .args(args)
        .current_dir(dir)
        .status()
        .unwrap_or_else(|e| {
            eprintln!("cannot run {program}: {e}");
            exit(1)
        });
    if !status.success() {
        exit(status.code().unwrap_or(1));
    }
}

fn cargo(args: &[&str]) {
    step("cargo", args, &root());
}

fn npm(args: &[&str]) {
    step("npm", args, &root().join("ui"));
}

fn ui_install() {
    if !root().join("ui/node_modules").exists() {
        npm(&["ci"]);
    }
}

/// The app embeds ui/dist at compile time, so every Rust step that compiles
/// the app needs it built first.
fn ui_build() {
    ui_install();
    npm(&["run", "build"]);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next();
    let rest: Vec<String> = args.collect();
    let rest: Vec<&str> = rest.iter().map(String::as_str).collect();
    match command.as_deref() {
        Some("ui") => ui_build(),
        Some("build") => {
            ui_build();
            let mut a = vec!["build", "--workspace"];
            a.extend(&rest);
            cargo(&a);
        }
        Some("fmt") => cargo(&["fmt", "--all"]),
        Some("lint") => lint(),
        Some("test") => test(),
        Some("check") => {
            lint();
            test();
        }
        Some("run") => {
            ui_build();
            let mut a = vec!["run", "--package", "hab-app"];
            a.extend(&rest);
            cargo(&a);
        }
        Some("server") => {
            let mut a = vec!["run", "--package", "hab-server", "--"];
            a.extend(&rest);
            cargo(&a);
        }
        Some("help" | "--help" | "-h") | None => println!("{HELP}"),
        Some(other) => {
            eprintln!("unknown command `{other}`\n\n{HELP}");
            exit(2);
        }
    }
}

fn lint() {
    ui_build();
    cargo(&["fmt", "--all", "--check"]);
    cargo(&[
        "clippy",
        "--workspace",
        "--all-targets",
        "--",
        "-D",
        "warnings",
    ]);
    npm(&["run", "lint"]);
}

fn test() {
    ui_build();
    cargo(&["test", "--workspace"]);
    npm(&["test"]);
}
