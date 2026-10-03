# Reminders

A reminder app that prompts people to do things at the right moment (at a time, at a place, when the weather turns) and records who did them.

## Language

### Reminders and firing

**Reminder**:
A rule for prompting someone to do something, either repeatedly, such as "take medicine every day at 7:00", or once, such as "call the plumber tomorrow at 9:00". A one-off reminder is finished once its occurrence is closed.
_Avoid_: Task, alarm

**Trigger**:
Whatever prompts an evaluation of a reminder: a schedule reaching its time, arriving at a place, a severe-storm warning being issued. A reminder is evaluated only when one of its triggers occurs.

**Condition**:
Something else that must be true at evaluation for a reminder to fire, such as being at home. A condition changing never prompts an evaluation by itself, except while the reminder is waiting.

**Evaluation**:
One check, prompted by a trigger, of whether a reminder's conditions are met and so whether it should fire.

**Fire**:
What a reminder does when an evaluation finds its conditions met: it produces an occurrence and alerts.

**Waiting**:
Said of a reminder whose trigger occurred while its conditions were not met, and which will fire as soon as they are, until its wait ends. Each reminder chooses whether to wait or let the trigger pass. Only conditions that can't be predicted, such as being at home, make a reminder wait: a trigger that falls outside a time-based condition, such as weekdays only, simply passes.

