import { render } from "preact";
import { useEffect, useState } from "preact/hooks";
import * as api from "./api";
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

function NewReminder({ onCreated }: { onCreated: () => void }) {
  const [title, setTitle] = useState("");
  const [when, setWhen] = useState(localInputValue(Date.now() + 60_000));
  const [error, setError] = useState("");

  const submit = async (e: Event) => {
    e.preventDefault();
    try {
      await api.createOneOff(title, new Date(when).getTime());
      setTitle("");
      setError("");
      onCreated();
      await askForPermissions();
    } catch (err) {
      setError(String(err));
    }
  };

  return (
    <form onSubmit={submit}>
      <input placeholder="Remind me to…" value={title} onInput={(e) => setTitle(e.currentTarget.value)} />
      <input type="datetime-local" value={when} onInput={(e) => setWhen(e.currentTarget.value)} />
      <button type="submit">Add</button>
      {error && <p class="error">{error}</p>}
    </form>
  );
}

function Inbox() {
  const [items, setItems] = useState<api.InboxItem[]>([]);
  const refresh = () => api.inbox().then(setItems);

  useEffect(() => {
    refresh();
    const unlisten = api.onChanged(refresh);
    return () => void unlisten.then((f) => f());
  }, []);

  const due = items.filter((i) => i.section === "due");
  return (
    <main>
      <h1>Inbox</h1>
      <NewReminder onCreated={refresh} />
      <section>
        <h2>Due</h2>
        {due.length === 0 && <p class="empty">Nothing due.</p>}
        <ul>
          {due.map(({ occurrence: o }) => (
            <li key={o.id}>
              <span>{o.title}</span>
              <time>{new Date(o.scheduled_at).toLocaleString()}</time>
              <button onClick={() => api.complete(o.id).then(refresh)}>Done</button>
            </li>
          ))}
        </ul>
      </section>
    </main>
  );
}

render(<Inbox />, document.getElementById("app")!);
