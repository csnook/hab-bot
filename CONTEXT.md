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
Said of an occurrence that has fired and has not yet been completed or skipped.

**Overdue**:
Said of an open occurrence that is past the time it should have been done by.

**Complete**:
To close an occurrence by recording that it was done, by whom and when.

**Skip**:
To close an occurrence without doing it, optionally recording a reason.

### Alerts

**Alert**:
How an occurrence gets someone's attention, anywhere from a quiet notification to a loud alarm.
_Avoid_: Alarm, notification (each is one kind of alert)
