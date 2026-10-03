import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import * as api from "./api";
import { DAYS, DAY_NAMES, ORDINAL_NAMES, rule, wall, type Choice, type Pattern } from "./schedule";
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
      await api.createReminder({ title, triggers, priority, tz: pin && (r !== null || choice.pattern === "countdown") ? zone : null });
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

function Inbox() {
  const [items, setItems] = useState<api.InboxItem[]>([]);
  const [settings, setSettings] = useState(false);
  const [opened, setOpened] = useState("");
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

  const by = (section: api.InboxItem["section"]) => items.filter((i) => i.section === section);
  const hhmm = (ms: number) => new Date(ms).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  return (
    <main>
      <h1>
        Inbox <button class="link" onClick={() => setSettings(true)}>Settings</button>
      </h1>
      {settings && <Settings onClose={() => setSettings(false)} />}
      <NewReminder onCreated={refresh} />
      {by("overdue").length > 0 && (
        <section>
          <h2>Overdue</h2>
          <ul>
            {by("overdue").map(({ occurrence: o }) => (
              <li key={o.id} class={`overdue ${o.id === opened ? "opened" : ""}`}>
                <span>
                  {o.title} <small>{o.priority}</small>
                </span>
                <time>since {new Date(o.overdue_at).toLocaleString()}</time>
                <button onClick={() => api.complete(o.id).then(refresh)}>Done</button>
                <button onClick={() => api.skip(o.id).then(refresh)}>Skip</button>
              </li>
            ))}
          </ul>
        </section>
      )}
      <section>
        <h2>Due</h2>
        {by("due").length === 0 && <p class="empty">Nothing due.</p>}
        <ul>
          {by("due").map(({ occurrence: o }) => (
            <li key={o.id} class={o.id === opened ? "opened" : ""}>
              <span>{o.title}</span>
              <time>{new Date(o.scheduled_at).toLocaleString()}</time>
              <button onClick={() => api.complete(o.id).then(refresh)}>Done</button>
              <button onClick={() => api.skip(o.id).then(refresh)}>Skip</button>
            </li>
          ))}
        </ul>
      </section>
      {by("later_today").length > 0 && (
        <section>
          <h2>Later today</h2>
          <ul>
            {by("later_today").map(({ occurrence: o }) => (
              <li key={o.id} class="expected">
                <span>{o.title}</span>
                <time>{hhmm(o.scheduled_at)}</time>
                <button onClick={() => api.completeEarly(o.reminder_id).then(refresh)}>Done early</button>
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
                <span>{o.title}</span>
                <time>
                  {o.status === "missed" ? "Missed" : o.status === "skipped" ? "Skipped" : "Done"} {hhmm(o.closed_at ?? o.scheduled_at)}
                </time>
              </li>
            ))}
          </ul>
        </section>
      )}
    </main>
  );
}

render(<Inbox />, document.getElementById("app")!);
