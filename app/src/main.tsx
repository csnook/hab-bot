import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import * as api from "./api";
import { DAYS, DAY_NAMES, ORDINAL_NAMES, rule, wall, type Choice, type Pattern } from "./schedule";
import { BottomNav, rememberedView, rememberView, Swipeable, TopBar, UndoToast, usePhone, VIEWS, type UndoAction, type View } from "./phone";
import "./style.css";

function localInputValue(ms: number): string {
  const d = new Date(ms - new Date(ms).getTimezoneOffset() * 60_000);
  return d.toISOString().slice(0, 16);
}

const WHY: Record<string, string> = {
  notifications: "Notifications: so a reminder can alert you when it's due.",
  alarms: "Alarms & reminders: so a reminder fires exactly on time, even when the phone is idle.",
};

/** Says why before Android asks. Refusing doesn't block anything. */
async function askForPermissions() {
  const missing = await api.missingPermissions();
  if (missing.length === 0) return;
  const ok = window.confirm(`Reminders need your permission:\n\n${missing.map((m) => WHY[m] ?? m).join("\n")}`);
  if (ok) await api.requestPermissions();
}

const PRIORITIES: api.Priority[] = ["minimum", "low", "medium", "high", "maximum"];

const dayLabel = (i: number) => DAY_NAMES[i];

/** Snooze: one tap uses the priority's interval; the menu lists the rest, and shows a known expiry. */
function SnoozeButton({ id, onDone, ahead }: { id: string; onDone: () => void; ahead?: boolean }) {
  const [picker, setPicker] = useState<api.SnoozePicker | null>(null);
  const [custom, setCustom] = useState("");
  const hhmm = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const until = (ms: number) => api.snoozeUntil(id, ms).then(() => { setPicker(null); onDone(); });
  return (
    <span class="snooze">
      <button onClick={() => api.snooze(id).then(onDone)}>{ahead ? "Snooze ahead" : "Snooze"}</button>
      <button title="More snooze choices" onClick={() => (picker ? setPicker(null) : api.snoozePicker(id).then(setPicker))}>▾</button>
      {picker && (
        <span class="menu">
          {picker.expires_at && <em>expires at {hhmm(picker.expires_at)}</em>}
          {picker.options.map((o) => (
            <button onClick={() => until(o.until)}>{o.label} ({hhmm(o.until)})</button>
          ))}
          <input type="datetime-local" value={custom} onInput={(e) => setCustom(e.currentTarget.value)} />
          <button disabled={!custom} onClick={() => until(new Date(custom).getTime())}>Until…</button>
        </span>
      )}
    </span>
  );
}

/** The details panel, with the buttons that fit what the occurrence is: open, expected or closed. */
function Details({ o, sheet, onClose, refresh }: { o: api.Occurrence; sheet?: boolean; onClose: () => void; refresh: () => void }) {
  const [more, setMore] = useState(false);
  const [when, setWhen] = useState(localInputValue(Date.now()));
  const [note, setNote] = useState("");
  const [notes, setNotes] = useState<string[]>([]);
  const [error, setError] = useState("");
  useEffect(() => void api.recentSkipNotes().then(setNotes), []);
  const act = (f: () => Promise<unknown>) => f().then(() => { refresh(); onClose(); }, (e) => setError(String(e)));
  const at = () => new Date(when).getTime();
  const n = note.trim() || null;
  const closed = o.status === "completed" || o.status === "skipped" || o.status === "missed";
  const time = (ms: number | null) => (ms ? new Date(ms).toLocaleString() : "");

  return (
    <dialog open class={sheet ? "details sheet" : "details"}>
      <h2>{o.title}</h2>
      <p>
        {o.priority} · scheduled {time(o.scheduled_at)}
        {o.status === "expected" && " · expected"}
        {o.closed_at && ` · ${o.status} ${time(o.closed_at)}`}
        {o.tapped_at && o.tapped_at !== o.closed_at && ` (tapped ${time(o.tapped_at)})`}
        {o.corrected_from && ` · corrected from ${o.corrected_from}`}
        {o.note && ` · "${o.note}"`}
      </p>
      {(o.status === "due") && (
        <p>
          <button onClick={() => act(() => api.complete(o.id))}>Done</button>
          <SnoozeButton id={o.id} onDone={() => { refresh(); onClose(); }} />
          <button onClick={() => act(() => api.skip(o.id, n))}>Skip</button>
          <button onClick={() => setMore(!more)}>More ▾</button>
        </p>
      )}
      {o.status === "due" && more && (
        <p>
          Done at a different time: <input type="datetime-local" value={when} onInput={(e) => setWhen(e.currentTarget.value)} />
          <button onClick={() => act(() => api.completeAt(o.id, at()))}>Done at this time</button>
          <button disabled title="Comes with the editor">Edit reminder</button>
        </p>
      )}
      {o.status === "expected" && (
        <p>
          <button onClick={() => act(() => api.completeEarly(o.reminder_id))}>Complete early</button>
          <button onClick={() => act(() => api.skipAhead(o.id, n))}>Skip ahead</button>
          <SnoozeButton id={o.id} ahead onDone={() => { refresh(); onClose(); }} />
        </p>
      )}
      {(o.status === "due" || o.status === "expected") && (
        <p>
          Skip note: <input list="notes" value={note} onInput={(e) => setNote(e.currentTarget.value)} />
          <datalist id="notes">{notes.map((x) => <option value={x} />)}</datalist>
        </p>
      )}
      {closed && (
        <p>
          {o.status !== "missed" && <button onClick={() => act(() => api.undo(o.id))}>Undo</button>}
          <input type="datetime-local" value={when} onInput={(e) => setWhen(e.currentTarget.value)} />
          <input placeholder="note" value={note} onInput={(e) => setNote(e.currentTarget.value)} />
          <button onClick={() => act(() => api.correct(o.id, "completed", at(), n))}>Correct to done</button>
          <button onClick={() => act(() => api.correct(o.id, "skipped", at(), n))}>Correct to skipped</button>
        </p>
      )}
      {error && <p class="error">{error}</p>}
      <button onClick={onClose}>Close</button>
    </dialog>
  );
}

