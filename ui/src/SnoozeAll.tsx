import { useEffect, useRef, useState } from "preact/hooks";
import { endSnoozeAll, snoozeAll, type ListInfo, type SnoozeAllView } from "./api";
import { listName } from "./lists";
import {
  LENGTHS,
  SNOOZE_ALL_NOTE,
  chipText,
  chips,
  lengthEnd,
  scopeOfValue,
  untilLabel,
  valueOfScope,
  type Length,
} from "./quiet";

const nowSeconds = () => Math.floor(Date.now() / 1000);

/**
 * "All snoozed until 19:00 · End now": one chip for each snooze-all holding.
 * Ending one early alerts everything it held back at its current level.
 */
export function SnoozeAllChips({
  holding,
  onError,
}: {
  holding: SnoozeAllView[];
  onError: (message: string) => void;
}) {
  const now = nowSeconds();
  return (
    <>
      {chips(holding, now).map((h) => (
        <p class="chip" role="status" key={h.id}>
          {chipText(h, now)} ·{" "}
          <button type="button" onClick={() => h.id && endSnoozeAll(h.id).catch((e) => onError(String(e)))}>
            End now
          </button>
        </p>
      ))}
    </>
  );
}

/**
 * Snooze all…: everything or one list, until a length, a time of day or
 * tomorrow morning, with Maximum left out unless included. It notes what
 * snoozing all does.
 */
export function SnoozeAllDialog({
  lists,
  onClose,
  onError,
}: {
  lists: ListInfo[];
  onClose: () => void;
  onError: (message: string) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [scope, setScope] = useState("");
  const [choice, setChoice] = useState(1);
  const [time, setTime] = useState("");
  const [includeMax, setIncludeMax] = useState(false);
  const [error, setError] = useState("");

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

  const OWN = LENGTHS.length;
  const length: Length = choice === OWN ? { kind: "time", time } : LENGTHS[choice][1];
  const until = lengthEnd(length, nowSeconds());

  const submit = (e: Event) => {
    e.preventDefault();
    if (until === null) return setError("Choose how long, or a time such as 19:00.");
    const s = scopeOfValue(scope);
    snoozeAll(s.kind === "list" ? s.list_id : null, until, includeMax)
      .then(onClose)
      .catch((err) => {
        setError(String(err));
        onError(String(err));
      });
  };

  return (
    <dialog ref={ref} class="snooze-all" aria-labelledby="snooze-all-title" onClose={onClose}>
      <form onSubmit={submit}>
        <h1 id="snooze-all-title">Snooze all</h1>
        <p class="muted">{SNOOZE_ALL_NOTE}</p>
        <label>
          Snooze
          <select value={scope} onChange={(e) => setScope(e.currentTarget.value)}>
            <option value={valueOfScope({ kind: "all" })}>Everything</option>
            {lists.map((l) => (
              <option key={l.id} value={l.id}>
                {listName(l)}
              </option>
            ))}
          </select>
        </label>
        <fieldset>
          <legend>Until</legend>
          {LENGTHS.map(([label], i) => (
            <label class="radio" key={label}>
              <input type="radio" name="snooze-all-length" checked={choice === i} onChange={() => setChoice(i)} />
              {label}
            </label>
          ))}
          <label class="radio">
            <input type="radio" name="snooze-all-length" checked={choice === OWN} onChange={() => setChoice(OWN)} />
            A time
            <input
              type="time"
              aria-label="Snooze all until a time"
              value={time}
              onFocus={() => setChoice(OWN)}
              onInput={(e) => {
                setTime(e.currentTarget.value);
                setChoice(OWN);
              }}
            />
          </label>
        </fieldset>
        <label class="check">
          <input type="checkbox" checked={includeMax} onChange={(e) => setIncludeMax(e.currentTarget.checked)} />
          Include maximum
        </label>
        <p class="muted" role="status">
          {until === null ? "" : `Alerts resume ${untilLabel(until, nowSeconds())}.`}
          {includeMax ? " Maximum priority is quieted too." : " Maximum priority still alerts."}
        </p>
        {error && <p role="alert">{error}</p>}
        <div class="row">
          <button type="submit" disabled={until === null}>Snooze all</button>
          <button type="button" onClick={onClose}>Cancel</button>
        </div>
      </form>
    </dialog>
  );
}
