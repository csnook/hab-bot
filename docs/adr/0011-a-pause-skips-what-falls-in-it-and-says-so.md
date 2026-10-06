# A pause skips the occurrences that fall in it, and the history says the pause did

Pausing a reminder or a reminder list "skips its occurrences in that period" (spec). The spec leaves open what that means for an instance that would have fired meanwhile, for the occurrence open when the pause starts, and for devices that disagree about the pause. So that every device reaches the same answer, a pause is data and what it skips is recorded.

A pause is `{ from, until }`: `from` is when it was made and `until` is a time, or none for "until resumed". It covers an instant `t` when `from <= t < until`. A reminder's pause is one of its settings (`Setting::Pause`, merged per setting by hybrid logical clock like every other, ADR 0005); resuming is the same setting with no value. A list's pause is `Event::ListPaused`, carrying a clock too, and the latest by clock counts. A reminder is paused at `t` by its own pause or by its list's, whichever covers `t`. It is the list it is in now that counts (ADR 0009), so a reminder moved into a paused list is paused, and one moved out is not.

## Decisions

- **An instance that falls in a pause is skipped and recorded**, not dropped silently, not marked missed (nobody failed to act) and not delayed until the pause ends. `Event::OccurrenceSkippedForPause` opens the occurrence closed, as skipped, dated at the instance's own time. Every device that ticks writes the same occurrence id and the same date, so the same skip from several devices is one skip with no notice about it. The history marks the skip with what paused it and until when ("Skipped by the pause until 3 March", or "the list's pause").
- **A countdown that runs out in a pause is skipped as of when it ran out**, and so restarts from there (a skip restarts a countdown). A three-day countdown paused for five days skips once, and fires three days after that skip, not the moment the pause ends. A one-off whose time falls in a pause is skipped, and so finished; the editor says what a pause does to it.
- **The occurrence open when a pause starts is skipped by it,** as of that moment, on every way of pausing (the editor's Pause section, Pause in an occurrence's More menu, pausing a list). Pausing is for being left alone, and a card that stays open but silent would be neither paused nor done. Undo is as for any skip.
- **An open occurrence that exists under a pause anyway** (a device fired it before it heard of the pause, or a skip was undone) stays open, in `Inbox::paused` rather than Overdue or Due. It never alerts, standing alerts for it are closed on the next pass, and the tray's badge, tooltip and rows leave it out. Its alerts begin again if it is still open when the pause ends. They are not "Waiting", which is for reminders whose conditions aren't met; the Board's Paused column is `Core::paused_reminders`.
- **Resuming early** sets the pause aside from then on. What was skipped stays skipped. Before any change to a pause (making, moving the end, resuming) the device first records the skips for what has come due in it as it stood, so that a device waking after a resume doesn't fire late what the pause covered.
- **Pausing a pause keeps where it began**: moving the end of a pause that is on keeps `from`; pausing again after it ran out starts a new one.
- **`Core::next_fire_at` still counts instances in a pause.** Nothing alerts, but the skip has to be recorded when its time comes (so the history says so and a countdown restarts), so the scheduler wakes for it. `Core::expected` leaves them out: they will not fire.
- **Event triggers don't fire during a pause** (spec). None exist yet. `Core::is_paused` is what they ask.
- **Pausing for yourself on a shared reminder, which declines,** comes with sharing.

## Several devices

- Two devices pausing or resuming out of touch: the later clock wins on every device, whatever order the server numbers the events in, and the other value stays in the history to be restored.
- A device that doesn't know of a pause yet fires and alerts as before, like one that doesn't know of a deletion (ADR 0009). When it hears, its alert is closed and its open occurrence is set aside. A device that records a skip before it hears of a resume leaves a skip that stands; the user can undo it.
- A pause skipping an occurrence that another device completes meanwhile loses to the completion, as any skip does (ADR 0010), and the device that paused is told.

## Considered options

- **Resume the countdown or schedule from the moment the pause ends**: rejected. It makes a pause change what is expected, not skip it, and needs the device to know when a pause ended.
- **Derive the skips from the pause when asked, recording nothing**: rejected. Undo, correct, the countdown's restart and the reliability figures all want an occurrence to act on.
- **Leave the open occurrence open and quiet**: rejected for the usual paths; it is what remains for the unusual ones.
- **A pause as a flag on the list or reminder that devices read at the time**: rejected. A late device would then fire what a resume had passed over.

## Consequences

- New events are format 10 (`Event::format`): the pause setting, `ListPaused` and `OccurrenceSkippedForPause`. An older app keeps them without applying them and shows the update notice, so **an app that hasn't updated keeps firing and alerting a paused reminder**, and records its own firings as before.
- A pause with no end on a short countdown or a frequent schedule records a skip for every instance (at most the latest 50 are recorded when a device was away). That is the history the spec asks for.
