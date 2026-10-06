import { useState } from "preact/hooks";
import {
  snoozeExpected,
  snoozeOccurrence,
  snoozePicker,
  snoozePickerExpected,
  type SnoozePicker,
} from "./api";
import {
  clock,
  expiryNote,
  expiryWarning,
  futureDateTime,
  nextTimeOfDay,
  optionLabel,
} from "./snooze";
import { formatTime } from "./time";

/** What the menu snoozes: an open occurrence, or an expected one ("snooze ahead"). */
export type SnoozeTarget =
  | { occurrenceId: string }
  | { reminderId: string; scheduledAt: number };

const nowSeconds = () => Math.floor(Date.now() / 1000);

/**
 * The Snooze ▾ menu: the priority's interval, 1 hour, until a time, tomorrow
 * morning and pick a time, with a note of any expiry and its last-chance
 * alert. A snooze quiets only your own alerts; the occurrence stays open.
 */
export function SnoozeMenu({
  target,
  label,
  onError,
}: {
  target: SnoozeTarget;
  label?: string;
  onError: (message: string) => void;
}) {
  const [picker, setPicker] = useState<SnoozePicker | null>(null);
  const [time, setTime] = useState("");
  const [picked, setPicked] = useState("");

  const ahead = "reminderId" in target;
  const load = () =>
    (ahead ? snoozePickerExpected(target.reminderId, target.scheduledAt) : snoozePicker(target.occurrenceId))
      .then(setPicker)
      .catch((e) => onError(String(e)));
  const snooze = (until: number) => {
    const done = ahead
      ? snoozeExpected(target.reminderId, target.scheduledAt, until)
      : snoozeOccurrence(target.occurrenceId, undefined, until);
    done.then(() => setPicker(null)).catch((e) => onError(String(e)));
  };

  if (!picker) {
    return (
      <button aria-haspopup="true" aria-expanded="false" onClick={load}>
        {label ?? "Snooze"} ▾
      </button>
    );
  }
  const now = nowSeconds();
  const untilTime = nextTimeOfDay(time, now);
  const untilPicked = futureDateTime(picked, now);
  const note = expiryNote(picker.expires_at);
  return (
    <div class="snooze-menu" role="group" aria-label={label ?? "Snooze"}>
      <button aria-expanded="true" onClick={() => setPicker(null)}>
        {label ?? "Snooze"} ▴
      </button>
      {ahead && (
        <p class="muted">
          It still fires at {formatTime(picker.from)}, quietly, and alerts when the snooze ends.
        </p>
      )}
      {picker.options.map((o) => (
        <button key={o.kind} onClick={() => snooze(o.until)}>
          {optionLabel(o)} <span class="muted">(until {clock(o.until)})</span>
          {o.last_chance && " ⚠"}
        </button>
      ))}
      <label>
        Until a time
        <input type="time" value={time} onInput={(e) => setTime(e.currentTarget.value)} />
      </label>
      <button disabled={untilTime === null} onClick={() => untilTime !== null && snooze(untilTime)}>
        Snooze until {untilTime !== null ? clock(untilTime) : "…"}
      </button>
      {untilTime !== null && <Warning picker={picker} until={untilTime} now={now} />}
      <label>
        Pick a date and time
        <input type="datetime-local" value={picked} onInput={(e) => setPicked(e.currentTarget.value)} />
      </label>
      <button disabled={untilPicked === null} onClick={() => untilPicked !== null && snooze(untilPicked)}>
        Snooze until {untilPicked !== null ? formatTime(untilPicked) : "…"}
      </button>
      {untilPicked !== null && <Warning picker={picker} until={untilPicked} now={now} />}
      {note && <p class="expiry">{note}</p>}
    </div>
  );
}

function Warning({ picker, until, now }: { picker: SnoozePicker; until: number; now: number }) {
  const w = expiryWarning(picker.expires_at, now, until);
  return w ? <p class="expiry" role="status">{w}</p> : null;
}
