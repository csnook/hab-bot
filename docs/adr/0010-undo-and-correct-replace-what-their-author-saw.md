# Undo and correct replace the closings their author saw, and nothing else

An occurrence can be closed by several devices out of touch (a completion beats a skip, which beats a miss; of one kind the first in the stream counts; closing beats snoozing). A deliberate undo or correction must not be read as one of those disagreements, and every device must reach the same answer whatever order the events arrive in. So undo and correct are events of their own (format 9): `OccurrenceCorrected` and `OccurrenceUndone`. Each names the closings its author could see, `replaces` (the ones that count and everything they in turn replaced). An occurrence is settled from all its records: those a correction or undo replaced are kept in the history, marked, and of the rest the usual rules pick the one that counts. A closing made on another device meanwhile is not named, so it stands.

The device that undoes decides where the occurrence ends up and says so in the event (`reopened`, `expected`, or `missed` as of a time), because "would it still be open" depends on the clock and the expiry, and the other devices must not each work it out for themselves.

## Decisions

- **Undo reopens only if it would still be open:** no newer occurrence fired and no expiry has passed (spec). Otherwise the undo leaves a missed closing, as of the expiry or of the newer firing. A countdown then counts from that earlier time, which moves it back. A reopened countdown occurrence is open, so the countdown waits for it to close again.
- **Closed ahead of its time and undone before that time comes** (completing early, skipping ahead): the occurrence is expected again, not open. It exists in the stream as an occurrence opened early (`fired_at` before `scheduled_at`) with its closing replaced; it fires at its time as the same occurrence, from any device, and a countdown or schedule goes back to what it was. Undone after its time has come, it is judged like any other.
- **A repeating reminder keeps one open occurrence (ADR 0001):** a reopened occurrence is closed as missed at once on every device that has a newer one, in whatever order the events arrive. So an undo made before hearing of a newer firing still ends missed.
- **A miss can't be undone**, only corrected: nobody chose it. An undo or correction replaces everything before it, so undoing a corrected closing reopens the occurrence rather than stepping one back.
- **Correct** can change any closed occurrence, a miss too, to completed or skipped at a time. The history keeps both. A miss corrected to completed counts as done late, however many corrections come after.
- **Two devices acting on the same closing:** two undos are one. An undo and a correction: the correction stands (a closing beats a reopening). Two corrections: the usual rules, so a completion beats a skip, and the device whose skip lost is told. An undo made before the device had heard of another device's closing leaves that closing standing, without a notice, because the undo was deliberate.
- **Early closing** of a schedule's next instance needs nothing open for the reminder (otherwise opening the later occurrence would miss the open one, ADR 0001); of a countdown or a one-off, the same. It is refused with "complete or skip the open one instead".
- **Not enforced yet:** who may undo or correct (the user who closed it, edit access on a shared reminder, act access for a miss). Personal lists have one user. Sharing adds the check where the action is made, not in the merge.

## Considered options

- **An undo that deletes the closing event:** can't be done on an append-only, encrypted log, and loses the original the history must keep.
- **A hybrid logical clock on corrections (latest wins):** a correction would then override a closing it never saw, and an undo could lose to an older skip on another device. Naming what was seen is causal and needs no clock.
- **Letting each device decide whether an undo reopens:** devices with different clocks or expiries would disagree about the same occurrence.

## Consequences

- Events in format 9 are kept unapplied by an older app, which shows the update notice.
- The server's receive time is kept beside each event once it has been numbered, shown in the history with the time said and the time tapped.
