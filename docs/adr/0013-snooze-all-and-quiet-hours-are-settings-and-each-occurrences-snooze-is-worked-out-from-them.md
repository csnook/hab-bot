# Snooze all and quiet hours are personal settings, and each occurrence's snooze is worked out from them

A snooze-all (spec: Alerts → Snooze all) quiets everything open and anything that fires before it ends; quiet hours are the same, recurring. Both are the user's own, so they live in the personal list's stream like the home location (ADR 0012), as three events, all format 12: `SnoozeAllStarted` (an id, a scope of everything or one list, whether Maximum is included, when it began and when it ends), `SnoozeAllEnded` (by id; the event's recorded time is when it was ended early) and `QuietHoursSet` (every rule as one value, merged by hybrid logical clock, the later wins; a rule is the days a stretch starts on, a time of day from and to, a scope and whether Maximum is included).

Nothing is written per occurrence. A device works out whether an occurrence is held at an instant from the settings alone (`State::hold_on`): a snooze-all holds it if it reaches its list and priority (Maximum only if included), has begun and not ended, and the occurrence was due before it ends; quiet hours hold it while a stretch is in progress, following stretches that run into each other. `Core::inbox` then puts the end of the hold in the item's `snoozed_until`, with no moment it was set, so the alerter, the tray and the window already treat it as a snooze: it goes overdue on schedule, quietly; Do Not Disturb, acknowledging and pausing behave as before; and it alerts at its current level when the hold ends. A single snooze that lasts longer still wins. What an occurrence's history shows ("snoozed by snooze-all until 19:00, ended early", "by quiet hours") is worked out the same way (`State::holds_over`), marked with its source.

The last-chance alert comes through a snooze-all and quiet hours as through any snooze, 10 minutes before a known expiry inside the hold. Because the dialog cannot show each occurrence's expiry the way the snooze menu does, it also comes if the hold began after that moment: a hold never silently turns into a miss.

Quiet hours are read in the zone each device is in, as floating reminders are. A stretch belongs to the day it starts on: "weeknights" is Monday to Friday, and Friday's runs into Saturday morning.

## Considered Options

- **A snooze event per affected occurrence, marked as part of the snooze-all**: the spec's literal wording ("each affected occurrence records its own snooze"). Rejected: occurrences that fire inside a snooze-all fire on every device, so every device would write one (the first-wins rule that merges firings does not apply to snoozes, which are a history); ending a snooze-all early would need an event per occurrence too, and a device that hears of a snooze-all after an occurrence fired would hold nothing until it had also heard of each snooze. Quiet hours would need a device awake at 22:00 to write one for everything open, and again for every night. The derived record carries the same facts (who, when it began, what it was set to end on, how it ended, its source) and cannot disagree between devices.
- **Quiet hours as a stored list of dated stretches**: rejected: stretches would have to be generated ahead by some device.
- **Making the tray badge count only what is not held**: rejected; a snoozed occurrence is open and counts (see `tray_model`).

## Consequences

- An older app (format 11) keeps these events without applying them, so it goes on alerting through a snooze-all and quiet hours until updated. Devices of one user should be updated together.
- Editing quiet hours restates the history of occurrences already open during past stretches, since their records are read from the rules as they stand.
- They live in the user's personal list, so on a shared list they affect only that user's own alerts, as a single snooze does.
- A standalone device that signs in to an account has its own personal list kept as a list of its own (`link_account`): its snooze-alls and quiet hours stay in that stream and no longer apply; the account's do.
- "Until I get home" is not offered (places and arriving are not built); the dialog takes lengths and a time of day.
- Several snooze-alls can hold at once (one for everything and some for lists); starting one of the same scope ends the earlier. The longest-lasting hold counts for an occurrence.
- The tray's Snooze all offers lengths for everything with Maximum left out; "More choices…" opens the dialog.
