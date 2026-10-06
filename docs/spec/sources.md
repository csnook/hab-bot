# Sources

> **First release:**
> - **In:** time (schedules, countdowns, sun events and time-based conditions), and calendar on Android only, through the system calendar provider. Occurrences the phone fires from calendars reach the desktop through sync.
> - **Later, in no set order:** calendar on Linux (CalDAV with the Google OAuth client), places, weather, connections and webhooks.

A **source** is something configured that the app watches, such as a specific calendar, the car's Bluetooth or the weather at Home. It supplies triggers, conditions or both. A **source class** is a kind of source as the app implements it (calendar, Bluetooth, weather). Only developers deal in source classes, and new ones arrive with app updates.

## The source-class contract

- **Built in:** each source class is a Rust module implementing a common trait, plus its platform glue: Kotlin on Android, D-Bus or portals on Linux. No third-party code is loaded at runtime.
- **Standalone first:** a source class that can work without a server must. Webhooks are the only class that needs a server by nature, and a standalone desktop can still listen for them on localhost.
- **What a source class declares:**

  | Part | What it provides |
  |---|---|
  | **Settings** | What configuring a source asks for, described as data so the editor can draw a standard form. An optional custom editor, such as a map picker. |
  | **Trigger kinds** | Parameters, and whether each is a **scheduled instance** or an **event**. Whether it can **predict** upcoming instances, for expected occurrences and last-chance alerts. The **capabilities** a device needs to sense it, and how easily it can be **faked**. |
  | **Condition kinds** | Parameters. Whether it can be **predicted** for a future time, and whether it can report **becoming true** (for waiting). Whether it's about the device's own user. Each class supplies its own opposite ("not at Home"). |
  | **Runtime, on each device** | Start watching, and emit trigger events and condition changes. Answer "is this true now?", and for predictable kinds "what are the instances between t1 and t2?" |
  | **Health** | Working, needs permission, can't reach the network, or not supported on this device. |

  If the contract doesn't hold up once the first classes are written, a throwaway prototype of the trait comes next.
- **Capabilities:**
  - Each trigger kind lists what it needs: location, Bluetooth, calendar access, network, always on, or portable.
  - Each device advertises what it has.
  - The editor's warning about triggers no device can sense comes from matching the two.
- **Faking levels:** each trigger and condition kind declares one of **none, difficult, moderate** or **easy**, with a one-line guard, such as "network names are easy to fake: combine with a place". It shows when you pick the kind in the editor, and as a small badge on the reminder's card.
- **When a source can't work:**
  - A banner in the app says "2 sources need attention".
  - **One** notification comes when a source that an active reminder depends on stops working. It isn't repeated.
  - A condition that can't be checked counts as met, with a note.
  - A trigger that can't be sensed simply doesn't occur.
- **Ownership:**
  - A source belongs either to a **reminder list**, where that list's reminders can use it and its users see it, or to a **user personally**.
  - A source can refer to another: "weather at Home" links to the Home place.
- **Ending snoozes:** any source's triggers can end a snooze or a wait, or be part of an expiry.

## Time

### Schedules

- **A schedule** fires by the calendar and clock, such as every weekday at 7:00 or the first Monday of the month.
- **They follow iCalendar's recurrence rules** (RRULE, the format Google Calendar uses). The UI offers the common patterns, and anything else can still be expressed.
- **Several times a day** means several schedule triggers on one reminder. Since a reminder has only one open occurrence, an unfinished 8:00 occurrence is missed when the 20:00 one fires.

### Countdowns

- **A countdown** fires a set time after the reminder's last occurrence closed, such as 3 days after the plants were last watered.
- **Units:** a countdown of hours or less counts elapsed time ("8 hours after the last dose"). One of days or longer can instead fire at a time of day ("3 days later, at 9:00"). This is chosen per reminder.
- **First occurrence:** creating the reminder asks "when was this last done?", with "now" as the default. "Never" makes it fire at once.
- **Restarting:**
  - A completion restarts the countdown from its **recorded** time. If you record 9:40 but tap at 11:00, it counts from 9:40.
  - Completing before it fires restarts it and cancels the pending firing.
  - A skip or a miss restarts it from when the occurrence closed.

### Sun events

