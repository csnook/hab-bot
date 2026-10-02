# History

> **First release:** the history is recorded from the first day, since it is the event stream. The History view (with its Reliability and Group tabs), the reliability line in panels, and the export come later.

**History** is the record of what happened to a list's reminders and occurrences: firings, alerts, claims and every action on them, edits, evaluations that didn't fire, and waits, each with who and when. It's kept back to the retention limit that applies (see [Retention and deletion](sync-and-server.md#retention-and-deletion)).

## What's recorded

- **The history is each list's event stream** ([ADR 0005](../adr/0005-encrypted-event-log-numbered-by-server.md)).
- **Most events already exist because sync needs them:**
  - firings from each device, and edits to settings
  - claims (including ones that lost or were released), declines and assignments
  - completions, skips, snoozes and acknowledgements
  - undos and corrections, keeping the original
  - access changes
- **Three kinds exist only for the history:**
  - **Alerts:** per user, the first alert, each change of style (escalation) and the last-chance alert, with the device that alerted. Repeats of an insistent alert or an alarm aren't recorded.
  - **Evaluations that didn't fire:** the trigger, and the conditions that weren't met. When several devices evaluate the same trigger, their records merge into one, as firings do.
  - **Waits:** when a wait started, and how it ended (fired, wait ended, or stopped by you).
- **Snoozes** record what they were set to end on, how they actually ended, whether they came from a swipe, and whether they were part of a snooze-all or quiet hours.
- **Completions keep three times:** the time you said it was done, when you tapped, and when the server received it.
- **Other users see who, never which device.** Device names live in each user's own encrypted settings, so on shared lists others see "Sam", not "Sam's phone".
- **Former members:** the history keeps their name on past actions, marked as a former member. Shared screens appear under their own user, such as "Kitchen (shared screen)".
- **Deleted reminders** keep their history, marked deleted, unless deleted with their history.

## Times

- **Grouping:** days and periods follow the occurrence's scheduled time, in the reminder's own time zone (floating local time unless pinned).
- **On time or late** is judged by the time you said it was done.
- **Order,** such as who claimed first, follows the server.
- **Travel and daylight saving:** every event also stores UTC and the device's time-zone offset, so nothing becomes ambiguous.

## The reliability view

For each reminder and each list, over 7 days, 30 days (the default), 90 days, 1 year, everything kept, or "since…". The view always says what period it covers.

- **Outcomes:**
  - **done on time:** completed while due
  - **done late:** completed while overdue, including a miss corrected to completed
  - **skipped**
  - **missed**
  - **didn't fire,** counted separately and belonging to neither side
- **Reliability** = done ÷ (done + missed). Skips don't count against it and are shown beside it. The **on-time rate** = on time ÷ (done + missed) sits alongside.
- **Snoozes** don't count against a reminder. The view shows the usual number of snoozes before it was done. At two or more it adds "often snoozed; a different time may suit it better".
- **Time to do it:** the usual time from firing to completion.
- **Everyone declined:** a shared occurrence that everyone declined and nobody did counts as missed, flagged "everyone declined".
- **Misses from an event firing again** count like any other miss.

## The group view

- **Scope:** each shared list, and each user over time.
- **Per user:**
  - completions and their share of the total
  - completions on someone else's turn
  - declines
  - misses while it was their turn. A turn they declined doesn't count as their miss.
- **Only while they had access:** a user counts only for the time they had access, with a note for anyone who joined partway.
- **Who sees it:** anyone with access to the list. Personal lists never appear.
- **Weighting:** every occurrence counts the same. Effort weighting is out of scope.

## Where the views live

- **History** is the fifth view in the desktop window, on Android and in the web client, with **Reliability** and **Group** tabs.
- **The reminder and occurrence panels** show a one-line summary ("Last 30 days: done 26 of 29, 3 late") with a link to History.
- **The views are computed on devices,** since the server can't read the history.
- **Layout:** see [Linux desktop app](desktop.md#history-view) and [Android app](android.md#the-views).

## Export

- **What:** everything you have access to, with occurrences as CSV and the full event history as JSON.
- **The record already supports it,** so nothing about the record has to change.
- **Where:** History's Export… button.

## Decided in

- [Completing, skipping and acknowledging occurrences][12]
- [How snoozing works][13]
- [Sync design][25]
- [History records and the two views][27]
- [Desktop dialogs and settings][29]

[12]: https://github.com/csnook/hab-bot/issues/12
[13]: https://github.com/csnook/hab-bot/issues/13
[25]: https://github.com/csnook/hab-bot/issues/25
[27]: https://github.com/csnook/hab-bot/issues/27
[29]: https://github.com/csnook/hab-bot/issues/29
