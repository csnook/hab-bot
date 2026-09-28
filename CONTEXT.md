# Reminders

A reminder app that prompts people to do things at the right moment (at a time, at a place, when the weather turns) and records who did them.

## Language

### Reminders and firing

**Reminder**:
A standing rule for prompting someone to do something, such as "take medicine every day at 7:00".
_Avoid_: Task, alarm

**Trigger**:
Whatever prompts an evaluation of a reminder: a schedule reaching its time, arriving at a place, a severe-storm warning being issued. A reminder is evaluated only when one of its triggers occurs.

**Condition**:
Something else that must be true at evaluation for a reminder to fire, such as being at home. A condition changing never prompts an evaluation by itself.

**Evaluation**:
One check, prompted by a trigger, of whether a reminder's conditions are met and so whether it should fire.

**Fire**:
What a reminder does when an evaluation finds its conditions met: it produces an occurrence and alerts.

**Schedule**:
A trigger that occurs at set times by the calendar and clock, such as every weekday at 7:00 or the first Monday of the month.
_Avoid_: Recurrence

**Countdown**:
A trigger that occurs a set time after the reminder's last occurrence was closed, such as 3 days after the plants were last watered. Completing, skipping or missing an occurrence restarts it.
_Avoid_: Interval, rolling repeat

### Sources

**Source**:
Something configured that the app watches, such as a specific calendar, the car's Bluetooth or the weather at home, supplying triggers, conditions or both. This is what users see and set up.

**Source class**:
A kind of source as the app implements it (calendar, Bluetooth, weather), before any configuration. Only developers deal in source classes; new ones arrive with app updates.
_Avoid_: Source type, provider, plugin

### Occurrences

**Occurrence**:
One instance of a reminder coming due, such as this morning's 7:00 medicine. Snoozing, completing and skipping apply to an occurrence, not to the reminder.

**Expected occurrence**:
A prediction that a reminder will come due at a given time, such as tomorrow's 7:00 medicine. When that time comes it becomes an occurrence only if the reminder fires.
_Avoid_: Upcoming occurrence, scheduled occurrence

**Open**:
Said of an occurrence that has fired and has not yet been completed, skipped or missed. A reminder has at most one open occurrence at a time. An open occurrence is either due or overdue.

**Due**:
Said of an open occurrence that is not yet overdue.

**Overdue**:
Said of an open occurrence that has stayed open longer than its reminder allows. Its priority sets how long by default, and a reminder can override it.

**Complete**:
To close an occurrence by recording that it was done, by whom and when.

**Skip**:
To close an occurrence without doing it, optionally recording a reason.

**Missed**:
Said of an occurrence the app closed because it expired before anyone completed or skipped it. Unlike a skip, nobody chose it.

**Expiry**:
The triggers that make a reminder's open occurrence missed, whichever occurs first. The reminder firing again is always one of them.
_Avoid_: Timeout (that is an alert stopping by itself)

### Alerts

**Alert**:
How an occurrence gets someone's attention, anywhere from a quiet notification to a loud alarm.
_Avoid_: Alarm, notification (each is one kind of alert)

**Acknowledge**:
To silence an occurrence's current alert without closing it. The occurrence stays open and can still become overdue or missed.
_Avoid_: Dismiss

**Priority**:
The level chosen for a reminder that sets how insistently its open occurrences alert, from a silent notification up to an alarm that breaks through Do Not Disturb. Some priorities are built in (minimum, low, medium, high, maximum); users can define their own.
_Avoid_: Urgency, importance, severity (severity belongs to weather warnings)