- **Sunrise, sunset, civil dawn and civil dusk,** each with an offset ("30 minutes before sunset"), calculated on the device.
- **Where:** they use a configured place (Home by default) or the device's current location, chosen per reminder.
- **In the first release,** before places exist, sun events and the daylight and darkness conditions use the user's **home location**. The first time a reminder needs it, the app asks for it: "Use where I am now" on phones, or a latitude and longitude on any device. It's stored as the user's Home place, so places later build on it, and it can be changed in Settings → You. Since it's configured rather than sensed, desktops evaluate these reminders too. Decided while slicing the [First release](https://github.com/csnook/hab-bot/issues/35).
- **No firing** on a day when the event doesn't happen, as in polar regions.

### Time zones and daylight saving

- **Each reminder chooses.** It's **floating** by default (7:00 wherever the device is), or pinned to a named time zone, for example medicine kept on a fixed interval while travelling.
- **Countdowns:** elapsed-time countdowns don't change with the time zone. A countdown with a time of day follows its reminder's setting.
- **Daylight-saving changes:**
  - A time that doesn't exist (2:30 on the spring-forward night) fires at 3:00.
  - A time that happens twice (1:30 on the fall-back night) fires once, the first time.
  - Days and longer are calendar days. Hours and shorter are elapsed time.

### Time-based conditions

- **The conditions:**
  - days of the week
  - a time window, which can cross midnight
  - a date range or season
  - daylight or darkness
- **They're checked on the device and applied when predicting.**
- **They never make a reminder wait.** A trigger outside one simply passes.
- **First release:** they apply to schedules and sun events; one-offs and countdowns take none (see [ADR 0012](../adr/0012-sun-events-and-time-conditions-are-computed-from-a-synced-home-location.md)).

### Overdue times and expiry delays

Both can be a duration after firing ("1 hour") or the next time a schedule matches after firing ("the next 1st at 00:00"). Delays produce no expected occurrences.

## Calendar

- **How each platform reads calendars:**
  - **Android:** the system calendar provider (`CalendarContract`). It needs no Google project, and it covers every calendar the phone syncs, whether through Google's sync or DAVx5.
  - **Linux:** CalDAV from our own Rust code, signing in to Google through the system browser with a loopback redirect ([ADR 0003](../adr/0003-linux-calendars-via-caldav.md)). The same code works for other CalDAV servers later. Akonadi (C++ only), Evolution Data Server (private D-Bus interface) and Google's Calendar API were rejected.
- **Google sign-in on Linux:**
  - The app ships with the owner's Google Cloud project and OAuth client, published but unverified. The household accepts Google's warning screen once.
  - A setting lets anyone self-hosting enter their own client ID.
- **Other providers later:** the calendar source class has internal providers (the Android calendar provider, CalDAV). Exchange or Outlook would be a new provider module.
- **A calendar source identifies one calendar** (an account plus a calendar). A device can sense it only if that calendar is on the device.
- **Trigger:** *N minutes before or after an event starts or ends*, for events matching a filter: which calendar, words in the title, and busy events only or all. It's a **scheduled instance**, so the phone and the desktop fire the same occurrence and it merges.
- **Conditions:**
  - *during a busy event*
  - *not during a busy event*. Declined events and those marked "free" don't count.
  - *today has an event matching …*
- **Predictable:** expected occurrences appear on the timeline and update as events change.
- **When events change:**
  - **Moved:** pending firings move with the event.
  - **Declined by you:** ignored.
  - **Cancelled or deleted before firing:** the expected occurrence disappears.
  - **Cancelled after firing:** the occurrence stays open, noted "event was cancelled", and you close it. It isn't closed automatically, because a skip is always a person's choice.
- **Shared lists** can use a calendar their users share, such as a family calendar. Each user's device that has it evaluates. Your personal calendar drives only your personal reminders.
- **Event details stay private:**
  - An occurrence carries the reminder's own text, never the event's title or details.
  - Your own devices can show the matching event alongside it.
  - Calendar events never leave the device.
- **On the timeline:**
  - Each device chooses which of its calendars to lay over the timeline, read-only.
  - Busy events show as blocks behind occurrences.
  - Clicking one opens it in the system calendar app.

## Places

- **A place** is a source with a name, a centre and a radius, such as Home or the gym.
  - **Radius:** 150 m by default, adjustable, with a minimum of 100 m.
  - **Belongs to** a list or a user. A shared list's Home is one place for everyone using that list.
- **The device's current position isn't a source** users set up. It's something a device can provide to other source classes, such as sunset "wherever I am".
- **Triggers:** *arrive at* and *leave*. Both are events.
- **Conditions:** *at* and *not at*.
- **Debounce:** you count as having left only once you're a margin beyond the radius for a few minutes. A fix whose uncertainty straddles the edge doesn't count as crossing it.
- **Finding location on Android:**
  - **With Google Play Services:** geofencing.
  - **Without it:** the framework's proximity alerts, which can be about 30 minutes late. The editor says so. An opt-in **precise location** mode (a foreground service with an ongoing notification) samples location itself, at a battery cost.
- **Wi-Fi networks listed on a place:**
  - **What they count toward:** connecting to one counts toward *arriving* and toward *at the place*, even without a location fix. They never count toward leaving.
  - **How they're detected:** with the Wi-Fi class's detection. They're part of the place's settings, not separate sources.
- **Desktops don't sense location.** Reminders that need it are left to phones.
- **"When anyone is home"** means anyone whose phone is at that place. Each phone checks its own user.
- **Other sources refer to a place by link** ("weather at Home"). Moving or resizing the place updates them, and a place in use can't be deleted.
- **Faking:**
  - The level is **moderate**, with the guard "combine with a home Wi-Fi or Bluetooth condition".
  - Mock-location fixes are ignored, and the occurrence is noted "location looked faked".

## Weather

- **Weather warnings** come from a **region module**, with the US module (NWS) first.
  - A region module says whether it covers a location. It returns active warnings normalised to CAP fields, including their update and cancel lifecycle.
  - It maps the region's event names to kinds the app understands, and declares its terms.
- **Forecasts come from NWS where it has data for the location, falling back to Open-Meteo.**
  - Open-Meteo is also used when NWS is erroring or rate-limiting at the moment of a poll. An affected occurrence is noted "forecast from Open-Meteo (NWS unavailable)".
  - The phone and the desktop may use different providers for the same place at the same moment, which is accepted.
- **Warnings have no fallback,** since Open-Meteo has none. Persistent failures show in the sources banner.
- **Outside every warnings region,** warning triggers are unavailable and the editor says so. Forecast conditions still work through Open-Meteo.
- **Triggers:**
  - **A warning is issued for a place.** It can be filtered by event type (Tornado Warning, Severe Thunderstorm Warning…) or by minimum severity, and by whether it's a warning or also a watch or advisory. The default filter is CAP severity Severe or Extreme, with urgency Immediate or Expected.
  - **A warning ends** (expires or is cancelled).
  - **Both are events.** Each warning's message ID is its identity, and an update to the same warning joins the open occurrence. A warning cancelled after the reminder fired leaves the occurrence open, with a note.
- **Conditions:** forecasts are used only as conditions. "Frost tonight" is a schedule plus a condition, such as *at 18:00, if tonight's low is below 0 °C*. Expected occurrences of such reminders are tentative. The conditions:
  - rain or snow expected within N hours, at a chance of at least P%
  - today's or tonight's low or high, above or below a temperature
  - raining now, or not
  - a warning of a kind, or any warning, in effect, or none
- **Polling:**
  - Every 15 minutes on every device, and only for places that some active reminder uses.
  - The server doesn't poll, and devices don't share what they see.
- **The editor says,** for warning triggers: "Warnings can reach you 15 minutes late or more. This isn't a safety alert. Your phone's emergency alerts are."
- **Faking:** the level is **none**, since warnings and forecasts are fetched over HTTPS from the provider.
- **NWS `User-Agent`** follows RFC 9110's product and comment form, with the repository URL as the contact, for example `hab-bot/0.1.0 (+https://github.com/csnook/hab-bot)`.
- **Open-Meteo attribution** (CC BY 4.0) is shown wherever forecast values appear, and in About.

## Connections: Bluetooth, Wi-Fi and USB

| | Android | Linux |
|---|---|---|
| Bluetooth | Yes | Yes (BlueZ over D-Bus) |
| Wi-Fi | Yes. Joining wakes the app, but *leaving* is noticed only when the app next runs, up to about 15 minutes late. The editor says so. | Yes (NetworkManager over D-Bus) |
| USB | No: a phone plugged into a car or computer sees only "power connected" | Yes |

- **Triggers:** *connected* and *disconnected*. Both are events.
- **Conditions:** *connected to* and *not connected to*.
- **Flapping:** a disconnect counts only after about a minute without reconnecting. That covers car Bluetooth dropping briefly, and Wi-Fi hopping between access points.
- **Identifying a device or network:**
  - **Bluetooth:** chosen from the paired devices, and identified by address, never by name.
  - **Wi-Fi:** the network name plus its security type, optionally pinned to specific access points.
  - **USB:** vendor and product ID plus serial number. With no serial number, the editor warns that any identical model will match.
- **Portable and stationary devices:**
  - A connection is a fact about a device, not about where you are. The desktop is always on the home Wi-Fi.
  - Each device is **portable** or stationary. It's set automatically (Android phones are portable, and Linux computers are stationary unless a battery suggests a laptop), and can be changed.
  - Wi-Fi and Bluetooth sources count only portable devices by default. A source can choose otherwise, such as "when the desktop's headset connects". USB counts any device.
- **Faking:**

  | Source | Level | Guard shown |
  |---|---|---|
  | Wi-Fi, open network | Easy | Anyone can broadcast your network's name. Combine it with a place. |
  | Wi-Fi, password-secured | Moderate | Anyone with the password could fake it. |
  | Wi-Fi, certificate-secured (WPA3 Enterprise and similar) | Difficult | |
  | Bluetooth | Moderate | Needs specialist tools and someone nearby who knows the device. |
  | USB | Moderate | Needs specialist knowledge and a cheap board, but is easy once someone has those. Combine it with a place or Wi-Fi. |

  - **Wi-Fi security** is asked when setting up the source ("open, password or certificate?"). It's pre-filled wherever the device can detect it without extra permissions, and the level follows the answer.
  - **The editor also notes** that other apps can't forge these events, except on a rooted phone.

## Webhooks

- **Each webhook source has its own unguessable URL** on the server, `https://<server>/hooks/<random>`.
  - Anyone with edit access to its list can see it and **rotate** it, which kills the old one.
  - A source can also require the body to be **signed** with a shared secret (HMAC), for senders that support it.
- **Trigger:** *webhook received*. It can be filtered on the payload by exact matches on a field path or query parameter (such as `event` equals `wash_done`), with no scripting.
- **Condition:** *the latest value received for a field is …* (such as `door` equals `closed`), optionally "only if received within N minutes". This lets a webhook report state as well as events.
- **Matching happens on devices,** since reminders are encrypted.
- **Identity and repeats:**
  - The server gives each received webhook an ID, so every device sees the same event and fires one occurrence.
  - There's no deduplication by content. A retry, or a doorbell pressed twice, joins the open occurrence under the event-joining rule.
- **Delivery:**
  - As soon as a payload arrives, the server seals it to the devices that hold the source's list. It keeps only the sealed copies, until every relevant device has fetched them or for 7 days at most. It still sees the payload in passing.
  - It sends a high-urgency, content-free push.
- **Late events fire late.** Overdue and expiry count from when the server received the event, so an old event may already be expired, and is then recorded without alerting.
- **Standalone desktops:** a desktop can also accept a webhook itself.
  - It listens on **localhost only**, at the same path and with the same secret, so scripts on that computer can trigger reminders with no server.
  - Phones don't listen.
- **Reaching the server:**
  - **Port:** webhooks share port 443 with sync by default, and can be given their own port and IP address. A deployment can then expose only webhooks (see [Running the server](sync-and-server.md#running-the-server)).
  - **The editor** says whatever the deployment allows, such as "reachable only on your home network".
- **Limits:** at most 60 requests a minute per URL, and bodies of at most 64 KiB. The excess is refused with "too many requests".
- **Faking:** **moderate** with the URL alone ("anyone who learns the URL can trigger it; use a signature if the sender supports one"), and **difficult** with a signature.

## Decided in

- [Time source class: schedules, repeating from the last completion, time zones][9]
- [Which device evaluates each reminder][15]
- [How a source class plugs in][16]
- [Location source class: places][17]
- [Calendar source class: Google Calendar first][18]
- [Weather source class][19]
- [Connection source classes: Bluetooth, Wi-Fi and USB][20]
- [Webhook source class][21]
- [Security hardening before the server goes on the internet][33]
- **Research:**
  - [US weather data](../research/us-weather-data.md)
  - [Google Calendar access](../research/google-calendar-access.md)
  - [Connection detection and spoofing](../research/connection-detection-spoofing.md)
  - [Android background work](../research/android-background-tauri.md)
  - [Linux desktop integration](../research/linux-desktop-integration.md)

[9]: https://github.com/csnook/hab-bot/issues/9
[15]: https://github.com/csnook/hab-bot/issues/15
[16]: https://github.com/csnook/hab-bot/issues/16
[17]: https://github.com/csnook/hab-bot/issues/17
[18]: https://github.com/csnook/hab-bot/issues/18
[19]: https://github.com/csnook/hab-bot/issues/19
[20]: https://github.com/csnook/hab-bot/issues/20
[21]: https://github.com/csnook/hab-bot/issues/21
[33]: https://github.com/csnook/hab-bot/issues/33
