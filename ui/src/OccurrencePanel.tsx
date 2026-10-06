import { useEffect, useRef, useState } from "preact/hooks";
import {
  closedOccurrence,
  completeEarly,
  completeOccurrence,
  correctOccurrence,
  pauseReminder,
  recentSkipNotes,
  skipAhead,
  skipOccurrence,
  undoOccurrence,
  type ClosedView,
  type DueItem,
  type EarlierItem,
  type ExpectedItem,
} from "./api";
import { SnoozeMenu } from "./SnoozeMenu";
import {
  buttonsFor,
  cleanNote,
  earlierLabel,
  historyLine,
  noteChoices,
  outcomeLabel,
  saidTime,
  undoMessage,
} from "./closing";
import { nextWeekOf, pauseUntil, tomorrowOf } from "./pause";
import { formatTime } from "./time";

/** What the panel is about: an open, expected or closed occurrence. */
export type PanelTarget =
  | { state: "open"; item: DueItem }
  | { state: "expected"; item: ExpectedItem }
  | { state: "closed"; item: EarlierItem };

const nowSeconds = () => Math.floor(Date.now() / 1000);

/**
 * The occurrence details panel with the spec's buttons: Done, Snooze ▾, Skip
 * and More ▾ (Done at a different time, Edit reminder) on an open occurrence;
 * Complete early, Skip ahead and Snooze ahead on an expected one; Undo and
 * Correct on a closed one, with the history of how it was closed.
 *
 * Pause, in the More menu of an open occurrence and beside Edit on an expected
 * one, sets the reminder aside until a day or until resumed (pause.ts); the
 * occurrence open now is skipped by it.
 *
 * Type-checked and unit-tested for its pure logic (closing.ts, pause.ts) only:
 * it has never been run in a real window.
 */