function NewReminder({ onCreated }: { onCreated: () => void }) {
  const [title, setTitle] = useState("");
  const [when, setWhen] = useState(localInputValue(Date.now() + 60_000));
  // further times of day, for "several times a day": each is its own schedule trigger
  const [extra, setExtra] = useState<string[]>([]);
  const [choice, setChoice] = useState<Choice>({ pattern: "once", days: [0], ordinal: 0, weekday: 0, custom: "", amount: 3, unit: "days", timeOfDay: "", lastDone: localInputValue(Date.now()) });
  const [priority, setPriority] = useState<api.Priority>("medium");
  const [pin, setPin] = useState(false);
  const [error, setError] = useState("");
  const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const set = (c: Partial<Choice>) => setChoice({ ...choice, ...c });

  const submit = async (e: Event) => {
    e.preventDefault();
    try {
      const first = new Date(when);
      const r = rule(choice, first);
      const triggers: api.Trigger[] =
        choice.pattern === "countdown"
          ? [
              {
                kind: "countdown",
                unit: choice.unit,
                amount: choice.amount,
                at: choice.unit === "days" && choice.timeOfDay ? choice.timeOfDay : null,
                last_done: choice.lastDone ? new Date(choice.lastDone).getTime() : null,
              },
            ]
          : r === null
          ? [{ kind: "one_off", at: first.getTime() }]
          : [when, ...extra].map((w) => ({ kind: "schedule", rule: r, start: wall(new Date(w)) }));
      await api.createReminder({ title, triggers, priority, expiry: null, tz: pin && (r !== null || choice.pattern === "countdown") ? zone : null });
      setTitle("");
      setExtra([]);
      setError("");
      onCreated();
      await askForPermissions();
    } catch (err) {
      setError(String(err));
    }
  };

  const repeating = choice.pattern !== "once" && choice.pattern !== "countdown";
  return (
    <form onSubmit={submit}>
      <input placeholder="Remind me to…" value={title} onInput={(e) => setTitle(e.currentTarget.value)} />
      <select value={priority} onChange={(e) => setPriority(e.currentTarget.value as api.Priority)} title="Priority">
        {PRIORITIES.map((p) => <option value={p}>{p[0].toUpperCase() + p.slice(1)}</option>)}
      </select>
      <select value={choice.pattern} onChange={(e) => set({ pattern: e.currentTarget.value as Pattern })}>
        <option value="once">Once</option>
        <option value="daily">Every day</option>
        <option value="weekdays">Weekdays</option>
        <option value="weekly">Weekly on…</option>
        <option value="monthly_date">Monthly by date</option>
        <option value="monthly_weekday">Monthly by weekday</option>
        <option value="custom">Custom (RRULE)</option>
        <option value="countdown">After the last time…</option>
      </select>
      {choice.pattern === "weekly" && (
        <span class="days">
          {DAYS.map((_, i) => (
            <label key={i}>
              <input
                type="checkbox"
                checked={choice.days.includes(i)}
                onChange={() => set({ days: choice.days.includes(i) ? choice.days.filter((d) => d !== i) : [...choice.days, i] })}
              />
              {dayLabel(i)}
            </label>
          ))}
        </span>
      )}
      {choice.pattern === "monthly_weekday" && (
        <span>
          <select value={choice.ordinal} onChange={(e) => set({ ordinal: Number(e.currentTarget.value) })}>
            {ORDINAL_NAMES.map((n, i) => <option value={i}>{n}</option>)}
          </select>
          <select value={choice.weekday} onChange={(e) => set({ weekday: Number(e.currentTarget.value) })}>
            {DAY_NAMES.map((n, i) => <option value={i}>{n}</option>)}
          </select>
        </span>
      )}
      {choice.pattern === "custom" && (
        <input placeholder="FREQ=MONTHLY;BYMONTHDAY=1,15" value={choice.custom} onInput={(e) => set({ custom: e.currentTarget.value })} />
      )}
      {choice.pattern === "countdown" && (
        <span>
          <input type="number" min="1" value={choice.amount} onInput={(e) => set({ amount: Number(e.currentTarget.value) })} />
          <select value={choice.unit} onChange={(e) => set({ unit: e.currentTarget.value as "hours" | "days" })}>
            <option value="hours">hours</option>
            <option value="days">days</option>
          </select>
          {choice.unit === "days" && (
            <label>
              at <input type="time" value={choice.timeOfDay} onInput={(e) => set({ timeOfDay: e.currentTarget.value })} />
            </label>
          )}
          <label>
            Last done{" "}
            <input
              type="datetime-local"
              value={choice.lastDone}
              disabled={choice.lastDone === ""}
              onInput={(e) => set({ lastDone: e.currentTarget.value })}
            />
          </label>
          <label>
            <input
              type="checkbox"
              checked={choice.lastDone === ""}
              onChange={() => set({ lastDone: choice.lastDone === "" ? localInputValue(Date.now()) : "" })}
            />
            never
          </label>
        </span>
      )}
      {choice.pattern !== "countdown" && (
        <input type="datetime-local" value={when} onInput={(e) => setWhen(e.currentTarget.value)} />
      )}
      {repeating &&
        extra.map((w, i) => (
          <input
            type="datetime-local"
            value={w}
            onInput={(e) => setExtra(extra.map((x, j) => (j === i ? e.currentTarget.value : x)))}
          />
        ))}
      {repeating && (
        <button type="button" onClick={() => setExtra([...extra, when])}>
          Add another time
        </button>
      )}
      {choice.pattern !== "once" && (
        <label>
          <input type="checkbox" checked={pin} onChange={() => setPin(!pin)} />
          Keep to {zone}
        </label>
      )}
      <button type="submit">Add</button>
      {error && <p class="error">{error}</p>}
    </form>
  );
}

