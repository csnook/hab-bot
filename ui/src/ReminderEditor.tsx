import { useEffect, useMemo, useRef, useState } from "preact/hooks";
import {
  createCountdownReminder,
  createRecurringReminder,
  createReminder,
  editReminder,
  lists as loadLists,
  moveReminder,
  pauseReminder,
  priorities,
  reminderView,
  resumeReminder,
  type ListInfo,
  type PriorityInfo,
  type PriorityName,
  type ReminderView,
} from "./api";
import { UNITS, hasTimeOfDay } from "./countdown";
import {
  DEFAULT_EXPIRY_TEXT,
  DURATION_UNITS,
  NEXT_REPEATS,
  defaultOverdueText,
  describeDelay,
  describeExpiries,
  emptyNext,
  priorityName,
  type DurationUnit,
  type NextForm,
  type NextRepeat,
} from "./delays";
import {
  buildEdit,
  buildNew,
  buildPause,
  listMove,
  newExpiry,
  newReminderList,
  newNextExpiry,
  newState,
  overdueSpec,
  expirySpecs,
  stateFromView,
  summaryInput,
  type EditorState,
  type ExpiryForm,
  type OverdueForm,
} from "./editor";
import { DeleteDialog } from "./DeleteDialog";
import { listById, listName as nameOf } from "./lists";
import { DAYS, REPEATS, type Repeat } from "./repeat";
import { activePause, nextWeekOf, pauseLine, tomorrowOf, type PauseForm } from "./pause";
import { expiryLine, noteLine, overdueLine, summarize } from "./summary";
import type { CountdownUnit } from "./api";
import type { LastDone } from "./countdown";

const PRIORITY_CHOICES: PriorityName[] = ["minimum", "low", "medium", "high", "maximum"];

const zoneName = () => Intl.DateTimeFormat().resolvedOptions().timeZone;

/**
 * The reminder editor, for a new reminder or (with `reminderId`) an existing
 * one: the live sentence, the always-visible basics, and the folded Overdue,
 * Expiry, Note and (for an existing reminder) Pause sections. Turns, Only if
 * (conditions) and the other triggers arrive with their own tickets.
 *
 * Type-checked only: the editor has never been run in a real window.
 */