export function OccurrencePanel({
  target,
  onClose,
  onEdit,
}: {
  target: PanelTarget;
  onClose: () => void;
  onEdit: (reminderId: string) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [recent, setRecent] = useState<string[]>([]);
  const [note, setNote] = useState("");
  const [when, setWhen] = useState("");
  const [closed, setClosed] = useState<ClosedView | null>(null);
  // Which form is showing: skipping with a note, or correcting.
  const [form, setForm] = useState<"skip" | "correct" | "pause" | null>(null);
  const [pauseDate, setPauseDate] = useState(() => nextWeekOf(nowSeconds()));
  const [untilResumed, setUntilResumed] = useState(false);
  const [kind, setKind] = useState<"completed" | "skipped">("completed");

  useEffect(() => {
    const d = ref.current;
    if (d && !d.open) {
      try {
        d.showModal();
      } catch {
        d.setAttribute("open", "");
      }
    }
  }, []);
  useEffect(() => {
    recentSkipNotes().then(setRecent).catch(() => {});
  }, []);
  useEffect(() => {
    if (target.state === "closed") {
      closedOccurrence(target.item.occurrence_id).then(setClosed).catch(() => {});
    }
  }, [target]);

  const run = (what: Promise<unknown>, then?: (r: unknown) => void) =>
    what
      .then((r) => {
        setError("");
        if (then) then(r);
        else onClose();
      })
      .catch((e) => setError(String(e)));
  const time = () => {
    const t = saidTime(when, nowSeconds());
    if (!t.ok) setError(t.error);
    return t.ok ? t.value : null;
  };

  const timeField = (label: string) => (
    <label>
      {label}
      <input type="datetime-local" value={when} onInput={(e) => setWhen(e.currentTarget.value)} />
    </label>
  );
  const noteField = () => (
    <>
      <label>
        Note (optional)
        <input value={note} onInput={(e) => setNote(e.currentTarget.value)} />
      </label>
      <div class="alarm-row">
        {noteChoices(recent, note).map((n) => (
          <button type="button" key={n} onClick={() => setNote(n)}>
            {n}
          </button>
        ))}
      </div>
    </>
  );

  // Pause the reminder until a day, or until it is resumed.
  const pauseForm = (reminderId: string) => (
    <form
      aria-label="Pause"
      onSubmit={(e) => {
        e.preventDefault();
        const until = pauseUntil(untilResumed, pauseDate, nowSeconds());
        if (!until.ok) return setError(until.error);
        run(pauseReminder(reminderId, until.value));
      }}
    >
      <label class="radio">
        <input type="radio" name="pause-end" checked={!untilResumed} onChange={() => setUntilResumed(false)} />
        Until
        <input
          type="date"
          aria-label="Paused until"
          min={tomorrowOf(nowSeconds())}
          value={pauseDate}
          disabled={untilResumed}
          onInput={(e) => setPauseDate(e.currentTarget.value)}
        />
      </label>
      <label class="radio">
        <input type="radio" name="pause-end" checked={untilResumed} onChange={() => setUntilResumed(true)} />
        Until I resume it
      </label>
      <p class="muted">What falls in the pause is skipped, this one too.</p>
      <button>Pause</button>
    </form>
  );

  let title = "";
  let body: preact.ComponentChild = null;
  if (target.state === "open") {
    const d = target.item;
    title = d.title;
    const buttons = buttonsFor("open");
    body = (
      <>
        <p class="muted">Due {formatTime(d.scheduled_at)}</p>
        <div class="alarm-row">
          <button onClick={() => run(completeOccurrence(d.occurrence_id))}>{buttons.main[0]}</button>
          <SnoozeMenu target={{ occurrenceId: d.occurrence_id }} onError={setError} />
          <button onClick={() => setForm(form === "skip" ? null : "skip")}>{buttons.main[2]}</button>
          <details>
            <summary>{buttons.main[3]}</summary>
            <button onClick={() => setForm("correct")}>{buttons.more[0]}</button>
            <button onClick={() => setForm(form === "pause" ? null : "pause")}>{buttons.more[1]}</button>
            <button onClick={() => { onEdit(d.reminder_id); onClose(); }}>{buttons.more[2]}</button>
          </details>
        </div>
        {form === "pause" && pauseForm(d.reminder_id)}
        {form === "skip" && (
          <form onSubmit={(e) => { e.preventDefault(); run(skipOccurrence(d.occurrence_id, cleanNote(note))); }}>
            {noteField()}
            <button>Skip</button>
          </form>
        )}
        {form === "correct" && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const at = time();
              if (at !== null) run(completeOccurrence(d.occurrence_id, at));
            }}
          >
            {timeField("Done at (it may be before it fired)")}
            <button>Done at this time</button>
          </form>
        )}
      </>
    );
  } else if (target.state === "expected") {
    const e = target.item;
    title = e.title;
    body = (
      <>
        <p class="muted">Expected {formatTime(e.scheduled_at)}</p>
        <div class="alarm-row">
          {e.can_close_early && (
            <>
              <button onClick={() => setForm(form === "correct" ? null : "correct")}>Complete early</button>
              <button onClick={() => setForm(form === "skip" ? null : "skip")}>Skip ahead</button>
            </>
          )}
          <SnoozeMenu
            target={{ reminderId: e.reminder_id, scheduledAt: e.scheduled_at }}
            label="Snooze ahead"
            onError={setError}
          />
          <button onClick={() => setForm(form === "pause" ? null : "pause")}>Pause</button>
          <button onClick={() => { onEdit(e.reminder_id); onClose(); }}>Edit reminder</button>
        </div>
        {form === "pause" && pauseForm(e.reminder_id)}
        {form === "correct" && (
          <form
            onSubmit={(ev) => {
              ev.preventDefault();
              const at = time();
              if (at !== null) run(completeEarly(e.reminder_id, at), () => { setMessage("Completed early. It won't fire."); setForm(null); });
            }}
          >
            {timeField("Done at (now if left empty)")}
            <button>Complete early</button>
          </form>
        )}
        {form === "skip" && (
          <form
            onSubmit={(ev) => {
              ev.preventDefault();
              run(skipAhead(e.reminder_id, cleanNote(note)), () => { setMessage("Skipped ahead. It won't fire."); setForm(null); });
            }}
          >
            {noteField()}
            <button>Skip ahead</button>
          </form>
        )}
      </>
    );
  } else {
    const c = target.item;
    title = c.title;
    const buttons = buttonsFor("closed", { canUndo: c.can_undo });
    body = (
      <>
        <p>
          {closed ? outcomeLabel(closed.outcome, closed.paused) : earlierLabel(c)} · {formatTime(c.closed_at)}
          {c.corrected && " (corrected)"}
        </p>
        <div class="alarm-row">
          {buttons.main.includes("Undo") && (
            <button
              onClick={() =>
                run(undoOccurrence(c.occurrence_id), (r) => {
                  setMessage(undoMessage(r as Parameters<typeof undoMessage>[0]));
                  setClosed(null);
                })
              }
            >
              Undo
            </button>
          )}
          <button onClick={() => setForm(form === "correct" ? null : "correct")}>Correct</button>
        </div>
        {form === "correct" && (
          <form
            onSubmit={(e) => {
              e.preventDefault();
              const at = time();
              if (at !== null) run(correctOccurrence(c.occurrence_id, kind, at, kind === "skipped" ? cleanNote(note) : undefined));
            }}
          >
            <label class="radio">
              <input type="radio" name="kind" checked={kind === "completed"} onChange={() => setKind("completed")} />
              Completed
            </label>
            <label class="radio">
              <input type="radio" name="kind" checked={kind === "skipped"} onChange={() => setKind("skipped")} />
              Skipped
            </label>
            {timeField("At (now if left empty)")}
            {kind === "skipped" && noteField()}
            <button>Correct</button>
          </form>
        )}
        {closed && closed.history.length > 0 && (
          <>
            <h3>History</h3>
            <ul>
              {closed.history.map((h) => (
                <li key={h.event_id} class={h.superseded ? "replaced" : undefined}>
                  {historyLine(h, formatTime)}
                </li>
              ))}
            </ul>
          </>
        )}
      </>
    );
  }

  return (
    <dialog ref={ref} class="editor panel" aria-labelledby="panel-title" onClose={onClose}>
      <header>
        <h2 id="panel-title">{title}</h2>
        <button type="button" aria-label="Close" onClick={onClose}>×</button>
      </header>
      {message && <p role="status">{message}</p>}
      {error && <p class="error" role="alert">{error}</p>}
      {body}
    </dialog>
  );
}
