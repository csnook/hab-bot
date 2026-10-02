# A reminder has at most one open occurrence

When a reminder fires while its previous occurrence is still open, the previous one is closed as missed; occurrences never stack up. We chose this over letting them stack because stacking invites the wrong action, such as a double dose of medicine after a forgotten one, and one open occurrence per reminder keeps the lists and notifications simple. "The reminder fired again" is therefore always part of every reminder's expiry.

## Considered Options

- **Stacking**: every occurrence stays open until closed. Rejected: yesterday's and today's medicine would both be open at once.
- **Per-reminder choice between stacking and replacing**: rejected. The rule holds for every reminder, so there is no setting for it.

## Consequences

- An occurrence can be missed without anyone seeing it, if the reminder fires again before its alerts reach anyone.
- Reminders that repeat from the last completion never fire again on their own, so without an added expiry their occurrence stays open until someone closes it.
- "Firing again" means a new instance of a schedule or countdown, or an event trigger (arriving somewhere, a connection, a webhook) once the open occurrence is overdue. While the occurrence is still due, or within 5 minutes of its firing, an event trigger joins it instead, alerting the user whose device sensed it. Due and overdue follow the reminder's own priority, the same for everyone it's shared with. Two devices firing the same occurrence produce one occurrence.
- Nothing else expires an occurrence by default. One-off reminders, countdowns, and occurrences fired by an event that doesn't recur stay open until someone closes them, and the lists fold old quiet ones away instead.
