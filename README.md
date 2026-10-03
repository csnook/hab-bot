# hab-bot

A reminder app: Rust core, Tauri 2 shell, Preact + TypeScript UI. See `docs/spec/` for the design.

## Layout

- `crates/core`: the reminder core. Every change is an event in SQLite; state is rebuilt by applying the stream.
- `app/`: the Preact UI; `app/src-tauri/`: the Tauri app (desktop and Android).
- `xtask/`: the single entry point for builds.

## Building

Everything goes through `cargo xtask`:

| Command | What it does |
|---|---|
| `cargo xtask test` | core unit tests, and the UI type check |
| `cargo xtask lint` | `cargo fmt --check`, clippy with warnings denied, UI type check |
| `cargo xtask build` | build the UI and the desktop app |
| `cargo xtask run` | run the desktop app in dev mode |
| `cargo xtask android` | build a debug APK and `adb install` it |

On Debian/Ubuntu the desktop app needs `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev`. Android needs the SDK/NDK, `JAVA_HOME`, `ANDROID_HOME`, `NDK_HOME` and the Rust Android targets.
