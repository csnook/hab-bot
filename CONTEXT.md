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

**Occurrence**:
One instance of a reminder coming due, such as this morning's 7:00 medicine. Snoozing and completing apply to an occurrence, not to the reminder.

### Alerts

**Alert**:
How an occurrence gets someone's attention, anywhere from a quiet notification to a loud alarm.
_Avoid_: Alarm, notification (each is one kind of alert)
