# Linux desktop app

> **First release:**
> - **In:**
>   - the window with the **Inbox**, **Agenda** and **Board** views. The Board hides empty columns.
>   - the tray, with its badge and menu
>   - the reminder editor's basics, plus the Overdue, Expiry, Note and Pause sections, with time and calendar pickers and the summary sentence
>   - a snooze menu with time-based choices, and the snooze-all chip
>   - the Linux alarm window
>   - first start (this device only, joining with the setup code, or signing in)
>   - settings: You (quiet hours), Priorities (read-only), Sources, This device, Account (with "What your server can see") and About
>   - the notices
> - **Later:** the Calendar and History views, the Turns section, the priority editor, the place, connection and webhook editors, the Groups, Server and Calendars settings sections, the sharing dialog and invitations.

The desktop app starts in the tray at login. Clicking the tray icon opens the full window.

## The window

- **Views:** **Inbox**, **Calendar**, **Agenda**, **Board** and **History**, switched from buttons at the left of the toolbar.
- **Which view opens:** the last one used. On the very first start it's Inbox.
- **First-run tip:** shown once, pointing at the switcher. It explains the views and that the last one is remembered. "Got it" dismisses it, and Help shows it again.
- **Toolbar:** the view switcher, a title, **Snooze all…**, and **New reminder**. Below it sits the "sources need attention" banner when there's something to fix.
- **Left sidebar:**
  - **Filters:** list and priority checkboxes that filter every view, remembered with the last view. Hiding a list only hides it from the window. Its alerts still come, unless you pause it for yourself.
  - **Mini month:** shown in the Calendar view.
  - **Footer:** quiet hours and the server connection.
  - **Sharing:** each list has a ⋯ button that opens sharing.
- **Details** open in a side panel on the right.

### Inbox

- **A strip across the top** covers yesterday, today and tomorrow, with occurrences as marks and busy calendar events as grey bars.
- **Sections:** Overdue, Due, Waiting, Later today, Earlier today. Each row has its buttons.
- **The Overdue section** sorts by highest priority first, then by longest overdue. Minimum and Low occurrences overdue for more than 7 days fold into one row at the end ("4 older quiet reminders ▸"), which offers Skip all….

### Calendar

- **Layout:** Day, 3 days or Week. Calendar events are grey blocks behind the occurrences, and a line marks now.
- **How each state looks:**

  | State | Look |
  |---|---|
  | Due | Filled in the list's colour |
  | Overdue | Red edge |
  | Waiting | Dashed amber |
  | Expected | Outlined |
  | Expected, depends on a condition | Dashed, marked "if someone is home" |
  | Completed | Muted |
  | Skipped | Struck through |
  | Missed | Red outline |
  | Didn't fire | Dotted and faint |

- **A right rail** lists Overdue, Due and Waiting, or shows the occurrence you clicked.

### Agenda

- **A "Now" band** at the top holds open occurrences and waiting reminders.
- **Below it** is one list in time order, from yesterday to tomorrow. Calendar events sit in the list, a rule marks now, and rows expand in place to show details.

### Board

- **Every reminder is a card,** in the columns **Overdue · Due · Waiting · Expected · Watching · Paused · Finished**.
  - **Expected:** reminders with a next expected occurrence.
  - **Watching:** reminders that fire only on an event trigger and have nothing open.
  - **Finished:** one-off reminders whose occurrence is closed. It shows the last 7 days, with **Show older** at the bottom.
  - A reminder with an open occurrence stays in Overdue, Due or Waiting until it closes.
- **A card shows:**
  - the title
  - a status line ("Next: today 19:00", "Waiting for: at High Street, until 20:00")
  - the list, the priority and the faking level
  - a warning when no device holding the list can sense one of its triggers
- **Opening a card** shows **Edit reminder…**, Pause or Resume, and Duplicate, plus the buttons of its open occurrence or wait.
- **No dragging** for now. Dragging into and out of Paused may come later.
- **The Board replaces** a separate list of reminders that aren't time-based.