export function ReminderEditor({
  reminderId,
  onClose,
}: {
  reminderId: string | null;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [state, setState] = useState<EditorState>(newState);
  const [view, setView] = useState<ReminderView | null>(null);
  const [infos, setInfos] = useState<PriorityInfo[]>([]);
  const [allLists, setAllLists] = useState<ListInfo[]>([]);
  const [deleting, setDeleting] = useState(false);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const d = ref.current;
    if (d && !d.open) {
      try {
        d.showModal();
      } catch {
        d.setAttribute("open", "");
      }
    }
    priorities().then(setInfos).catch(() => {});
    loadLists()
      .then((l) => {
        setAllLists(l);
        // A new reminder starts in the personal list, the default.
        setState((s) => (s.listId ? s : { ...s, listId: l[0]?.id ?? "" }));
      })
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (!reminderId) return;
    reminderView(reminderId)
      .then((v) => {
        setView(v);
        setState(stateFromView(v));
      })
      .catch((e) => setError(String(e)));
  }, [reminderId]);

  const set = (patch: Partial<EditorState>) => setState((s) => ({ ...s, ...patch }));
  const setWhen = (patch: Partial<EditorState>) =>
    setState((s) => ({ ...s, ...patch, whenTouched: true }));

  const editing = reminderId !== null;
  const chosen = listById(allLists, state.listId);
  const listName = chosen ? nameOf(chosen) : view ? (view.list_name ?? "Personal") : "Personal";
  const sentence = useMemo(() => summarize(summaryInput(state, listName, zoneName())), [state, listName]);

  const submit = async (e: Event) => {
    e.preventDefault();
    setError("");
    setBusy(true);
    try {
      if (editing) {
        if (!view) return;
        const edit = buildEdit(state, view, zoneName());
        if (!edit.ok) return setError(edit.error);
        const pause = buildPause(state, view, Math.floor(Date.now() / 1000));
        if (!pause.ok) return setError(pause.error);
        if (Object.keys(edit.value).length) await editReminder(view.reminder_id, edit.value);
        // Pausing skips what falls in the period, so it is an action of its own.
        if (pause.value.kind === "pause") {
          await pauseReminder(view.reminder_id, pause.value.until);
        } else if (pause.value.kind === "resume") {
          await resumeReminder(view.reminder_id);
        }
        // Moving to another list keeps its history.
        const to = listMove(state, view);
        if (to) await moveReminder(view.reminder_id, to);
      } else {
        const made = buildNew(state, Math.floor(Date.now() / 1000), zoneName());
        if (!made.ok) return setError(made.error);
        const { plan, priority, extras } = made.value;
        const listId = newReminderList(state);
        let id: string;
        if (plan.kind === "once") {
          id = await createReminder(plan.title, plan.fireAt, priority, listId);
        } else if (plan.kind === "schedule") {
          id = await createRecurringReminder(
            plan.title,
            plan.pattern,
            plan.date,
            plan.time,
            plan.zone,
            priority,
            listId,
          );
        } else {
          id = await createCountdownReminder(
            plan.title,
            plan.countdown,
            plan.lastDone,
            plan.zone,
            priority,
            listId,
          );
        }
        if (extras) await editReminder(id, extras);
      }
      onClose();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const counting = state.repeat === "countdown";
  const kind = view?.trigger.kind ?? null;
  const defaultText = defaultOverdueText(state.priority, infos);
  const overdue = overdueSpec(state.overdue);
  const expiries = expirySpecs(state.expiries);
  const overdueSummary = overdueLine(overdue.ok ? overdue.value : null, defaultText);
  const expirySummary = expiryLine(expiries.ok ? expiries.value : [], DEFAULT_EXPIRY_TEXT);
  const noteSummary = noteLine(state.note);
  const nowSeconds = Math.floor(Date.now() / 1000);
  const pauseSummary = pauseLine(view?.pause ?? null, view?.list_pause ?? null, nowSeconds);

  return (
    <dialog ref={ref} class="editor" aria-labelledby="editor-title" onClose={onClose}>
      <form onSubmit={submit} aria-label={editing ? "Edit reminder" : "New reminder"}>
        <h1 id="editor-title">{editing ? "Edit reminder" : "New reminder"}</h1>
        <p class="sentence" role="status" aria-live="polite">{sentence}</p>
        {error && <p class="error" role="alert">{error}</p>}

        <label>
          Title
          <input
            value={state.title}
            onInput={(e) => set({ title: e.currentTarget.value })}
            required
          />
        </label>
        <div class="row">
          <label>
            List
            <select
              aria-label="List"
              value={state.listId}
              disabled={allLists.length < 2}
              onChange={(e) => set({ listId: e.currentTarget.value })}
            >
              {allLists.length === 0 && <option>{listName}</option>}
              {allLists.map((l) => (
                <option value={l.id} key={l.id}>{nameOf(l)}</option>
              ))}
            </select>
          </label>
          <label>
            Priority
            <select
              value={state.priority}
              onChange={(e) => set({ priority: e.currentTarget.value as PriorityName })}
            >
              {PRIORITY_CHOICES.map((p) => (
                <option value={p} key={p}>{priorityName(p)}</option>
              ))}
            </select>
          </label>
        </div>

        <fieldset class="when">
          <legend>When</legend>
          {state.whenLocked && <p class="muted">{state.whenLocked}</p>}
          {!state.whenLocked && (
            <>
              {kind !== "countdown" && (
                <div class="row">
                  <label>
                    {state.repeat === "once" ? "Date" : "Starting"}
                    <input
                      type="date"
                      value={state.date}
                      onInput={(e) => setWhen({ date: e.currentTarget.value })}
                      required
                    />
                  </label>
                  <label>
                    Time
                    <input
                      type="time"
                      value={state.time}
                      onInput={(e) => setWhen({ time: e.currentTarget.value })}
                      required
                    />
                  </label>
                </div>
              )}
              {(!editing || kind === "schedules") && (
                <label>
                  Repeat
                  <select
                    value={state.repeat}
                    onChange={(e) => setWhen({ repeat: e.currentTarget.value as Repeat })}
                  >
                    {REPEATS.filter(([v]) => !editing || (v !== "once" && v !== "countdown")).map(
                      ([value, label]) => (
                        <option value={value} key={value}>{label}</option>
                      ),
                    )}
                  </select>
                </label>
              )}
              {state.repeat === "weekly" && (
                <fieldset>
                  <legend>On</legend>
                  <DayBoxes
                    days={state.days}
                    onChange={(days) => setWhen({ days })}
                  />
                </fieldset>
              )}
              {counting && (
                <>
                  <label>
                    Fires after
                    <input
                      type="number"
                      min="1"
                      step="1"
                      value={state.amount}
                      onInput={(e) => setWhen({ amount: Number(e.currentTarget.value) })}
                    />
                  </label>
                  <label>
                    Unit
                    <select
                      value={state.unit}
                      onChange={(e) => setWhen({ unit: e.currentTarget.value as CountdownUnit })}
                    >
                      {UNITS.map(([value, label]) => (
                        <option value={value} key={value}>{label}</option>
                      ))}
                    </select>
                  </label>
                  {hasTimeOfDay(state.unit) && (
                    <label>
                      At this time of day (or leave empty to keep the time it was done)
                      <input
                        type="time"
                        value={state.timeOfDay}
                        onInput={(e) => setWhen({ timeOfDay: e.currentTarget.value })}
                      />
                    </label>
                  )}
                  {!editing && (
                    <fieldset>
                      <legend>When was this last done?</legend>
                      {(
                        [
                          ["now", "Just now"],
                          ["never", "Never (fire at once)"],
                          ["at", "At…"],
                        ] as Array<[LastDone, string]>
                      ).map(([value, label]) => (
                        <label key={value} class="radio">
                          <input
                            type="radio"
                            name="last-done"
                            checked={state.lastDone === value}
                            onChange={() => set({ lastDone: value })}
                          />
                          {label}
                        </label>
                      ))}
                      {state.lastDone === "at" && (
                        <input
                          type="datetime-local"
                          aria-label="Last done at"
                          value={state.lastDoneAt}
                          onInput={(e) => set({ lastDoneAt: e.currentTarget.value })}
                        />
                      )}
                    </fieldset>
                  )}
                </>
              )}
              {(counting
                ? hasTimeOfDay(state.unit) && state.timeOfDay !== ""
                : state.repeat !== "once") && (
                <label class="radio">
                  <input
                    type="checkbox"
                    checked={state.pinned}
                    onChange={(e) => set({ pinned: e.currentTarget.checked })}
                  />
                  Keep to this time zone when I travel
                </label>
              )}
            </>
          )}
        </fieldset>

        <fieldset class="only-if">
          <legend>Only if</legend>
          <p class="muted">
            No conditions yet: it fires whenever it comes due. Conditions such as being at home
            arrive in a later version.
          </p>
        </fieldset>

        <details class="fold" aria-label="Overdue">
          <summary>
            Overdue{" "}
            <span class={overdueSummary.isDefault ? "one-line grey" : "one-line"}>
              {overdueSummary.text}
            </span>
          </summary>
          <OverdueSection
            value={state.overdue}
            defaultText={defaultText}
            onChange={(overdue) => set({ overdue })}
          />
        </details>

        <details class="fold" aria-label="Expiry">
          <summary>
            Expiry{" "}
            <span class={expirySummary.isDefault ? "one-line grey" : "one-line"}>
              {expirySummary.text}
            </span>
          </summary>
          <ExpirySection
            value={state.expiries}
            onChange={(expiries) => set({ expiries })}
          />
        </details>

        <details class="fold" aria-label="Note">
          <summary>
            Note{" "}
            <span class={noteSummary.isDefault ? "one-line grey" : "one-line"}>
              {noteSummary.text}
            </span>
          </summary>
          <label>
            Shown on every occurrence of this reminder
            <textarea
              rows={3}
              value={state.note}
              onInput={(e) => set({ note: e.currentTarget.value })}
            />
          </label>
        </details>

        {editing && view && (
          <details class="fold" aria-label="Pause">
            <summary>
              Pause{" "}
              <span class={pauseSummary.isDefault ? "one-line grey" : "one-line"}>
                {pauseSummary.text}
              </span>
            </summary>
            <PauseSection
              value={state.pause}
              listPaused={activePause(view.list_pause, nowSeconds) !== null}
              onChange={(pause) => set({ pause })}
            />
          </details>
        )}

        <div class="buttons">
          <button type="button" onClick={onClose}>Cancel</button>
          {editing && view && (
            <button type="button" class="danger" onClick={() => setDeleting(true)}>
              Delete…
            </button>
          )}
          <button type="submit" disabled={busy || (editing && !view)}>
            {editing ? "Save" : "Create"}
          </button>
        </div>
      </form>
      {deleting && view && (
        <DeleteDialog
          reminderId={view.reminder_id}
          title={view.title}
          onCancel={() => setDeleting(false)}
          onDeleted={onClose}
        />
      )}
    </dialog>
  );
}

/** Pause: not paused, until a day, or until resumed. Turning it off resumes early. */
function PauseSection({
  value,
  listPaused,
  onChange,
}: {
  value: PauseForm;
  listPaused: boolean;
  onChange: (v: PauseForm) => void;
}) {
  const now = Math.floor(Date.now() / 1000);
  return (
    <div class="section">
      <label class="radio">
        <input
          type="radio"
          name="pause"
          checked={value.mode === "off"}
          onChange={() => onChange({ ...value, mode: "off" })}
        />
        Not paused
      </label>
      <label class="radio">
        <input
          type="radio"
          name="pause"
          checked={value.mode === "until"}
          onChange={() => onChange({ ...value, mode: "until", date: value.date || nextWeekOf(now) })}
        />
        Until
        {value.mode === "until" && (
          <input
            type="date"
            aria-label="Paused until"
            min={tomorrowOf(now)}
            value={value.date}
            onInput={(e) => onChange({ ...value, mode: "until", date: e.currentTarget.value })}
          />
        )}
      </label>
      <label class="radio">
        <input
          type="radio"
          name="pause"
          checked={value.mode === "resumed"}
          onChange={() => onChange({ ...value, mode: "resumed" })}
        />
        Until I resume it
      </label>
      {listPaused && <p class="muted">Its list is paused too: resume the list from the sidebar.</p>}
      <p class="muted">
        Occurrences in the pause are skipped, and the history says the pause did it. Resuming early
        brings back normal firing from its next one.
      </p>
    </div>
  );
}

function DayBoxes({
  days,
  onChange,
}: {
  days: string[];
  onChange: (days: string[]) => void;
}) {
  return (
    <>
      {DAYS.map(([code, label]) => (
        <label key={code} class="radio">
          <input
            type="checkbox"
            checked={days.includes(code)}
            onChange={(e) =>
              onChange(
                e.currentTarget.checked ? [...days, code] : days.filter((d) => d !== code),
              )
            }
          />
          {label}
        </label>
      ))}
    </>
  );
}

/** Overdue: follow the priority (greyed, with its source), or override it. */
function OverdueSection({
  value,
  defaultText,
  onChange,
}: {
  value: OverdueForm;
  defaultText: string;
  onChange: (v: OverdueForm) => void;
}) {
  return (
    <div class="section">
      <label class="radio">
        <input
          type="radio"
          name="overdue"
          checked={value.mode === "default"}
          onChange={() => onChange({ mode: "default" })}
        />
        <span class="grey">{defaultText}</span>
        <span class="muted">follows the priority</span>
      </label>
      <label class="radio">
        <input
          type="radio"
          name="overdue"
          checked={value.mode === "after"}
          onChange={() =>
            onChange(value.mode === "after" ? value : { mode: "after", amount: 1, unit: "hours" })
          }
        />
        After
        {value.mode === "after" && (
          <Duration
            amount={value.amount}
            unit={value.unit}
            label="Overdue after"
            onChange={(amount, unit) => onChange({ mode: "after", amount, unit })}
          />
        )}
      </label>
      <label class="radio">
        <input
          type="radio"
          name="overdue"
          checked={value.mode === "next"}
          onChange={() =>
            onChange(
              value.mode === "next" ? value : { mode: "next", next: emptyNext() },
            )
          }
        />
        The next time…
      </label>
      {value.mode === "next" && (
        <NextEditor
          label="Overdue"
          value={value.next}
          onChange={(next) => onChange({ mode: "next", next })}
        />
      )}
      {value.mode === "keep" && (
        <p class="muted">{describeDelay(value.spec)} (a rule written by hand)</p>
      )}
      <p class="muted">
        Counted from the scheduled time. Until you override it, it keeps following the priority.
      </p>
    </div>
  );
}

/** Expiry: the default (greyed) plus any delays or schedules added. */
function ExpirySection({
  value,
  onChange,
}: {
  value: ExpiryForm[];
  onChange: (v: ExpiryForm[]) => void;
}) {
  const update = (i: number, e: ExpiryForm) => onChange(value.map((x, j) => (j === i ? e : x)));
  const specs = expirySpecs(value);
  return (
    <div class="section">
      <p class="grey">{DEFAULT_EXPIRY_TEXT}</p>
      {value.map((e, i) => (
        <div class="row expiry-row" key={i}>
          {e.kind === "keep" ? (
            <span>{describeDelay(e.spec)} (a rule written by hand)</span>
          ) : (
            <>
              <select
                aria-label={`Expiry ${i + 1} kind`}
                value={e.kind}
                onChange={(ev) =>
                  update(i, ev.currentTarget.value === "after" ? newExpiry() : newNextExpiry())
                }
              >
                <option value="after">After</option>
                <option value="next">The next time…</option>
              </select>
              {e.kind === "after" ? (
                <Duration
                  amount={e.amount}
                  unit={e.unit}
                  label={`Expiry ${i + 1}`}
                  onChange={(amount, unit) => update(i, { kind: "after", amount, unit })}
                />
              ) : (
                <NextEditor
                  label={`Expiry ${i + 1}`}
                  value={e.next}
                  onChange={(next) => update(i, { kind: "next", next })}
                />
              )}
            </>
          )}
          <button type="button" onClick={() => onChange(value.filter((_, j) => j !== i))}>
            Remove
          </button>
        </div>
      ))}
      <button type="button" onClick={() => onChange([...value, newExpiry()])}>
        Add expiry
      </button>
      {specs.ok && specs.value.length > 1 && (
        <p class="muted">Whichever comes first marks it missed: {describeExpiries(specs.value)}.</p>
      )}
      <p class="muted">
        Counted from the scheduled time. Expiry delays add no expected occurrences.
      </p>
    </div>
  );
}

function Duration({
  amount,
  unit,
  label,
  onChange,
}: {
  amount: number;
  unit: DurationUnit;
  label: string;
  onChange: (amount: number, unit: DurationUnit) => void;
}) {
  return (
    <span class="duration">
      <input
        type="number"
        min="1"
        step="1"
        aria-label={`${label}, amount`}
        value={amount}
        onInput={(e) => onChange(Number(e.currentTarget.value), unit)}
      />
      <select
        aria-label={`${label}, unit`}
        value={unit}
        onChange={(e) => onChange(amount, e.currentTarget.value as DurationUnit)}
      >
        {DURATION_UNITS.map(([v, l]) => (
          <option value={v} key={v}>{l}</option>
        ))}
      </select>
    </span>
  );
}

/** "the next [day choice] at [time]". */
function NextEditor({
  label,
  value,
  onChange,
}: {
  label: string;
  value: NextForm;
  onChange: (v: NextForm) => void;
}) {
  const set = (patch: Partial<NextForm>) => onChange({ ...value, ...patch });
  return (
    <span class="next">
      <select
        aria-label={`${label}, which days`}
        value={value.repeat}
        onChange={(e) => set({ repeat: e.currentTarget.value as NextRepeat })}
      >
        {NEXT_REPEATS.map(([v, l]) => (
          <option value={v} key={v}>{l}</option>
        ))}
      </select>
      {value.repeat === "weekly" && (
        <DayBoxes days={value.days} onChange={(days) => set({ days })} />
      )}
      {value.repeat === "monthly_date" && (
        <select
          aria-label={`${label}, day of the month`}
          value={String(value.day)}
          onChange={(e) => set({ day: Number(e.currentTarget.value) })}
        >
          {Array.from({ length: 31 }, (_, i) => i + 1).map((d) => (
            <option value={String(d)} key={d}>{d}</option>
          ))}
          <option value="-1">last</option>
        </select>
      )}
      {value.repeat === "monthly_weekday" && (
        <>
          <select
            aria-label={`${label}, which one`}
            value={String(value.ordinal)}
            onChange={(e) => set({ ordinal: Number(e.currentTarget.value) })}
          >
            <option value="1">1st</option>
            <option value="2">2nd</option>
            <option value="3">3rd</option>
            <option value="4">4th</option>
            <option value="-1">last</option>
          </select>
          <select
            aria-label={`${label}, weekday`}
            value={value.weekday}
            onChange={(e) => set({ weekday: e.currentTarget.value })}
          >
            {DAYS.map(([c, l]) => (
              <option value={c} key={c}>{l}</option>
            ))}
          </select>
        </>
      )}
      <input
        type="time"
        aria-label={`${label}, time of day`}
        value={value.time}
        onInput={(e) => set({ time: e.currentTarget.value })}
      />
    </span>
  );
}
