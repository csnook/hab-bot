# Reminders and occurrences

> **First release:**
> - **In:** reminders and their own lists, schedules, countdowns, one-offs, sun events and time-based conditions, calendar triggers and conditions (read on Android only), waiting (on the calendar's busy condition only), expiry, and every action on personal occurrences.
> - **Later:** event triggers, which arrive with places, connections, weather and webhooks. Claims, declines and turns arrive with sharing.

## Reminders and reminder lists

- **A reminder** is a rule for prompting someone to do something.
  - It repeats (a schedule or a countdown) or happens once (a **one-off**). A one-off is finished once its occurrence closes.
  - It has a title, a note, a reminder list, a priority, triggers, conditions, what to do when the conditions aren't met, an optional overdue override, an expiry, turns (shared reminders only) and a pause.
- **Reminder lists:** every reminder belongs to exactly one list, like a calendar in a calendar app.
  - Each user has a private **personal list**, which is the default.
  - Users can make more lists. Lists other than the personal list can be shared (see [People, sharing and accounts](sharing-and-accounts.md)).
  - Reminders can move between lists and keep their history.
- **Deleting a reminder** keeps its history, marked deleted, unless you choose "delete with its history".

## Triggers, conditions and evaluation

- **A trigger** prompts an **evaluation**, which checks whether the reminder's **conditions** hold. If they do, the reminder **fires**: it produces an occurrence and alerts.
- **Two kinds of trigger:**
  - **Scheduled instances:** schedules, countdowns, sun events and calendar events. They can be predicted.
  - **Events:** arriving or leaving, connecting or disconnecting, a weather warning, a webhook.
- **A condition changing never prompts an evaluation by itself,** except while the reminder is waiting (below).
- **Conditions combine with AND.** Each source class supplies its own opposites ("not at Home"). OR and grouping may come later without breaking existing reminders.
- **Conditions about a person mean "anyone"** who meets them. Each device checks its own user, so nobody's whereabouts are shared.
- **When a condition can't be checked:**
  - **Temporarily** (permission revoked, no location fix): it counts as met. The occurrence notes "couldn't check: at Home", since being forgotten is the costlier mistake.
  - **Never** (the device lacks the capability, such as a desktop with no location): that device doesn't evaluate the reminder. It leaves it to devices that can, and shows the occurrence once it syncs.

### Which device evaluates

- **Every device that holds a reminder evaluates it,** and the device that senses a trigger fires it.
  - Every device senses time.
  - Only a phone senses places and Bluetooth.
  - A device that can read a calendar senses that calendar's triggers.
- **The server never evaluates reminders.** It passes on webhook events and orders claims.
- **Each device holds the lists chosen for it,** all by default. The editor warns when no device holding a list can sense one of its reminder's triggers ("this list isn't on any device that can sense *leaving Home*").
- **Firings merge across devices** into one occurrence:
  - **Schedules and countdowns:** the occurrence's identity is the reminder plus the scheduled time, so even offline firings merge.
  - **Calendar triggers:** the reminder, the event's UID, its instance start time and the offset. If Android doesn't expose the UID reliably, the title and start time are used instead.
  - **Events:** firings of the same reminder within 5 minutes merge, keeping the earliest firing time. Webhook events carry an ID from the server, and weather warnings their own message ID.

## Waiting

- **When a trigger occurs but the conditions aren't met,** each reminder either **waits** (the default) or **lets the trigger pass**. An umbrella-if-rain reminder would let it pass.
- **While waiting,** the conditions becoming true acts as a trigger. For example, the Tuesday 19:00 trash with "at Home" fires when you get home.
- **Only conditions that can't be predicted make a reminder wait:** places, connections, weather and calendar busy. A trigger outside a time-based condition, such as "weekdays only", simply passes.
- **A wait ends** at whichever comes first:
  - **the reminder's next trigger**, always
  - **by default, the priority's due interval after the trigger:** up to 1 day for Minimum and Low, and 1 hour for Medium. High and Maximum (due interval 0) wait until the next trigger.
  - a duration, a schedule match or a source trigger, if the reminder sets one instead

  Midnight is never special, so people whose days don't follow the clock aren't caught out.
- **After waiting, the reminder fires with full time:** its overdue time and expiry count from when it fired.
- **Countdowns with conditions always wait,** with no default end. If a wait they were given ends without firing, the countdown restarts from that moment.
- **When a reminder doesn't fire** (it let the trigger pass, or its wait ended), there is no occurrence. That counts as neither done nor missed. The evaluation is recorded, and a faint marker stays on the timeline ("didn't fire: not at Home"). No alert says so.
- **Waiting reminders** show in a Waiting group with no alerts. **Done anyway** completes the reminder early and ends the wait. **Stop waiting** ends the wait without firing.
- **On shared reminders:**
  - Only the users who meet the conditions are alerted. Everyone else can still see the occurrence and claim it.
  - Someone who meets them later, while the occurrence is open and unclaimed, is alerted then.
  - If it's Sam's turn but Sam doesn't meet the conditions, the occurrence fires unclaimed to whoever does, and Sam stays up next time.
  - The wait's length follows the reminder's own priority.

## Occurrences

- **An occurrence** is one instance of a reminder coming due. Snoozing, completing and skipping apply to an occurrence, not to the reminder.
- **One open occurrence per reminder** ([ADR 0001](../adr/0001-one-open-occurrence-per-reminder.md)). Occurrences never stack up, so a forgotten dose can't invite a double dose.
- **States:**

  | State | Meaning |
  |---|---|
  | Expected | Predicted to come due at a given time. It becomes an occurrence only if the reminder fires. |
  | Due | Open, and not yet overdue. |
  | Overdue | Open for longer than the reminder allows. |
  | Completed | Closed by someone recording that it was done, by whom and when. |
  | Skipped | Closed without doing it, by someone's choice, with an optional note. |
  | Missed | Closed by the app because it expired. Nobody chose it. |

- **Going overdue:**
  - By default it happens after the priority's **due interval**: 1 day for Minimum and Low, 1 hour for Medium, and 0 for High and Maximum.
  - A reminder can override this with a duration, or with the next time a schedule matches. The rent reminder fires on the 28th and is overdue at "the next 1st at 00:00".
  - The default follows the priority until the reminder overrides it.
- **Expected occurrences:**
  - **Prediction range:** schedules and sun events predict as far ahead as the timeline shows. A countdown predicts only its next occurrence.
  - **Conditions:** time-based conditions are applied when predicting, so a weekdays-only reminder shows nothing on Saturday. Reminders whose conditions can't be predicted show **tentative** expected occurrences, marked with the condition.
  - **Calendar triggers:** their expected occurrences update as events change.

## Expiry

- **A reminder's expiry** is a set of triggers. Whichever occurs first marks the open occurrence **missed**.
- **Firing again is always one of them:**
  - a new schedule or countdown instance
  - an event trigger, **once the occurrence is overdue**
- **While an occurrence is due, or within 5 minutes of its firing,** an event trigger **joins** it instead of firing again, alerting the user whose device sensed it. Take "when anyone gets home, bring in the post" at Low: Sam arriving 30 minutes after you is alerted about the same occurrence, but the next day's arrival fires again and the old occurrence is missed.
- **Users can add** a delay after firing ("1 hour later"), the next time a schedule matches ("the next 23:59"), or a source trigger ("when I leave Home"). These can take conditions, using the same mechanism as triggers. A trigger such as "leave Home" counts whenever it occurs after the firing.
- **Nothing else expires an occurrence by default.** These stay open until someone closes them, and the lists fold old quiet ones away instead (see [Alerts](alerts.md#keeping-the-lists-tidy)):
  - one-offs
  - countdowns, which restart only once their occurrence closes
  - occurrences fired by an event that doesn't recur
- **On a shared reminder,** overdue time, expiry and whether an event joins or fires again follow the reminder's own priority, the same for everyone.
- **Late firings** (the device was off or asleep):
  - The device first checks with the server whether another device already closed the occurrence.
  - If it's still open, it fires late. Its overdue time and expiry still count from the scheduled time, so it may already be overdue, or be missed at once.
  - If the next firing has already passed, the late one is recorded as missed.

## Acting on occurrences

Actions belong to the user, not the device: whatever you do to an occurrence applies on all of your devices.

- **Complete** ("Done"):
  - **The recorded time** defaults to the moment you tap, and you can edit it, even to before the firing ("took it at 6:55").
  - **Early:** completing early closes the **next expected occurrence**, which then never fires. A countdown restarts and a wait ends. Reminders with no expected occurrences (such as "when I arrive at the gym") don't offer early completion.
  - **Who:** anyone with act access, from any device, offline included.
- **Skip** closes an occurrence for everyone, with an optional note and recent notes offered. Expected occurrences can be skipped ahead of time. On a shared reminder anyone with act access can skip, and the skip is attributed ("skipped by Sam").
- **Decline**, on shared reminders only, steps you out without closing it ("I can't do it this time"):
  - It stops your alerts and leaves the occurrence to the others.
  - It releases your claim, and if it was your turn, the occurrence goes unclaimed to the others while you stay up next time.
  - It takes an optional note, and works ahead of time too.
  - **If everyone declines,** the occurrence stays open and notifications quietly update to "everyone has declined". Anyone can still claim or complete it until it expires. Otherwise it's missed, flagged "everyone declined".
- **Pause:**
  - Pausing a reminder or a reminder list until a date skips its occurrences in that period. Event triggers don't fire during a pause.
  - On a shared reminder you can instead pause just for yourself, which declines them.
  - Pausing a group's list for yourself with no end date is how you opt out of it without leaving the group.
- **Acknowledge** silences the current alert on all your devices without closing anything. While due, it quiets the occurrence until it goes overdue. While overdue, it quiets it for one overdue interval, and then escalation resumes.
- **Snooze** is covered in [Alerts](alerts.md#snoozing).
- **Claim, release and assign** are covered in [People, sharing and accounts](sharing-and-accounts.md#claims-and-turns).
- **Undo:** undoing a completion or skip reopens the occurrence if it would still be open (not expired, and no newer occurrence fired). Otherwise it becomes missed. A countdown moves back.
- **Correct:** any closed occurrence, including a missed one, can be changed to completed or skipped, with a time. The history keeps both the original and the correction, and a miss corrected to completed counts as done late.
- **Who can undo or correct:** the user who closed it, or anyone with edit access on a shared reminder. For a miss, anyone with act access.
- **Acting on someone else's claim:** when the server can be reached, you're asked first ("Sam claimed this at 19:05. Complete it anyway?"). If you acted offline, you're told afterwards and your completion is recorded too. Sam gets a quiet notice, and Sam's alerts stop.
- **When your devices disagree:** two devices may act on the same occurrence without syncing. When they meet:
  - matching actions merge silently, and the first one counts
  - a completion beats a skip or a miss, and you're told
  - closing beats snoozing or acknowledging
- **Not sent yet:** an action the server hasn't received yet is marked "not sent yet" until it's sent.

## Decided in

- [How overdue occurrences are handled][8]
- [What happens when a reminder's conditions aren't met][11]
- [Completing, skipping and acknowledging occurrences][12]
- [Which device evaluates each reminder][15]
- [How a source class plugs in][16]
- [One person, several devices][23]
- [Default expiries and keeping the lists tidy][32]
- [ADR 0001](../adr/0001-one-open-occurrence-per-reminder.md)

[8]: https://github.com/csnook/hab-bot/issues/8
[11]: https://github.com/csnook/hab-bot/issues/11
[12]: https://github.com/csnook/hab-bot/issues/12
[15]: https://github.com/csnook/hab-bot/issues/15
[16]: https://github.com/csnook/hab-bot/issues/16
[23]: https://github.com/csnook/hab-bot/issues/23
[32]: https://github.com/csnook/hab-bot/issues/32
