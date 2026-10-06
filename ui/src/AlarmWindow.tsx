import { useEffect, useState } from "preact/hooks";
import {
  acknowledgeOccurrence,
  alarmView,
  completeOccurrence,
  onStateChanged,
  skipOccurrence,
  snoozeOccurrence,
  type OccurrenceView,
} from "./api";
import { dueLine, ringingHeading, SNOOZE_MINUTES } from "./alarm";
import { expiryNote, expiryWarning, nextTimeOfDay, clock } from "./snooze";

/** The alarm window: what is ringing, with a large Done. Closing the window
 * silences the alarm; the Rust side closes it when the occurrence is acted on. */
export function AlarmWindow({ occurrenceId }: { occurrenceId: string }) {
  const [view, setView] = useState<OccurrenceView | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  const [skipping, setSkipping] = useState(false);
  const [note, setNote] = useState("");
  const [snoozing, setSnoozing] = useState(false);
  const [until, setUntil] = useState("");

  useEffect(() => {
    const load = () => alarmView(occurrenceId).then(setView).catch((e) => setError(String(e)));
    load();
    const unlisten = onStateChanged(load);
    return () => {
      unlisten.then((f) => f());
    };
  }, [occurrenceId]);

  const act = (f: () => Promise<void>) => f().catch((e) => setError(String(e)));

  if (view === undefined) return <main class="alarm" />;
  if (view === null) {
    return (
      <main class="alarm">
        <p class="empty">This reminder is no longer open.</p>
      </main>
    );
  }
  return (
    <main class="alarm">
      <p class="ringing">{ringingHeading(view)}</p>
      <h1>{view.title}</h1>
      <p class="muted">{dueLine(view.scheduled_at, Math.floor(Date.now() / 1000))}</p>
      {error && <p class="error" role="alert">{error}</p>}
      <button class="done" onClick={() => act(() => completeOccurrence(occurrenceId))}>
        Done
      </button>
      <div class="alarm-row">
        <button aria-expanded={snoozing} onClick={() => setSnoozing(!snoozing)}>
          Snooze ▾
        </button>
        <button onClick={() => act(() => acknowledgeOccurrence(occurrenceId))}>Acknowledge</button>
        <button aria-expanded={skipping} onClick={() => setSkipping(!skipping)}>
          Skip…
        </button>
      </div>
      {snoozing && (
        <div class="alarm-row" role="group" aria-label="Snooze for">
          {SNOOZE_MINUTES.map((m) => (
            <button key={m} onClick={() => act(() => snoozeOccurrence(occurrenceId, m))}>
              {m} minutes
            </button>
          ))}
        </div>
      )}
      {snoozing && (
        <form
          class="alarm-row"
          onSubmit={(e) => {
            e.preventDefault();
            const t = nextTimeOfDay(until, Math.floor(Date.now() / 1000));
            if (t !== null) act(() => snoozeOccurrence(occurrenceId, undefined, t));
          }}
        >
          <label>
            Until…
            <input type="time" value={until} onInput={(e) => setUntil((e.target as HTMLInputElement).value)} />
          </label>
          <button type="submit" disabled={nextTimeOfDay(until, Math.floor(Date.now() / 1000)) === null}>
            Snooze until {until || "…"}
          </button>
        </form>
      )}
      {snoozing && <ExpiryNote expiresAt={view.expires_at} until={until} />}
      {skipping && (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            act(() => skipOccurrence(occurrenceId, note));
          }}
        >
          <label>
            Note (optional)
            <input value={note} onInput={(e) => setNote((e.target as HTMLInputElement).value)} />
          </label>
          <button type="submit">Skip this one</button>
        </form>
      )}
    </main>
  );
}

/** "Expires at 23:59" for what is ringing, and what the chosen time does about it. */
function ExpiryNote({ expiresAt, until }: { expiresAt: number | null; until: string }) {
  const now = Math.floor(Date.now() / 1000);
  const chosen = nextTimeOfDay(until, now);
  const warning = chosen === null ? null : expiryWarning(expiresAt, now, chosen);
  const note = expiryNote(expiresAt);
  if (!note) return null;
  return <p class="expiry">{warning ?? note}{chosen !== null && !warning ? ` (until ${clock(chosen)})` : ""}</p>;
}
