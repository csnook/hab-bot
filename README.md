# hab-bot

A reminder app that prompts people to do things at the right moment and records who did them. The design is in [`docs/spec`](docs/spec/README.md), the vocabulary in [`CONTEXT.md`](CONTEXT.md) and the decisions in [`docs/adr`](docs/adr).

This is the walking skeleton: on the Linux desktop, with no server, you can create a one-off reminder, it fires at its time with a notification, and you complete it from the Inbox.

## Layout

| Path | What |
|---|---|
| `crates/core` | `hab-core`: events, state, firing and SQLite storage |
| `crates/server` | `hab-server`: the sync server (HTTPS, self-signed certificate, setup code, SQLite) |
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
| `cargo xtask server [options]` | run the sync server (see below) |
| `cargo xtask build` | build the UI and the whole workspace (add `--release` for release) |
| `cargo xtask test` | Rust tests (`cargo test --workspace`) and UI tests (vitest) |
| `cargo xtask lint` | `cargo fmt --check`, `clippy -D warnings`, and the UI's type-check |
| `cargo xtask fmt` | format the Rust code |
| `cargo xtask check` | lint, then test |

## Using it

Fill in the form (title, date, time) and press Create. At that time an occurrence opens, a notification with **Done** and **Skip** buttons shows the title, and the reminder appears under **Due** in the Inbox. **Done** completes it.

## Notifications

Alerts are our own `org.freedesktop.Notifications` calls through `zbus` (Tauri's notification plugin has no buttons, and `notify-rust` can only report a button by blocking a thread per notification). The code is in `app/src-tauri/src/notify.rs`; what to alert, and when, is decided by `hab_core::Alerter` (`crates/core/src/alerter.rs`), which Android can share.

- **Styles:** silent is low urgency with `suppress-sound`; gentle is normal urgency with the `message-new-instant` sound from the sound theme and a 10 s timeout; insistent is gentle again every overdue interval until the occurrence closes. Each priority's due style and escalation steps are followed, counted from when the occurrence went overdue.
- **Buttons:** Done and Skip, and a click on the notification (`default`) opens the window at that occurrence. They work with the window closed. Snooze joins with the snoozing ticket.
- **Do Not Disturb:** the server's `Inhibited` property (Plasma). GNOME doesn't expose it, so there the `show-banners` setting is read with `gsettings`. While on, priorities that don't break it alert as silent, then catch up at their current level.
- **History:** the first alert and each change of style are written to the list as `OccurrenceAlerted` events (format 4) with the device that alerted. Repeats aren't.
- **Alarm** (#42) isn't built: where the alerter decides on an alarm it is delivered as an insistent notification, with `Notification::style` still `Alarm` for the alarm code to act on.

Closing the window hides it and leaves the app running in the tray, where reminders still fire. Use **Quit** in the tray menu to exit. On Linux Tauri doesn't report tray clicks, so use **Open Reminders** in the tray menu to bring the window back. Starting the app a second time raises the running one.

Every change is stored as an event in the personal list's stream in `~/.local/share/io.github.csnook.hab-bot/hab-bot.db` (set `HAB_BOT_DB` to use another file). On start the state is rebuilt from the stream, and a reminder whose time passed while the app was closed fires then.

## The sync server

`cargo xtask server -- --help` lists the options; each has an `HAB_SERVER_*` environment variable too. For example, to try it without privileges:

```sh
cargo xtask server --data-dir /tmp/hab-server --listen 127.0.0.1:8443
```

- **State** is one SQLite file, `hab-server.db`, in the data folder (default: the current folder). It holds the certificate and its private key, so the file is made readable by its owner only.
- **One HTTPS listener**, on `0.0.0.0:443` unless `--listen` (or `--address` and `--port`) says otherwise.
- **The certificate** is self-signed, made on first start and kept. Its SHA-256 fingerprint is printed at every start, for invites to carry.
- **The setup code** is printed in the console only. It stops working after 24 hours or once the first account exists, and a restart makes a new one.
- **IP addresses** are held in memory for open connections and appear in the log only with `--debug`.
- **Operations:** a trusted certificate from files (served for its names, beside the self-signed one), a nightly SQLite backup, a systemd unit in `packaging/` and a `Containerfile`. See [Running the server](docs/running-the-server.md).
- `GET /api/v1/info` returns `{"name": ..., "version": ...}`, for an app to confirm before joining.