const span = (ms: number) =>
  ms === 0 ? "at once" : ms % 86_400_000 === 0 ? `${ms / 86_400_000} day` : ms % 3_600_000 === 0 ? `${ms / 3_600_000} h` : `${ms / 60_000} min`;

/** Settings opens as one dialog. Priorities are the built-ins, read-only; About shows the version. */
function Settings({ onClose }: { onClose: () => void }) {
  const [list, setList] = useState<api.PrioritySettings[]>([]);
  const [version, setVersion] = useState("");
  useEffect(() => {
    api.priorities().then(setList);
    api.about().then(setVersion);
  }, []);
  return (
    <dialog open>
      <h2>Settings</h2>
      <h3>Priorities</h3>
      <table>
        <thead>
          <tr><th></th><th>Due style</th><th>Due interval</th><th>Once overdue</th><th>Overdue interval</th><th>Swipe</th><th>Breaks DND</th></tr>
        </thead>
        <tbody>
          {list.map((p) => (
            <tr key={p.priority}>
              <th>{p.name}</th>
              <td>{p.due_style}</td>
              <td>{span(p.due_interval)}</td>
              <td>{p.overdue.map((s) => (s.after ? `${s.style} after ${span(s.after)}` : s.style)).join(", then ")}</td>
              <td>{span(p.overdue_interval)}</td>
              <td>{p.swipeable ? "yes" : "no"}</td>
              <td>{p.breaks_do_not_disturb ? "yes" : "no"}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <h3>About</h3>
      <p>Version {version}</p>
      <button onClick={onClose}>Close</button>
    </dialog>
  );
}

/** "4 older quiet reminders ▸": Minimum and Low occurrences overdue for over a week. */
function FoldedRow({ items, refresh, select }: { items: api.Occurrence[]; refresh: () => void; select: (o: api.Occurrence) => void }) {
  const [open, setOpen] = useState(false);
  const skipAll = () => {
    if (window.confirm(`Skip ${items.length} older quiet reminder${items.length === 1 ? "" : "s"}?`)) {
      api.skipOlderQuiet().then(refresh);
    }
  };
  return (
    <section class="folded">
      <p>
        <button class="link" onClick={() => setOpen(!open)}>
          {items.length} older quiet reminder{items.length === 1 ? "" : "s"} {open ? "▾" : "▸"}
        </button>
        <button onClick={skipAll}>Skip all…</button>
      </p>
      {open && (
        <ul>
          {items.map((o) => (
            <li key={o.id} class="overdue">
              <span class="title" onClick={() => select(o)}>
                {o.title} <small>{o.priority}</small>
              </span>
              <time>since {new Date(o.overdue_at).toLocaleDateString()}</time>
              <button onClick={() => api.complete(o.id).then(refresh)}>Done</button>
              <SnoozeButton id={o.id} onDone={refresh} />
              <button onClick={() => api.skip(o.id).then(refresh)}>Skip</button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function Inbox() {
  const phone = usePhone();
  const [items, setItems] = useState<api.InboxItem[]>([]);
  const [settings, setSettings] = useState(false);
  const [opened, setOpened] = useState("");
  const [selected, setSelected] = useState<api.Occurrence | null>(null);
  const [adding, setAdding] = useState(!phone);
  const [undo, setUndo] = useState<UndoAction | null>(null);
  const [view, setView] = useState<View>(() => rememberedView(VIEWS, "inbox"));
  const refresh = () => api.inbox().then(setItems);

  useEffect(() => {
    refresh();
    const unlisten = api.onChanged(refresh);
    const unlistenOpen = api.onOpen(setOpened);
    return () => {
      void unlisten.then((f) => f());
      void unlistenOpen.then((f) => f());
    };
  }, []);
  useEffect(() => rememberView(view), [view]);

  const by = (section: api.InboxItem["section"]) => items.filter((i) => i.section === section);
  const hhmm = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

  // Swipe right for Done and left to snooze for the priority's interval, then 5 seconds of Undo.
  const swipeDone = (o: api.Occurrence) =>
    api.complete(o.id).then(() => { refresh(); setUndo({ label: `Done: ${o.title}`, undo: () => api.undo(o.id).then(refresh) }); });
  const swipeSnooze = (o: api.Occurrence) =>
    api.snooze(o.id).then(() => { refresh(); setUndo({ label: `Snoozed: ${o.title}`, undo: () => api.unsnooze(o.id).then(refresh) }); });

  /** An open occurrence. On a phone it swipes, and has just Done (the sheet has the rest). */
  const openRow = (o: api.Occurrence, overdue: boolean) => (
    <Swipeable
      key={o.id}
      enabled={phone}
      class={`${overdue ? "overdue" : ""} ${o.id === opened ? "opened" : ""}`}
      onRight={() => swipeDone(o)}
      onLeft={() => swipeSnooze(o)}
    >
      <span class="title" onClick={() => setSelected(o)}>
        {o.title} {overdue && <small>{o.priority}</small>}
      </span>
      <time>{overdue ? `since ${new Date(o.overdue_at).toLocaleString()}` : new Date(o.scheduled_at).toLocaleString()}</time>
      <button onClick={() => api.complete(o.id).then(refresh)}>Done</button>
      {!phone && <SnoozeButton id={o.id} onDone={refresh} />}
      {!phone && <button onClick={() => api.skip(o.id).then(refresh)}>Skip</button>}
    </Swipeable>
  );

  const body = (
    <>
      {settings && <Settings onClose={() => setSettings(false)} />}
      {selected && <Details o={selected} sheet={phone} onClose={() => setSelected(null)} refresh={refresh} />}
      {adding && <NewReminder onCreated={() => { refresh(); if (phone) setAdding(false); }} />}
      {by("overdue").length > 0 && (
        <section>
          <h2>Overdue</h2>
          <ul>{by("overdue").map(({ occurrence: o }) => openRow(o, true))}</ul>
        </section>
      )}
      {by("overdue_folded").length > 0 && (
        <FoldedRow items={by("overdue_folded").map((i) => i.occurrence)} refresh={refresh} select={setSelected} />
      )}
      <section>
        <h2>Due</h2>
        {by("due").length === 0 && <p class="empty">Nothing due.</p>}
        <ul>{by("due").map(({ occurrence: o }) => openRow(o, false))}</ul>
      </section>
      {by("later_today").length > 0 && (
        <section>
          <h2>Later today</h2>
          <ul>
            {by("later_today").map(({ occurrence: o }) => (
              <li key={o.id} class="expected">
                <span class="title" onClick={() => setSelected(o)}>{o.title}</span>
                <time>{hhmm(o.scheduled_at)}</time>
                {!phone && <button onClick={() => api.completeEarly(o.reminder_id).then(refresh)}>Done early</button>}
                {!phone && <SnoozeButton id={o.id} onDone={refresh} />}
              </li>
            ))}
          </ul>
        </section>
      )}
      {by("earlier_today").length > 0 && (
        <section>
          <h2>Earlier today</h2>
          <ul>
            {by("earlier_today").map(({ occurrence: o }) => (
              <li key={o.id} class={o.status}>
                <span class="title" onClick={() => setSelected(o)}>{o.title}</span>
                <time>
                  {o.status === "missed" ? "Missed" : o.status === "skipped" ? "Skipped" : "Done"} {hhmm(o.closed_at ?? o.scheduled_at)}
                </time>
              </li>
            ))}
          </ul>
        </section>
      )}
    </>
  );

  if (!phone) {
    return (
      <main>
        <h1>
          Inbox <button class="link" onClick={() => setSettings(true)}>Settings</button>
        </h1>
        {body}
      </main>
    );
  }
  return (
    <div class="phone">
      <TopBar title="Inbox" onFilter={() => { /* the filter sheet comes with Agenda and Board */ }} onSettings={() => setSettings(true)} />
      <main>{body}</main>
      {!selected && <UndoToast action={undo} onGone={() => setUndo(null)} />}
      <button class="fab" aria-label="New reminder" onClick={() => setAdding(!adding)}>+</button>
      <BottomNav view={view} onView={setView} />
    </div>
  );
}

/** The alarm window: "Ringing · list · priority", the title, when it was due, and the buttons. */
function AlarmScreen({ id }: { id: string }) {
  const [o, setO] = useState<api.Occurrence | null | undefined>(undefined);
  const [skipping, setSkipping] = useState(false);
  const [note, setNote] = useState("");
  const [options, setOptions] = useState(false);
  const [custom, setCustom] = useState("");
  useEffect(() => void api.getOccurrence(id).then(setO), []);
  // after an action the alarm is over: take down the window and sound
  const act = (f: () => Promise<unknown>) => f().then(() => api.closeAlarm());
  if (o === undefined) return null;
  if (o === null) return <main class="alarm"><p>This alarm is over.</p><button onClick={() => api.closeAlarm()}>Close</button></main>;
  const minutes = (m: number) => api.snoozeUntil(id, Date.now() + m * 60_000);
  return (
    <main class="alarm">
      <p class="ringing">Ringing · Personal · {o.priority}</p>
      <h1>{o.title}</h1>
      <p>Due {new Date(o.scheduled_at).toLocaleString()}</p>
      <button class="big" onClick={() => act(() => api.complete(id))}>Done</button>
      <p>
        <button onClick={() => setOptions(!options)}>Snooze ▾</button>
        <button onClick={() => act(() => api.acknowledge(id))}>Acknowledge</button>
        <button onClick={() => setSkipping(!skipping)}>Skip…</button>
      </p>
      {options && (
        <p>
          {[5, 10, 30].map((m) => <button onClick={() => act(() => minutes(m))}>{m} minutes</button>)}
          <input type="datetime-local" value={custom} onInput={(e) => setCustom(e.currentTarget.value)} />
          <button disabled={!custom} onClick={() => act(() => api.snoozeUntil(id, new Date(custom).getTime()))}>Until…</button>
        </p>
      )}
      {skipping && (
        <p>
          <input placeholder="Note (optional)" value={note} onInput={(e) => setNote(e.currentTarget.value)} />
          <button onClick={() => act(() => api.skip(id, note.trim() || null))}>Skip</button>
        </p>
      )}
    </main>
  );
}

const alarmId = new URLSearchParams(window.location.search).get("alarm");
render(alarmId ? <AlarmScreen id={alarmId} /> : <Inbox />, document.getElementById("app")!);