**Watching**:
Said of a reminder that has nothing open or expected and fires only when an event trigger occurs, such as arriving home, a weather warning or a webhook.
_Avoid_: Waiting (that is a trigger that occurred while the conditions weren't met), listening

**Schedule**:
A trigger that occurs at set times by the calendar and clock, such as every weekday at 7:00 or the first Monday of the month.
_Avoid_: Recurrence

**Countdown**:
A trigger that occurs a set time after the reminder's last occurrence was closed, such as 3 days after the plants were last watered. Completing, skipping or missing an occurrence restarts it.
_Avoid_: Interval, rolling repeat

### People and accounts

**User**:
Someone who uses the app, usually a person. A screen that several people share, such as a kitchen tablet, is a user of its own.

**Account**:
A user's registration on a server. A standalone user has no account, and a user has at most one. Say "account" only when joining, signing in, recovering or deleting, and "user" otherwise.

**Invite**:
A single-use link or QR code that lets someone create an account on a server. Only server admins make them.
_Avoid_: Invitation (that is for groups and shares)

**Sign-in code**:
A link, also shown as a QR code, that any signed-in device can show to let another device of the same user find the server and pin its certificate. It carries the server's name, address and certificate fingerprint and nothing secret: signing in still takes the username and password.
_Avoid_: Invite (that creates an account), ticket

**Approval**:
Letting a new device in from an existing signed-in one, instead of typing the password on it. One device shows a link, also as a QR code, and the other scans it or is given it pasted. The existing device shows the new device's name, asks for confirmation, and then signs the new device and gives it the account's keys.
_Avoid_: Pairing, sign-in code (that only finds the server)

**Device**:
A phone, tablet, computer or browser running the app, belonging to exactly one user. Whatever a user does to an occurrence applies on all of their devices.

**Web client**:
The app running in a web browser, loaded from a static web host rather than from the user's server. Each browser that signs in is a device of its own.
_Avoid_: Web app, website

**Server admin**:
A user who runs a server's accounts: inviting people, removing or resetting accounts and releasing escrow. A server admin can't read anyone's encrypted data or change their access.
_Avoid_: Admin on its own

### Sharing

A reminder can be shared with users individually, or with a group as a whole.

**Group**:
A defined set of users, such as a household, that reminders can be shared with as a whole.
_Avoid_: Household (one kind of group), family, team

**Member**:
A user who belongs to a group. Use "member" only when talking about a group, and "user" otherwise.

**Group admin**:
A member who can add and remove the group's members and make other members group admins. A group has no other roles.
_Avoid_: Admin on its own

**Invitation**:
An offer to join a group, or to receive a reminder list or reminder shared with you individually, which you accept or decline. A list shared with a group reaches its members without one.
_Avoid_: Invite (that is for accounts), refuse

**Reminder list**:
A named collection that every reminder belongs to exactly one of, like a calendar in a calendar app. Each user has a private personal list; other lists can be shared with users and groups.
_Avoid_: Calendar, project, board

**Access**:
What a user can do with a reminder list, or with a reminder shared with them directly: view (see it, never alerted), act (also alerted, and can claim, decline, complete, skip, snooze and acknowledge), edit (also create, change and delete reminders) or manage (also share it and change access). A user with access in several ways, such as through a group and individually, gets the highest. Everyone with access can see who else has it.
_Avoid_: Role (roles belong to groups)

**Claim**:
To take on an open occurrence of a shared reminder, telling everyone else it is shared with that you will do it. When several users claim the same occurrence, the first claim the server receives wins.

**Decline**:
To step out of an open or expected occurrence of a shared reminder yourself, stopping your alerts and leaving it to the others. Unlike a skip, it does not close the occurrence.
_Avoid_: Pass, skip for me

**Assign**:
To claim a shared reminder's occurrences ahead of time on another user's behalf.

**Rotation**:
Users taking turns on a shared reminder. The next turn goes to whoever in the rotation did it least recently, and each turn is an assignment.

### Privacy

**Server trust**:
A device's limit, set per server, on how much it lets that server read. Reminder content, history and source settings are always encrypted so that only the devices of users with access can read them; the server reads only accounts, groups, access and timing. Server trust decides whether the device allows anything more, such as escrow.

**Standalone**:
Said of the app used on a device with no server. Everything works on that device; sharing, several devices and recovery need a server.

**Password**:
The secret a user remembers, from which their devices derive the key that unlocks that user's own keys, such as when signing in on a new device. It never leaves their devices.

**Escrow**:
An optional arrangement where the server keeps a copy of a user's personal keys and releases it after verifying the user, for someone who can no longer unlock them. It lets whoever runs the server read that user's personal data.

**Retention limit**:
How far back history is kept, if not forever. A user sets one for their personal list, a group's admins set one for lists shared with the group, and a server's admins set one that caps everything on it. A list or reminder shared with individual users rather than a group is kept on the server for the server's limit, while each user's devices keep it for that user's own limit.

**Verify**:
To confirm that a user is who they say they are. A server admin verifies a user before releasing their escrow; someone with manage access verifies a user whose account was reset before a list comes back to them. Until then, that user is unverified on that list.

### Sources

**Source**:
Something configured that the app watches, such as a specific calendar, the car's Bluetooth or the weather at home, supplying triggers, conditions or both. This is what users see and set up.

**Source class**:
A kind of source as the app implements it (calendar, Bluetooth, weather), before any configuration. Only developers deal in source classes; new ones arrive with app updates.
_Avoid_: Source type, provider, plugin

**Place**:
A source that is a named area, with a centre and a radius, such as Home or the gym. Like any source it belongs to a reminder list or to one user, so a shared list's Home is one place for everyone using that list.
_Avoid_: Location (that is where a device is), geofence

**Portable**:
Said of a device that goes where its user goes, such as a phone, so its connections and location say something about where the user is. Other devices, such as a desktop, are stationary. By default only portable devices count for Wi-Fi and Bluetooth sources.

**Weather warning**:
An official watch, warning or advisory issued by a weather agency for an area, such as a Severe Thunderstorm Warning. Issuing one can trigger a reminder; it is not itself an alert.
_Avoid_: Weather alert (alert means how an occurrence gets attention)

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
To close an occurrence without doing it, optionally with a note. An expected occurrence can be skipped ahead of time.

**Pause**:
To set a reminder or a reminder list aside for a period, skipping its occurrences in that period. On a shared reminder a user can instead pause just for themselves, declining them.

**Missed**:
Said of an occurrence the app closed because it expired before anyone completed or skipped it. Unlike a skip, nobody chose it.

**Correct**:
To change how or when a closed occurrence was closed, such as turning a missed occurrence into one completed at 9:40. The original closing stays in the history.

**Expiry**:
The triggers that make a reminder's open occurrence missed, whichever occurs first. The reminder firing again is always one of them: a new schedule or countdown instance, or an event trigger once the occurrence is overdue. While the occurrence is due, or just after it fired, an event trigger joins it instead.
_Avoid_: Timeout (that is an alert stopping by itself)

### Alerts

**Alert**:
How an occurrence gets someone's attention, anywhere from a quiet notification to a loud alarm. Each of their devices that holds the reminder alerts, not just the one that fired it.
_Avoid_: Alarm, notification (each is one kind of alert)

**Snooze**:
To quiet an occurrence's alerts for yourself until a chosen time or trigger. The occurrence stays open and still goes overdue on schedule. An expected occurrence can be snoozed ahead of time. Snoozing all of a user's reminders, or one reminder list, at once also covers anything that fires before the snooze ends.
_Avoid_: Postpone, delay

**Acknowledge**:
To silence an occurrence's current alert for yourself, on all your devices, without closing it. The occurrence stays open and can still become overdue or missed.
_Avoid_: Dismiss

**Priority**:
The level chosen for a reminder that sets how insistently its open occurrences alert, from a silent notification up to an alarm that breaks through Do Not Disturb. It sets an alert style and an interval for while an occurrence is due and for once it is overdue; the interval is also the snooze length. Some priorities are built in (minimum, low, medium, high, maximum); users can define their own.
_Avoid_: Urgency, importance, severity (severity belongs to weather warnings)

**Alert style**:
How loud one alert is: silent, gentle, insistent (gentle, repeated each interval) or alarm (ringing until someone acts).

**Quiet hours**:
A snooze-all that recurs on a schedule, such as 22:00 to 07:00 on weeknights. Like any snooze-all it leaves out maximum priority unless told otherwise.
_Avoid_: Do Not Disturb (that is the operating system's own setting)

### History

**History**:
The record of what happened to a list's reminders and occurrences: firings, alerts, claims and every action on them, edits, evaluations that didn't fire, and waits, each with who and when. It is kept back to the retention limit that applies. Other users see who did something, never which of their devices.

**Late**:
Said of an occurrence completed while it was overdue, including a missed one corrected to completed. One completed while due was done on time.

**Reliability**:
How often a reminder's occurrences get done: those completed, on time or late, out of those completed or missed. Skipped occurrences and evaluations that didn't fire don't count against it.
