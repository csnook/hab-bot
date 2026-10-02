# Alerts, priorities and snoozing

> **First release:**
> - **In:** the five built-in priorities with all four alert styles and escalation, the full-screen alarm on Android, the Linux alarm window, the last-chance alert, snoozing (including ahead of time and snooze all), quiet hours, the Do Not Disturb check, each device's loudest style and "quiet this device", and the tidy-list rules.
> - **Later:** custom priorities and the priority editor. The cap on other users' reminders arrives with sharing.

## Alert styles

An **alert** is how an occurrence gets attention. Each of the user's devices that holds the reminder alerts.

| Style | Android | Linux |
|---|---|---|
| **Silent** | A notification in the shade, with no sound or vibration | A low-urgency notification with no sound |
| **Gentle** | A pop-up notification with one sound and vibration, which settles into the shade | A normal notification with a themed sound, which times out |
| **Insistent** | Gentle, repeated every interval until someone acts | The same |
| **Alarm** | A full-screen alarm with looping sound and vibration, rising in volume over 30 seconds | A critical notification that doesn't time out, with the same buttons. The app plays a looping sound and opens its alarm window, though Wayland may not focus it. |

## Priorities

Each reminder has a **priority**, which sets how insistently its open occurrences alert.

### What a priority is made of

- **Style when due.**
- **Style once overdue,** which can escalate in steps: "after X overdue, switch to style Y".
- **Due interval.** While the occurrence is due, it is:
  - the snooze length
  - how long until the occurrence goes overdue
  - how long a wait lasts by default
  - how long a later event joins the occurrence instead of firing again (see [Expiry](reminders.md#expiry))

  An interval of 0 means overdue at once.
- **Overdue interval.** Once overdue, it is the snooze length, the repeat interval, and the quiet period after acknowledging.
- **Ring duration:** how long an alarm rings. By default, the whole interval, meaning until someone acts.
- **Server wait:** how long the check with the server before each alert may hold it up (see below).
- **Swipeable on Android:** yes or no. A successful swipe is always a snooze for the current interval.
- **Breaks Do Not Disturb:** yes or no.

### The built-in priorities

| | Due style | Due interval | Once overdue | Overdue interval | Swipe (as snooze) | Server wait | Breaks Do Not Disturb |
|---|---|---|---|---|---|---|---|
| **Minimum** | Silent (only listed, on desktop) | 1 day | Silent, in the overdue list | 1 day | Yes | 60 s | No |
| **Low** | Gentle | 1 day | Silent, in the overdue list | 1 day | Yes | 60 s | No |
| **Medium** | Gentle | 1 h | Insistent, then Alarm after 1 h overdue | 10 min | Yes | 60 s | No |
| **High** | Alarm | 0 | Alarm, repeating | 10 min | No | 60 s | No |
| **Maximum** | Alarm | 0 | Alarm, repeating | 10 min | No | None | Yes |

- **Minimum and Low** are to-do-like, and differ only in how they notify.
- **High and Maximum** mirror the stock Android alarm clock.
- **Alerts keep escalating** until the occurrence closes. Snoozing or acknowledging pauses them.

### Custom priorities

- **A user makes one** by copying a built-in and editing any of its settings.
- **They belong to that user** and sync across their devices.
- **Built-ins can't be edited.**

### Priorities on shared reminders

- **A shared reminder's own priority** must be a built-in one.
- **Each user can override it** with any of their own, for a whole list ("Pets: Minimum for me") or a single reminder. The reminder's override wins.
- **The override changes only how the occurrence reaches that user:** alert style, snooze length, whether it folds in their Overdue section, and whether it counts in their badge.
- **Timing comes from the reminder's own priority,** the same for everyone: when it goes overdue, how long a wait lasts, and whether an event joins or fires again.
- **Anyone with view access** is never alerted.

### Defaults that follow the priority

- **Which defaults:** a reminder's overdue time, wait and event-joining window come from its priority until the reminder overrides them.
- **Changing the priority:** changing a reminder's priority, or editing a custom priority, moves every reminder still on the default.
- **In the editor,** each default shows greyed with its source, such as "after 1 h (Medium)".

## Reminders from other users

- **Each user has a cap:** the highest priority other users' reminders can reach them at.
- **It's asked when joining a server,** with Maximum preselected, and can be changed later.
- **The UI discourages a cap below Medium** ("reminders from others may be easy to miss").
- **The user's own overrides aren't capped.** The cap limits only what others choose.

## Do Not Disturb

- **The app checks Do Not Disturb itself:** Android's interruption filter, or the Linux notification server's "inhibited" state.
- **While it's on,** every priority that doesn't break Do Not Disturb is downgraded to silent. It catches up afterwards at its current level.
- **Maximum** uses Android's alarm category. Do Not Disturb access is requested on first start, so that Maximum gets through even strict modes. On Linux, Maximum uses a critical notification plus the app's own sound.
- **The phone's own Do Not Disturb schedules** are honoured.

## Settings per device

- **Loudest style:** each device can cap the loudest style it uses, for example no alarms on the desktop. Maximum still gets through unless the cap says otherwise.
- **Quiet this device until…** downgrades everything on that device to silent. It isn't a snooze, so other devices still alert. Maximum is left out unless included.

## Quiet hours

- **Quiet hours are a recurring snooze-all,** set per user, such as 22:00 to 07:00 on weeknights.
- **They cover** all of the user's reminders or one list.
- **Maximum is left out** by default.

## Alerting on several devices

- **Every device that holds the reminder and knows the occurrence is open alerts,** each within its own loudest-style cap and quiet setting.
  - That includes a device that only learned of the occurrence from the server. The desktop can't sense arriving home, but alerts once it learns the phone fired.
  - A device that learns late alerts at the occurrence's current state, due or overdue.
  - No device stands in for the others, and there's no primary device.
- **Acting on any device applies to all of them,** including acknowledging. Acknowledging the phone's alarm silences the desktop's current alert too, and both repeat at the next interval.
- **Checking with the server before every alert:**
  - Every firing, and every repeat of an insistent alert or an alarm, checks briefly with the server when it can be reached. That catches an occurrence closed elsewhere that hasn't synced yet.
  - The check is capped by the priority's server wait. Maximum rings at once and checks at the same time.
  - When the server can't be reached, the device alerts without waiting.
- **How quickly other devices must stop:**
  - **A ringing alarm** stops within about 5 seconds of you acting on another device, whenever both can reach the server. On Android the ringing alarm runs as a foreground service and keeps a live connection while it rings.
  - **A desktop** keeps a live connection whenever it can reach the server, and updates within seconds.
  - **A notification that isn't ringing** updates at the device's next check or sync. Push makes this faster, but nothing depends on it.

## Snoozing

- **A snooze** quiets an occurrence's alerts for you until a duration, a time or a trigger ("until I get home"), whichever comes first. The occurrence stays open.
- **One tap on Snooze** uses the priority's current interval:
  - Minimum and Low: 1 day
  - Medium: 1 hour while due, 10 minutes once overdue
  - High and Maximum: 10 minutes

  The full choice is in the opened notification and on the alarm screen.
- **There's no limit** on repeated snoozes. Each is recorded.
- **Going overdue:** a snooze doesn't stop the occurrence going overdue on schedule. It does so quietly, and when the snooze ends it alerts at its current level.
- **Expiry:** a snooze can't silently turn into a miss. If an expiry at a known time falls inside the snooze, a **last-chance alert** comes shortly before it, and the picker shows "expires at 23:59". An expiry that's a trigger, including an event firing again, can't be foreseen and simply applies.
- **Swiping** a notification away is a snooze for the current interval, recorded as made by swiping. High and Maximum can't be swiped.
- **On shared occurrences:**
  - A snooze only ever affects your own alerts.
  - If you've claimed the occurrence, your snooze doesn't hold back the others' overdue alert, and the picker warns you: "Sam will be alerted at 20:00 if it's still open."
- **Ahead of time:** you can snooze an expected occurrence ("make the 7:00 medicine 7:30"). It still fires at 7:00, quietly, into the lists, and alerts at 7:30. Its overdue time and expiry still count from 7:00.

### Snooze all

- **Scope:** all your reminders, or one list. It covers everything open now, plus anything that fires before the snooze ends.
- **Maximum** is left out by default, with a clear "include maximum" option.
- **All devices:** it applies on all your devices.
- **The same rules as a single snooze:**
  - occurrences still go overdue on schedule
  - last-chance alerts still come
  - warnings about claimed shared occurrences are gathered into one list
- **Recording:** each affected occurrence records its own snooze, marked as part of a snooze-all.
- **Ending early:** a snooze-all can be ended early. Everything held back then alerts at its current level.

### The last-chance alert

- **Style:** the priority's due style, never quieter than Gentle.
- **When:** 10 minutes before a known expiry that falls inside a snooze.
- **Text:** "Last chance: Call the plumber · Expires at 23:59".

## Notification buttons

**Done · Snooze** always come first, followed by one button that depends on the situation. Tapping the notification opens the occurrence with every action. Linux uses the same sets.

| Situation | Buttons |
|---|---|
| Personal, Minimum to Medium | Done · Snooze · Skip |
| Shared, unclaimed | Done · Snooze · Claim |
| Shared, claimed by you | Done · Snooze · Release |
| Alarm (High, Maximum) | Done · Snooze · Acknowledge. The full-screen alarm adds Skip. |

## The alarm screens

- **Android, full screen:**
  - "Ringing · list · priority", the title and note, when it was due, and whose turn it is
  - a large **Done**
  - **Snooze ▾** with 5, 10 or 30 minutes, "until…" and custom
  - Acknowledge and Skip…, plus Claim or Release on shared reminders

  The volume rises over 30 seconds. Without the full-screen permission, the alarm is a large pop-up notification with the same buttons.
- **Linux:** an alarm window with the same layout, alongside a critical notification with Done · Snooze · the third button. Closing either one silences both.

## Keeping the lists tidy

- **The Overdue section** (Inbox, the Board's column, Android and the web client) sorts by highest priority first, then by longest overdue.
- **Old quiet occurrences fold:** Minimum and Low occurrences overdue for more than 7 days fold into one row at the end, "4 older quiet reminders ▸". Medium and above never fold. Priority here is each user's own, including their overrides.
- **Skip all…** on that row skips your personal occurrences and declines shared ones, leaving those to the others. Its dialog says how many of each.
- **The tray badge** counts Low and above, and is red only when something Medium or above is overdue. Minimum never counts. The web client's app badge follows the same rule.
- **Android notifications:** Minimum and Low go into one collapsed "quiet reminders" group with a summary line, and each keeps its buttons when expanded. Medium and above stay separate.

## Decided in

- [How overdue occurrences are handled][8]
- [Completing, skipping and acknowledging occurrences][12]
- [How snoozing works][13]
- [Alert styles and escalation][22]
- [One person, several devices][23]
- [Desktop dialogs and settings][29]
- [Android screens][30]
- [Default expiries and keeping the lists tidy][32]

[8]: https://github.com/csnook/hab-bot/issues/8
[12]: https://github.com/csnook/hab-bot/issues/12
[13]: https://github.com/csnook/hab-bot/issues/13
[22]: https://github.com/csnook/hab-bot/issues/22
[23]: https://github.com/csnook/hab-bot/issues/23
[29]: https://github.com/csnook/hab-bot/issues/29
[30]: https://github.com/csnook/hab-bot/issues/30
[32]: https://github.com/csnook/hab-bot/issues/32
