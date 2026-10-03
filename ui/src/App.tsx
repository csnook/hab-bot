import { useEffect, useState } from "preact/hooks";
import {
  completeOccurrence,
  createReminder,
  dismissNotice,
  onStateChanged,
  skipOccurrence,
  snapshot,
  type Snapshot,
} from "./api";
import { formatTime, signInNoticeText, toUnixSeconds } from "./time";
import { setupState, type Setup } from "./api";
import { Account } from "./Account";
import { FirstStart } from "./FirstStart";

/** The first start asks how to use this device, then the app opens. */
export function App() {
  const [setup, setSetup] = useState<Setup | null | undefined>(undefined);
  const [page, setPage] = useState<"inbox" | "settings">("inbox");
  // The device a "Not you? Remove it" notice was about, until it is dealt with.
  const [removing, setRemoving] = useState<string | null>(null);

  useEffect(() => {
    setupState().then(setSetup).catch(() => setSetup({ mode: "standalone" }));
  }, []);

  if (setup === undefined) return <main />;
  if (setup === null) return <FirstStart onDone={setSetup} />;
  return (
    <>
      <nav class="tabs" aria-label="Views">
        <button aria-pressed={page === "inbox"} onClick={() => setPage("inbox")}>Inbox</button>
        <button aria-pressed={page === "settings"} onClick={() => setPage("settings")}>Settings</button>
      </nav>
      {setup.mode === "joined" && <SignInNotices
          onRemove={(deviceId) => {
            setRemoving(deviceId);
            setPage("settings");
          }}
        />}
      {setup.mode === "joined" && <ReconciliationBanners />}
      {page === "inbox" ? (
        <Inbox />
      ) : (
        <main>
          <h1>Settings</h1>
          <Account setup={setup} removing={removing} />
        </main>
      )}
    </>
  );
}

/** "New device signed in: <name>, just now. Not you? Remove it", on every other device. */
function SignInNotices({ onRemove }: { onRemove: (deviceId: string) => void }) {
  const [notices, setNotices] = useState<Snapshot["sign_in_notices"]>([]);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  useEffect(() => {
    const refresh = () => {
      setNow(Math.floor(Date.now() / 1000));
      snapshot().then((s) => setNotices(s.sign_in_notices)).catch(() => {});
    };
    refresh();
    const unlisten = onStateChanged(refresh);
    const tick = setInterval(refresh, 30_000);
    return () => {
      clearInterval(tick);
      unlisten.then((f) => f());
    };
  }, []);

  return (
    <>
      {notices.map((n) => (
        <p class="notice banner" role="status" key={n.id}>
          {signInNoticeText(n.device_name, n.at, now)} Not you?{" "}
          <button type="button" onClick={() => onRemove(n.device_id)}>Remove it</button>
          <button type="button" onClick={() => dismissNotice(n.id).catch(() => {})}>Dismiss</button>
        </p>
      ))}
    </>
  );
}

/**
 * "Your phone skipped… It counts as completed.", when two devices disagreed,
 * and "5 failed sign-ins to your account", from the server.
 */
function ReconciliationBanners() {
  const [notices, setNotices] = useState<Array<{ id: string; text: string }>>([]);

  useEffect(() => {
    const refresh = () => {
      snapshot()
        .then((s) => setNotices([...s.security_notices, ...s.reconciliations]))
        .catch(() => {});
    };
    refresh();
    const unlisten = onStateChanged(refresh);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  return (
    <>
      {notices.map((n) => (
        <p class="notice banner" role="status" key={n.id}>
          {n.text}{" "}
          <button type="button" onClick={() => dismissNotice(n.id).catch(() => {})}>Dismiss</button>
        </p>
      ))}
    </>
  );
}

function Inbox() {
  const [snap, setSnap] = useState<Snapshot>({
    due: [],
    upcoming: [],
    sign_in_notices: [],
    reconciliations: [],
    security_notices: [],
    update_notice: null,
  });
  const [error, setError] = useState("");

  const refresh = () => snapshot().then(setSnap).catch((e) => setError(String(e)));

  useEffect(() => {
    refresh();
    const unlisten = onStateChanged(refresh);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const done = (id: string) =>
    completeOccurrence(id).catch((e) => setError(String(e)));
  const skip = (id: string) =>
    skipOccurrence(id).catch((e) => setError(String(e)));

  return (
    <main>
      <h1>Inbox</h1>
      {error && <p class="error" role="alert">{error}</p>}
      {snap.update_notice && <p class="notice" role="status">{snap.update_notice}</p>}

      <section aria-labelledby="due">
        <h2 id="due">Due</h2>
        {snap.due.length === 0 && <p class="empty">Nothing due.</p>}
        <ul>
          {snap.due.map((d) => (
            <li key={d.occurrence_id}>
              <span class="title">{d.title}</span>
              {d.not_sent && <NotSent />}
              <span class="when">{formatTime(d.scheduled_at)}</span>
              <button onClick={() => done(d.occurrence_id)}>Done</button>
              <button onClick={() => skip(d.occurrence_id)}>Skip</button>
            </li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="upcoming">
        <h2 id="upcoming">Later</h2>
        {snap.upcoming.length === 0 && <p class="empty">No reminders waiting.</p>}
        <ul>
          {snap.upcoming.map((u) => (
            <li key={u.reminder_id}>
              <span class="title">{u.title}</span>
              {u.not_sent && <NotSent />}
              <span class="when">{formatTime(u.fire_at)}</span>
            </li>
          ))}
        </ul>
      </section>

      <NewReminder onError={setError} />
    </main>
  );
}

/** A change the server hasn't numbered yet: still on this device only. */
function NotSent() {
  return <span class="not-sent">not sent yet</span>;
}

function NewReminder({ onError }: { onError: (e: string) => void }) {
  const [title, setTitle] = useState("");
  const [date, setDate] = useState("");
  const [time, setTime] = useState("");

  const submit = async (e: Event) => {
    e.preventDefault();
    const fireAt = toUnixSeconds(date, time);
    if (fireAt === null) return onError("Pick a date and time.");
    try {
      await createReminder(title, fireAt);
      setTitle("");
      onError("");
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <form onSubmit={submit} aria-label="New reminder">
      <h2>New reminder</h2>
      <label>
        Title
        <input value={title} onInput={(e) => setTitle(e.currentTarget.value)} required />
      </label>
      <label>
        Date
        <input type="date" value={date} onInput={(e) => setDate(e.currentTarget.value)} required />
      </label>
      <label>
        Time
        <input type="time" value={time} onInput={(e) => setTime(e.currentTarget.value)} required />
      </label>
      <button type="submit">Create</button>
    </form>
  );
}
