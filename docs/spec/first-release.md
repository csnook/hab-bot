# First release and 1.0

## Who it's for

- **The owner alone, on Android and Linux, synced through the home server.**
- **Personal lists only.** You can make more lists, but not share them.
- **Versions are 0.x** until 1.0.
- **Nothing is planned after the first release.** Priorities are set after using it.

## In the first release

| Area | What's in |
|---|---|
| [Reminders](reminders.md) | Reminders and their own lists, schedules, countdowns, one-offs, waiting, expiry, and every action on personal occurrences: complete (including early), skip, pause, acknowledge, snooze, undo and correct. |
| [Sources](sources.md) | **Time:** schedules, countdowns, sun events and time-based conditions. **Calendar on Android only**, through the system calendar provider, which reads Google Calendar with no Google project. Occurrences the phone fires reach the desktop through sync and alert there too. |
| [Alerts](alerts.md) | The five built-in priorities with all four alert styles and escalation, the full-screen alarm on Android, the Linux alarm window, the last-chance alert, snooze all and quiet hours, the Do Not Disturb check, each device's loudest style and "quiet this device", and the tidy-list rules. |
| [Accounts](sharing-and-accounts.md) | The setup code, which creates your account as server admin. A password through OPAQUE and Argon2id. Signing in on the second device with the password, or approving it by QR code. The device list, where Remove rotates the keys. Changing the password. A standalone device joining later. The password backoff and failed sign-in notice. "What your server can see". |
| [Sync and the server](sync-and-server.md) | The encrypted event log and its merge rules, keys and signatures, SQLite and the key stores, one WebSocket per device plus HTTPS for bulk downloads, the pinned certificate plus a trusted one read from files, and format versions. One listener on 443 with a configurable address, the setup code's expiry, and logs without IP addresses. |
| [Desktop](desktop.md) | The Inbox, Agenda and Board views (the Board hides empty columns), and the tray. The editor's basics plus the Overdue, Expiry, Note and Pause sections, with time and calendar pickers and the summary sentence. A snooze menu with time-based choices, and the snooze-all chip. First start. Settings: You (quiet hours), Priorities (read-only), Sources, This device, Account and About. The notices. |
| [Android](android.md) | Bottom navigation with three views, swipes with 5 seconds of Undo, the details sheet and the full-screen editor, notifications with the quiet-reminders group, and the full-screen alarm. First start asks for Notifications, Alarms & reminders, Do Not Disturb, and Local network on Android 17. Calendar access and the full-screen alarm are asked for when first needed. |
| [History](history.md) | Recorded from the first day, since it is the event stream. |

### Known gaps

- **No push.** The phone relies on periodic sync (every 15 minutes at best, later in Doze), the check before every firing, and a live connection while ringing. So:
  - a reminder created on the desktop that comes due before the phone's next sync rings only on the desktop
  - a quiet notification dealt with on the desktop stays on the phone until it next syncs
- **No recovery without a device.** There's no escrow and no admin reset, so losing every device *and* the password loses the data. The app warns about this when the password is set.

## Later, in no set order

- **Sharing:**
  - lists shared with users and groups, groups, invites for other people, invitations, claims, assignments, rotation and verification
  - the editor's Turns section, the sharing dialog, the Groups settings, and the cap on other users' reminders
  - invite rate limits
- **Push:** UnifiedPush, starting with the Doze wake-up prototype, and the ntfy suggestion.
- **Source classes:**
  - calendar on Linux (CalDAV with the Google OAuth client)
  - places, with the place editor and precise location
  - weather, connections and webhooks
  - for webhooks, the separate webhook port and address, and webhook rate limits
- **Views:** Calendar and History, and the export.
- **Priorities:** custom priorities and the priority editor.
- **Accounts:**
  - escrow and admin resets, with the 1-hour wait
  - retention limits
  - leaving the server, deleting the account, and the Server settings section
- **The web client,** with its trusted certificate, GitHub Pages site and WebAssembly benchmark.
- **Hardening:** padding events, and automated VPN and DNS setup (possibly through Tailscale).

## How it ships

- **Android:** a signed APK, built from tagged releases and attached to GitHub Releases. It's installed and updated with **Obtainium**, which watches the repository's releases. Keep the signing key backed up, because Android refuses an update signed with a different key.
- **Linux:** a **`.deb`** for now, from the same releases.
- **Server:** a single Rust binary run as a systemd service, or a container image. It makes a nightly SQLite backup to a folder you choose (see [Running the server](sync-and-server.md#running-the-server)).
- **Builds:**
  - Every build runs through **`cargo xtask`**, a Rust program in the repo, inside a pinned container image. It works with Podman or Docker on any machine or CI.
  - `cargo xtask release` builds the APK, the `.deb`, the server and (later) the web bundle, and writes their hashes.
  - Releases list those hashes.

### Setup work for the first release

- an Android signing key, backed up
- `cargo xtask release` producing the APK, the `.deb` and the server binary from a tag
- the home server's service and its backup folder

The other setup items wait for the features that need them:
- the Google OAuth client, with calendar on Linux
- the Doze push prototype, with push
- the OpenStreetMap and Nominatim usage-policy check, with places
- the domain or Tailscale name, the GitHub Pages site and the WebAssembly benchmark, with the web client

## 1.0

**1.0 is the first release meant for anyone other than you,** whether the household or other people running their own servers.

### Before 1.0

- **Reconsider** the 1-hour wait before releasing escrow or completing an admin reset.
- **Confirm reproducible builds:** two machines building the same tag must get identical web bundle and server hashes. The APK and `.deb` aim to be reproducible but don't have to be.
- **Run releases in GitHub Actions,** calling `cargo xtask release` and adding signed build attestations.

## Decided in

- [Where to draw the first-release line][28]
- [Security hardening before the server goes on the internet][33]

[28]: https://github.com/csnook/hab-bot/issues/28
[33]: https://github.com/csnook/hab-bot/issues/33
