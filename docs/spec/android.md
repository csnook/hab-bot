# Android app

> **First release:**
> - **In:**
>   - bottom navigation with three views: Inbox, Agenda and Board
>   - swipes with 5 seconds of Undo
>   - the details sheet and the full-screen editor
>   - notifications, with the quiet-reminders group, and the full-screen alarm
>   - first start, asking for Notifications, Alarms & reminders, Do Not Disturb, and Local network on Android 17
>   - calendar access and the full-screen alarm, asked for when first needed
>   - settings matching the desktop's first release
> - **Later:** the Calendar and History views, push and the ntfy suggestion, precise location, and the place, connection and webhook sources with their permissions.

The Android app is the same Tauri app as the desktop, with the same Rust core and Preact UI, laid out for a phone. Background work is Kotlin we write, calling into Rust.

## Navigation

- **The bottom bar** holds the same five views as the desktop: Inbox, Calendar, Agenda, Board and History. The app opens on the last one used.
- **The top bar** has:
  - the title
  - a **filter** button, which opens list and priority filters in a bottom sheet. They apply to every view and are remembered.
  - **Settings**
- **The + button** creates a new reminder.

## The views

- **Inbox:** the desktop's sections (Overdue, Due, Waiting, Later today, Earlier today), with a Done button on each open occurrence.
  - **Swipe right for Done, and left to snooze** for the priority's interval, then **5 seconds of Undo**. Rows that aren't open don't swipe.
  - **The Overdue section** sorts and folds as on desktop. Older quiet reminders go into one row with Skip all….
- **Calendar:** a one-day timeline, with calendar events behind the occurrences.
- **Agenda:** open occurrences in a scrolling row of chips, above the agenda.
- **Board:** one column per screen width, with column chips to jump between them.
- **History:** a card per reminder (the strip, reliability, on time, skipped, "often snoozed"), and a Group tab with share bars.
- **Details:** tapping a row or card opens a **bottom sheet** with the details, the full set of buttons and the reliability line.
- **The editor** is full-screen: the one-sentence summary on top, then the form, then the folded sections.

## Notifications and the alarm

- **Notifications** show Done · Snooze · the third button (see [Alerts](alerts.md#notification-buttons)).
  - **Swiping one away** snoozes it for the priority's interval. High and Maximum can't be swiped away.
  - **Minimum and Low** go into one collapsed "quiet reminders" group with a summary line, and each keeps its buttons when expanded. Medium and above stay separate.
- **The full-screen alarm:**
  - "Ringing · list · priority", the title, when it was due and whose turn it is
  - a large **Done**
  - **Snooze ▾** (5, 10 or 30 minutes, or until…), Claim, Acknowledge and Skip…

  The volume rises over 30 seconds. It runs as a foreground service with the screen on, keeping a live connection to the server while it rings.
- **Without the full-screen permission,** the alarm is a large pop-up with the same buttons.

## First start and permissions

- **The same three choices as desktop:** this device only, join with an invite, or sign in. Joining scans the invite code with the camera or takes a pasted link.
- **No portable-or-stationary question,** since phones and tablets are portable by default.
- **Asked at first start,** each explained before Android asks:
  - **Notifications** (Android 13 and later)
  - **Alarms & reminders**, for exact alarms
  - **Do Not Disturb access**, so that Maximum gets through
  - **Local network**, on Android 17, when joining a server on the home network
- **Asked the first time they're needed:**
  - full-screen alarms, for the first High or Maximum reminder
  - calendar
  - location all the time, for the first place trigger
  - Nearby devices, for the first Bluetooth source
  - location for Wi-Fi names, for the first Wi-Fi source
- **If you say no,** you can still continue. A banner stays in the app until it's fixed, such as "Reminders can't alert you: allow notifications · Fix". Every permission is also listed with its status in Settings → This device.

## Settings

- **The same sections as desktop,** shown as a list that opens one page at a time.
- **This device** covers phones and tablets, and holds:
  - the name, portable or stationary, loudest alert, and "Quiet this device until…"
  - **Updates from your other devices:** whether push works, through Google Play Services or a UnifiedPush app, or whether updates arrive only every 15 minutes or so
  - **Precise location**
  - every permission, with its status
- **The place editor** starts with **Use where I am now**, with the map to adjust it, and credits OpenStreetMap.

## Phones without Google

- **Push:** a UnifiedPush app such as ntfy is suggested **once, at first start**. After that the suggestion lives only in Settings → This device, with "Get ntfy…" and "Not now".
- **Precise location** is an opt-in setting there. It's offered with an explanation on phones without Google, where arrivals can be about 30 minutes late. While it's on, an ongoing notification says "Watching for arrivals precisely · Turn off".

## Background work

From the [Android background research](../research/android-background-tauri.md):

- **Tauri's Rust starts only when an Activity starts.** Every background entry point therefore needs Kotlin we write, which starts the Rust core without opening the app and calls into it. That covers alarms, boot, geofences, connection broadcasts, periodic work and push. Existing plugins cover little of this.
- **Exact alarms:**
  - `AlarmManager.setAlarmClock()` fires on time through Doze, and `setExactAndAllowWhileIdle()` for quieter ones. Both need the "Alarms & reminders" permission (`SCHEDULE_EXACT_ALARM`).
  - Alarms are cleared by a reboot (and by a force-stop on Android 15), so they're registered again at boot.
- **The full-screen alarm** needs `USE_FULL_SCREEN_INTENT` and the alarm category. From Android 17, sound played in the background needs a foreground service.
- **Places:** `GeofencingClient` with Google Play Services, or the framework's `addProximityAlert` without them. The precise mode samples location in a foreground service.
- **Bluetooth:** connection broadcasts reach a manifest receiver while the app is closed (`BLUETOOTH_CONNECT` permission).
- **Wi-Fi:** a network callback wakes the app when a matching network becomes available, but not when it's lost. Reading the network name needs location.
- **Periodic work:** WorkManager, at most every 15 minutes, which Doze can defer. It's used for weather polling and, without push, for syncing.
- **Push:** our own Kotlin push service for UnifiedPush (see [Push](sync-and-server.md#push)).
- **Do Not Disturb:** the app reads Android's interruption filter.
- **Keys:** the Android Keystore.

## Decided in

- [How Tauri 2 apps can run in the background on Android][2]
- [Alert styles and escalation][22]
- [Sync design][25]
- [Android screens][30]
- [Default expiries and keeping the lists tidy][32]
- **The throwaway prototype:** [`prototypes/android-screens`](https://github.com/csnook/hab-bot/tree/prototype/desktop-window/prototypes/android-screens). Its README lists the later changes it doesn't show.

[2]: https://github.com/csnook/hab-bot/issues/2
[22]: https://github.com/csnook/hab-bot/issues/22
[25]: https://github.com/csnook/hab-bot/issues/25
[30]: https://github.com/csnook/hab-bot/issues/30
[32]: https://github.com/csnook/hab-bot/issues/32
