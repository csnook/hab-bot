# Reminders: design spec

A reminder app that prompts people to do things at the right moment (at a time, at a place, when the weather turns) and records who did them. This spec covers the whole system:
- the Tauri app on Android and desktop Linux
- the web client
- the sync server
- sharing between users and groups
- the history with its two views

[First release and 1.0](first-release.md) marks what gets built first.

## How to read it

- **Terms** are defined in the glossary, [`CONTEXT.md`](../../CONTEXT.md), and this spec uses them exactly. In particular, an *alert* is how an occurrence gets attention (a notification is one kind), a *trigger* prompts an evaluation, and a *condition* must hold for a reminder to fire.
- **Hard-to-reverse choices** are recorded as ADRs, which give the reasons and the options turned down:

  | ADR | Decision |
  |---|---|
  | [0001](../adr/0001-one-open-occurrence-per-reminder.md) | A reminder has at most one open occurrence |
  | [0002](../adr/0002-per-list-keys-sealed-to-devices.md) | Encrypted reminder lists use a per-list key sealed to each device |
  | [0003](../adr/0003-linux-calendars-via-caldav.md) | On Linux, calendars are read over CalDAV |
  | [0004](../adr/0004-password-unlocked-accounts-and-device-vouching.md) | Accounts are unlocked by a password, and a user's devices vouch for each other |
  | [0005](../adr/0005-encrypted-event-log-numbered-by-server.md) | Sync is an encrypted event log per reminder list, numbered by the server |
  | [0006](../adr/0006-push-through-unifiedpush.md) | Push goes through UnifiedPush, with its embedded FCM distributor on phones with Google |
  | [0007](../adr/0007-web-client-from-a-static-host-with-a-browser-trusted-server-certificate.md) | The web client loads from a static host, and syncs only with a server that has a browser-trusted certificate |

- **Research notes** in [`docs/research/`](../research/) hold the platform facts that decisions rest on, with their sources.
- **Where each decision came from:** every part ends with links to the tickets of the map [Reminder app design](https://github.com/csnook/hab-bot/issues/1). A ticket's resolution comment, and any later "Revised by" or "Refined by" comments on it, hold the reasoning and the alternatives. This spec holds the outcome, with later revisions already applied.
- **Prototypes** are throwaway and live on the [`prototype/desktop-window`](https://github.com/csnook/hab-bot/tree/prototype/desktop-window) branch: the desktop window under `prototypes/desktop-window/` and the Android screens under `prototypes/android-screens/`. Where they differ from this spec, the spec wins.
- **First release:** each part opens with a short note on which of its parts are in the first release.

## Audience and principles

- **Audience:** the owner and their household, on a server they run. A public product (hosted sign-up, Google Play) comes much later. Nothing here plans for it, but nothing should make it hard.
- **Standalone first:** reminders fire with no network, and the app works on a device with no server at all. A source class that can work without a server must. Sharing, several devices and recovery need a server.
- **Encrypted by default:** reminder content, history and source settings are always encrypted so that only the devices of users with access can read them. The server reads only accounts, groups, access and timing.
- **Little dependence on Google:** Google Play Services is used where present, and everything degrades gracefully without it.
- **Linux:** KDE Plasma on Wayland is the primary desktop, and the app follows freedesktop.org conventions so that it works on other desktops too.
- **Languages:** Rust for the app core and the server, with the core also compiled to WebAssembly for the web client. TypeScript with Preact for the UI, shared by the apps and the web client. Minimal Kotlin where Android requires it, and no other languages.
- **Reaching the server is a deployment choice:** the home network, a VPN such as Tailscale, or the internet. The server has to be safe however it's deployed.

## Parts

1. [Reminders and occurrences](reminders.md): reminders, triggers, conditions, waiting, occurrences, expiry, and acting on occurrences.
2. [Alerts, priorities and snoozing](alerts.md): alert styles, priorities, escalation, snoozing, quiet hours, and alerting on several devices.
3. [Sources](sources.md): the source-class contract, then time, calendar, places, weather, connections and webhooks.
4. [People, sharing and accounts](sharing-and-accounts.md): users, accounts, devices, groups, access, claims, rotation, escrow and verification.
5. [Sync, encryption and the server](sync-and-server.md): the event log, keys, cryptography, storage, push, retention, and running the server.
6. [History](history.md): what's recorded, the reliability view, the group view, and the export.
7. [Linux desktop app](desktop.md): the window and its views, the tray, dialogs, settings and Linux integration.
8. [Android app](android.md): screens, notifications, the alarm, permissions and background work.
9. [Web client](web-client.md): where the code comes from, reaching the server, storage, alerts and layout.
10. [First release and 1.0](first-release.md): what ships first, how it ships, and what must happen before 1.0.

## Architecture at a glance

- **The core (Rust)** is shared by Android, Linux and, as WebAssembly, the web client. It holds:
  - the domain model: reminders, occurrences and evaluation
  - the source classes, as Rust modules
  - the event log and its merge rules
  - cryptography
  - storage in SQLite (IndexedDB in the browser)
- **The UI (Preact and TypeScript)** is one codebase. It lays out as the Android screens on small screens and as the desktop window on large ones.
- **Platform glue:**
  - **Android:** Kotlin for exact alarms, the full-screen alarm, geofences, connection broadcasts, periodic work and the push service. It calls into Rust without opening the app.
  - **Linux:** D-Bus and portal code for notifications with buttons, tray clicks, NetworkManager, BlueZ, sound, and raising the window on Wayland.
- **The server (Rust, SQLite)** has these jobs:
  - number each list's events and store them, encrypted
  - hold sealed list keys
  - run OPAQUE password checks
  - receive webhooks
  - send content-free Web Push

  It never evaluates reminders and can't read their content.

## Out of scope

These are ruled out of this effort. Some may come back as separate efforts.
- **A public product:** hosted sign-up and Google Play distribution. Open sign-up would be a server setting then.
- **Sharing between servers (federation):** not planned, but nothing rules it out.
- **More providers:** weather outside the US and calendars other than Google come later. The design only has to make them addable.
- **Home Assistant integration,** and analytics beyond the two history views.
- **Native apps for iOS, Windows and macOS.** The web client reaches them in a browser on a best-effort basis.
- **Calendar sources in the web client.**
- **Conditions about a named user or everyone** ("when Sam is home"), which would need presence sharing with consent.
- **Server-side help for sources:** a server cache for polled data, devices sharing what they observe, and readable lists and sources.
- **More location:** location on desktops, and choosing the forecast provider per place.
- **USB on Android.**
- **A public relay service for webhooks.**
- **Several users on one device,** and a "who did this?" picker on a shared screen.
- **Dragging Board cards between columns.**
- **Effort weighting in the group view.**
