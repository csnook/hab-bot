# hab-bot

A reminder app that prompts people to do things at the right moment and records who did them. The design is in [`docs/spec`](docs/spec/README.md), the vocabulary in [`CONTEXT.md`](CONTEXT.md) and the decisions in [`docs/adr`](docs/adr).

This is the walking skeleton: on the Linux desktop, with no server, you can create a one-off reminder, it fires at its time with a notification, and you complete it from the Inbox.

## Layout

| Path | What |
|---|---|
| `crates/core` | `hab-core`: events, state, firing and SQLite storage |
| `app/src-tauri` | `hab-app`: the Tauri 2 desktop app (tray, scheduler, notifications) |
| `ui` | the Preact and TypeScript UI |
| `xtask` | `cargo xtask`, the single entry point below |

## Prerequisites

- Rust (the stable toolchain, with rustfmt and clippy; `rust-toolchain.toml` asks for them) and Node.js 22 with npm.
- Tauri's Linux libraries. On Debian or Ubuntu:

  ```sh
  sudo apt-get install build-essential pkg-config libwebkit2gtk-4.1-dev libgtk-3-dev \
    libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev libdbus-1-dev
  ```

- For the tray on GNOME, the AppIndicator extension. KDE Plasma shows it natively.

## Commands

Everything goes through `cargo xtask`. It installs the UI's packages when they're missing and builds the UI before any step that compiles the app, which embeds it.

| Command | Does |
|---|---|
| `cargo xtask run` | build the UI, then run the desktop app |
| `cargo xtask build` | build the UI and the whole workspace (add `--release` for release) |
| `cargo xtask test` | Rust tests (`cargo test --workspace`) and UI tests (vitest) |
| `cargo xtask lint` | `cargo fmt --check`, `clippy -D warnings`, and the UI's type-check |
| `cargo xtask fmt` | format the Rust code |
| `cargo xtask check` | lint, then test |

## Using it

Fill in the form (title, date, time) and press Create. At that time an occurrence opens, a notification shows the title, and the reminder appears under **Due** in the Inbox. **Done** completes it.

Closing the window hides it and leaves the app running in the tray, where reminders still fire. Use **Quit** in the tray menu to exit. On Linux Tauri doesn't report tray clicks, so use **Open Reminders** in the tray menu to bring the window back. Starting the app a second time raises the running one.

Every change is stored as an event in the personal list's stream in `~/.local/share/io.github.csnook.hab-bot/hab-bot.db` (set `HAB_BOT_DB` to use another file). On start the state is rebuilt from the stream, and a reminder whose time passed while the app was closed fires then.