### History view

- **Two tabs,** **Reliability** and **Group**, with a period picker (7 days to "All kept", and "Since…") and **Export…**.
- **Reliability tab:**
  - **At the top:** a total per list.
  - **One row per reminder:** a strip of its occurrences (done on time, done late, skipped, missed, didn't fire), reliability and on-time rates, skips, didn't fire, usual snoozes, and usual time to do it, with the "often snoozed" and "everyone declined" hints.
  - **The strip** covers the chosen period, up to the 60 most recent occurrences. The numbers always cover the whole period, and hovering over a cell shows its outcome and date.
  - **Row order:** lowest reliability first. Reminders with no occurrences drop to a closing line ("3 reminders had no occurrences"). Clicking a row opens the reminder panel.
- **Group tab:** per shared list, each person's completions as a share bar, work on someone else's turn, declines, and misses on their own turn, with a note for anyone who joined partway.

### Occurrence details and buttons

- **Details:** list, priority, state, trigger, expiry, rotation, sharing, faking level, note, recent history, and the reliability line with a link to History.
- **Buttons on an open occurrence:** **Done**, then **Snooze ▾**, then the third button, then **More ▾**.
  - **Snooze ▾** offers:
    - the priority's interval, 1 hour, until a time, until I get home, tomorrow morning, or pick a time
    - a note of any expiry and its last-chance alert
    - a warning when someone else has claimed a shared occurrence
  - **The third button:**
    - on a personal occurrence: Skip
    - on a shared one: Claim, then Can't do it once someone else has claimed it, or Release on your own claim
  - **More ▾** holds Skip for everyone, I can't do it this time, Done at a different time, Pause, and Edit reminder.
- **Expected occurrences:** Complete early, Skip ahead, Snooze ahead, and on shared ones "I can't".
- **Closed occurrences:** Undo, and Correct.
- **Waiting reminders** have no occurrence yet, so their buttons are **Done anyway** and **Stop waiting**.

## The tray

- **The badge** counts due and overdue occurrences of Low priority and above. It's red when something Medium or above is overdue, and blue otherwise. Minimum and waiting reminders aren't counted.
- **Left-clicking** opens the window.
- **The menu:**
  - Open Reminders
  - each open occurrence as a submenu with Done · Snooze · the third button. KDE's tray menus are plain menus, so the buttons can't sit in the row.
  - a Waiting section
  - Snooze all ▸
  - Quiet this device until… ▸
  - Settings
  - Quit

## The reminder editor

- **Opened from:** New reminder, Edit reminder… on a card or reminder panel, and an occurrence's More menu.
- **At the top,** a live, read-only sentence sums up the reminder: "In Household, remind whoever's turn it is to take the bins out every Wednesday at 18:00 if someone is at Home. If not, wait until 21:00." It mentions an overdue time, expiry or wait only when the reminder overrides it.
- **Always visible:**
  - the title
  - reminder list and priority
  - **When** (triggers)
  - **Only if** (conditions)
  - what happens when the conditions aren't met (wait until…, or let it pass)
- **Folded sections,** each showing a one-line summary when closed: **Overdue**, **Expiry**, **Turns**, **Note**, **Pause**.
- **Defaults show in place,** greyed with their source:
  - Overdue: "after 1 h (Medium)"
  - Expiry: "when it fires again" (for event triggers, "a new event once it's overdue")
  - Waiting: "up to 1 h (Medium)"
- **The trigger and condition pickers:**
  - separate lists for triggers and conditions, grouped by source class
  - every row shows how easily it can be faked
  - they offer existing sources ("Arrive at: Home, High Street, or a new place…")
- **Unsensed triggers:** a trigger that no device holding the list can sense stays selectable. Its warning stays on the chip and on the reminder's card.
- **Editor notes:**
  - weather warning triggers: "Warnings can reach you 15 minutes late or more. This isn't a safety alert."
  - places: arrivals may be late on phones without Google Play Services
  - Wi-Fi: leaving is noticed late on Android
- **Sharing one reminder** happens from the editor, which moves it into a stream of its own.

## Settings

One dialog, opened from ⚙ and from the tray menu.

| Section | What it holds |
|---|---|
| **You** | Your cap on others' reminders, with a warning below Medium. Quiet hours, with "include maximum". Your home location, in the first release (see [Sun events](sources.md#sun-events)). Your retention limit, with its warning and the 7-day notice. |
| **Priorities** | The built-ins read-only, with Copy. The custom-priority editor: style and interval while due, escalation steps once overdue, overdue interval, ring duration, server wait, swipe, and breaking Do Not Disturb. |
| **Sources** | Every source you can see, with its owner, health and faking level, and an editor for each (below). The "sources need attention" banner links here. |
| **This device** | Name, portable or stationary, loudest alert, "Quiet this device until…", which lists it holds, and start at login. |
| **Account** | Profile, password, your devices with Remove, a sign-in code (a QR code and link carrying the server's name and certificate fingerprint), escrow, "What your server can see", leaving the server, deleting the account. |
| **Groups** | Invitations. Each group's members and roles, pending invites, retention limit, and leaving. Creating a group. |
| **Server** (server admins only) | Invites (QR code, single use, expiry, groups to add), accounts with reset and remove, escrow requests to confirm in person, and the server's retention limit. |
| **Calendars** | Signing in to Google through the browser, which calendars to lay over the timeline, and your own OAuth client ID. |
| **About** | The server and its certificate, the app's version, and NWS and Open-Meteo attribution. |

## Source editors

- **Place:**
  - a map with the circle, a radius slider (150 m by default), which list it belongs to, and the Wi-Fi networks that count as arriving
  - the note about late arrivals on phones without Google Play
  - **the map:** OpenStreetMap's map images, with Nominatim address search, used only while the editor is open. Requests identify the app, and the editor shows "© OpenStreetMap contributors". Their usage policies must be checked before building it.
- **Connection:**
  - **Bluetooth:** a paired device.
  - **Wi-Fi:** the name, security (open, password or certificate, each with its faking level) and optional pinned access points.
  - **USB:** a device, with a warning when it has no serial number.
  - which devices count, portable only by default
- **Webhook:**
  - the address, with Copy and "Change address…", and a note on where it's reachable from
  - the signature secret
  - payload filters
  - the listener on this computer (localhost only)
  - which list it belongs to

## Sharing

- **Where:** a list is shared from the ⋯ beside it in the sidebar. A single reminder is shared from the editor. The personal list can't be shared, and says how to share a reminder from it instead.
- **The sharing dialog has:**
  - a row for a username or group, a level, **Share** and **Scan code**
  - an explanation of the four levels
  - everyone with access, and their level
  - pending invitations, with Cancel
  - any unverified user, with **Verify…** and a note that their access is on hold
  - how long history is kept, and where that limit comes from
- **Verifying someone:** both screens show the same five everyday words, such as "maple · orbit · candle · river · flint", to compare in person. If they match, the list key is sealed to that person's new devices. If they don't, nothing is shared, and both people are warned.
- **Invitations:** one gentle notification on arrival, never ringing or repeating. Until answered, a banner in the window shows **Accept**, **Decline** and Details, with a badge on Settings → Groups.

## First start

It starts with a choice:
- **This device only:** everything works standalone.
- **Join with an invite** (or the setup code, for the first account):
  1. Confirm the server, its fingerprint, how long it keeps history, and the groups the invite adds you to, along with "What your server can see".
  2. Pick a username and display name.
  3. Set a password, with a strength meter and a passphrase suggestion. Continue stays disabled until it's good enough.
  4. Escrow, with its warning.
  5. The cap on others' reminders, with Maximum preselected.
  6. Portable or stationary.
  7. Done, followed by the first-run tip.
- **Sign in:** go straight to the password.

## Snooze all

- **The dialog:** everything or one list, until when (durations, a time, or "I get home"), and "include maximum", with a note on what snoozing all does.
- **While active:** a toolbar chip, "All snoozed until 19:00 · End now". The tray menu offers the same choice.

## The alarm window and alerts

- **The alarm window:**
  - "Ringing · list · priority", the title, when it was due and whose turn it is
  - a large **Done**
  - **Snooze ▾** (5, 10 or 30 minutes, or until…), the third button, Acknowledge and Skip…
- **The critical notification** appears beside it with Done · Snooze · the third button. Closing either silences both.
- **The last-chance alert** is a gentle notification: "Last chance: Call the plumber · Expires at 23:59".

## Notices

- **Notes on an occurrence** (an amber strip):
  - a condition that couldn't be checked and counted as met
  - a forecast from the fallback provider
  - a cancelled calendar event
  - a location that looked faked
- **"Not sent yet"** tag on actions the server hasn't received.
- **Banners in the window:**
  - your devices' actions were reconciled ("Your phone skipped… It counts as completed.")
  - "Update the app to see recent changes to Household"
  - the 7-day retention notice
  - "5 failed sign-ins to your account"
  - an escrow release or admin reset in its 1-hour wait, with Cancel
  - "New device signed in"
- **Delete confirmation:** keep the history, marked deleted, or delete it too for everyone, noting that copies already on removed members' devices can't be recalled.

## Linux integration

From the [Linux desktop integration research](../research/linux-desktop-integration.md):

- **Autostart:** an XDG autostart entry (Tauri's autostart plugin), or the Background portal for a sandboxed build. The app starts in the tray with no window, and keeps running when the window closes.
- **Tray:** Tauri's tray speaks KDE's StatusNotifierItem protocol, so Plasma shows it natively. GNOME needs the AppIndicator extension. **Tauri reports no tray clicks on Linux,** so left-click to open the window needs the `ksni` backend or our own StatusNotifierItem code.
- **Notifications with buttons:** Tauri's desktop notifications have no action buttons. Done · Snooze · the third button need our own D-Bus code (`org.freedesktop.Notifications` through `notify-rust` or `zbus`) or the Notification portal (`ashpd`).
- **Sounds:** use the sound theme through the notification's `sound-name` hint. The alarm's looping sound is played by the app.
- **Single instance:** Tauri's single-instance plugin forwards a second launch to the running app.
- **Raising the window on Wayland** needs an xdg-activation token. Plasma provides one to tray items and notification clients, but none of the libraries above pass it on, so that needs our own code.
- **Connections:**
  - Wi-Fi from NetworkManager over D-Bus
  - Bluetooth from BlueZ (`org.bluez.Device1.Connected`)
  - USB from udev events (the `nusb` crate)
- **Location:** none. Desktops don't sense places.
- **Do Not Disturb:** the notification server's "inhibited" state.
- **Keys:** the Secret Service (KWallet on Plasma) through `oo7`.
- **Packaging:** a `.deb` first. A Flatpak would later need permissions for the tray, NetworkManager, BlueZ and USB.

## Decided in

- [How a Tauri 2 app fits into Linux desktops through freedesktop.org conventions][3]
- [Desktop window][26]
- [History records and the two views][27]
- [Desktop dialogs and settings][29]
- [Default expiries and keeping the lists tidy][32]
- [Security hardening before the server goes on the internet][33]
- **The throwaway prototype:** [`prototypes/desktop-window`](https://github.com/csnook/hab-bot/tree/prototype/desktop-window/prototypes/desktop-window). It still says "This computer", which is now "This device".

[3]: https://github.com/csnook/hab-bot/issues/3
[26]: https://github.com/csnook/hab-bot/issues/26
[27]: https://github.com/csnook/hab-bot/issues/27
[29]: https://github.com/csnook/hab-bot/issues/29
[32]: https://github.com/csnook/hab-bot/issues/32
[33]: https://github.com/csnook/hab-bot/issues/33
