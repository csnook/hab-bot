import { useEffect, useRef, useState } from "preact/hooks";
import {
  completeCountdown,
  completeOccurrence,
  createCountdownReminder,
  skipCountdown,
  type CountdownUnit,
  type DueItem,
  type PriorityName,
  createRecurringReminder,
  createReminder,
  dismissNotice,
  inbox,
  type Inbox as InboxSections,
  onOpenOccurrence,
  onStateChanged,
  skipOccurrence,
  snapshot,
  type Snapshot,
} from "./api";
import { formatTime, signInNoticeText, toUnixSeconds } from "./time";
import { setupState, type Setup } from "./api";
import {
  countdownFor,
  describeCountdown,
  hasTimeOfDay,
  lastDoneSeconds,
  type LastDone,
  UNITS,
} from "./countdown";
import { DAYS, patternFor, REPEATS, type Repeat } from "./repeat";
import { Settings } from "./Settings";
import { FirstStart } from "./FirstStart";

/** The first start asks how to use this device, then the app opens. */
export function App() {
  const [setup, setSetup] = useState<Setup | null | undefined>(undefined);
  const [settings, setSettings] = useState<"priorities" | "account" | "about" | null>(null);
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
        <button aria-pressed="true">Inbox</button>
        <button aria-label="Settings" title="Settings" onClick={() => setSettings("priorities")}>
          ⚙
        </button>
      </nav>
      {setup.mode === "joined" && <SignInNotices
          onRemove={(deviceId) => {
            setRemoving(deviceId);
            setSettings("account");
          }}
        />}
      {setup.mode === "joined" && <ReconciliationBanners />}
      <Inbox />
      {settings && (
        <Settings
          setup={setup}
          removing={removing}
          start={settings}
          onClose={() => setSettings(null)}
        />
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
    countdowns: [],
    sign_in_notices: [],
    reconciliations: [],
    security_notices: [],
    update_notice: null,
  });
  const [sections, setSections] = useState<InboxSections>({
    overdue: [],
    due: [],
    later_today: [],
    earlier_today: [],
  });
  const [error, setError] = useState("");
  // The occurrence a clicked notification asked for.
  const [opened, setOpened] = useState<string | null>(null);

  const refresh = () => {
    snapshot().then(setSnap).catch((e) => setError(String(e)));
    inbox().then(setSections).catch((e) => setError(String(e)));
  };

  useEffect(() => {
    refresh();
    const unlisten = onStateChanged(refresh);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  useEffect(() => {
    const unlisten = onOpenOccurrence(setOpened);
    return () => {
      unlisten.then((f) => f());
    };
  }, []);

  const done = (id: string, doneAt?: number) =>
    completeOccurrence(id, doneAt).catch((e) => setError(String(e)));
  const skip = (id: string) =>
    skipOccurrence(id).catch((e) => setError(String(e)));

  return (
    <main>
      <h1>Inbox</h1>
      {error && <p class="error" role="alert">{error}</p>}
      {snap.update_notice && <p class="notice" role="status">{snap.update_notice}</p>}

      <section aria-labelledby="overdue">
        <h2 id="overdue">Overdue</h2>
        {sections.overdue.length === 0 && <p class="empty">Nothing overdue.</p>}
        <ul>
          {sections.overdue.map((d) => (
            <OpenItem key={d.occurrence_id} item={d} done={done} skip={skip} opened={opened === d.occurrence_id} />
          ))}
        </ul>
      </section>

      <section aria-labelledby="due">
        <h2 id="due">Due</h2>
        {sections.due.length === 0 && <p class="empty">Nothing due.</p>}
        <ul>
          {sections.due.map((d) => (
            <OpenItem key={d.occurrence_id} item={d} done={done} skip={skip} opened={opened === d.occurrence_id} />
          ))}
        </ul>
      </section>

      <section aria-labelledby="later-today">
        <h2 id="later-today">Later today</h2>
        {sections.later_today.length === 0 && <p class="empty">Nothing more today.</p>}
        <ul>
          {sections.later_today.map((e) => (
            <li key={`${e.reminder_id}@${e.scheduled_at}`}>
              <span class="title">{e.title}</span>
              <span class="when">{formatTime(e.scheduled_at)}</span>
            </li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="earlier-today">
        <h2 id="earlier-today">Earlier today</h2>
        {sections.earlier_today.length === 0 && <p class="empty">Nothing closed yet today.</p>}
        <ul>
          {sections.earlier_today.map((e) => (
            <li key={e.occurrence_id}>
              <span class="title">{e.title}</span>
              <span class="when">
                {e.kind} {formatTime(e.closed_at)}
              </span>
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

      <section aria-labelledby="counting-down">
        <h2 id="counting-down">Counting down</h2>
        {snap.countdowns.length === 0 && <p class="empty">No countdowns.</p>}
        <ul>
          {snap.countdowns.map((c) => (
            <li key={c.reminder_id}>
              <span class="title">{c.title}</span>
              <span class="priority">{describeCountdown(c.countdown)}</span>
              <span class="when">
                {c.next_at === null ? "waiting for you" : formatTime(c.next_at)}
              </span>
              {c.next_at !== null && (
                <>
                  <button
                    onClick={() => completeCountdown(c.reminder_id).catch((e) => setError(String(e)))}
                  >
                    Done now
                  </button>
                  <button
                    onClick={() => skipCountdown(c.reminder_id).catch((e) => setError(String(e)))}
                  >
                    Skip
                  </button>
                </>
              )}
            </li>
          ))}
        </ul>
      </section>

      <NewReminder onError={setError} />
    </main>
  );
}

function OpenItem({
  item: d,
  done,
  skip,
  opened,
}: {
  item: DueItem;
  done: (id: string, doneAt?: number) => void;
  skip: (id: string) => void;
  opened: boolean;
}) {
  const row = useRef<HTMLLIElement>(null);
  // When it was really done, if earlier than now: a countdown restarts from it.
  const [doneAt, setDoneAt] = useState("");
  // A clicked notification brings its occurrence into view.
  useEffect(() => {
    if (opened) row.current?.scrollIntoView?.({ block: "center" });
  }, [opened]);
  return (
    <li ref={row} class={opened ? "opened" : undefined} aria-current={opened ? "true" : undefined}>
      <span class="title">{d.title}</span>
      <span class="priority">{d.priority}</span>
      {d.not_sent && <NotSent />}
      <span class="when">{formatTime(d.scheduled_at)}</span>
      <input
        type="datetime-local"
        aria-label="Done at (if earlier than now)"
        value={doneAt}
        onInput={(e) => setDoneAt(e.currentTarget.value)}
      />
      <button
        onClick={() => {
          const t = doneAt ? Math.floor(new Date(doneAt).getTime() / 1000) : undefined;
          done(d.occurrence_id, t !== undefined && !Number.isNaN(t) ? t : undefined);
        }}
      >
        Done
      </button>
      <button onClick={() => skip(d.occurrence_id)}>Skip</button>
    </li>
  );
}

/** A change the server hasn't numbered yet: still on this device only. */
function NotSent() {
  return <span class="not-sent">not sent yet</span>;
}

const PRIORITY_CHOICES: Array<[PriorityName, string]> = [
  ["minimum", "Minimum"],
  ["low", "Low"],
  ["medium", "Medium"],
  ["high", "High"],
  ["maximum", "Maximum"],
];

function NewReminder({ onError }: { onError: (e: string) => void }) {
  const [title, setTitle] = useState("");
  const [date, setDate] = useState("");
  const [time, setTime] = useState("");
  const [repeat, setRepeat] = useState<Repeat>("once");
  const [days, setDays] = useState<string[]>([]);
  // Floating (the same local time wherever the device is) unless pinned.
  const [pinned, setPinned] = useState(false);
  const [priority, setPriority] = useState<PriorityName>("medium");
  // A countdown: how long after it was last done, and when that was.
  const [amount, setAmount] = useState(3);
  const [unit, setUnit] = useState<CountdownUnit>("days");
  const [timeOfDay, setTimeOfDay] = useState("");
  const [lastDone, setLastDone] = useState<LastDone>("now");
  const [lastDoneAt, setLastDoneAt] = useState("");
  const counting = repeat === "countdown";

  const submit = async (e: Event) => {
    e.preventDefault();
    if (counting) {
      const countdown = countdownFor(amount, unit, timeOfDay);
      if (!countdown.ok) return onError(countdown.error);
      const done = lastDoneSeconds(lastDone, Math.floor(Date.now() / 1000), lastDoneAt);
      if (!done.ok) return onError(done.error);
      // Elapsed countdowns have no zone; a time of day is pinned or floating.
      const zone = pinned && countdown.value.at ? Intl.DateTimeFormat().resolvedOptions().timeZone : null;
      try {
        await createCountdownReminder(title, countdown.value, done.value, zone, priority);
        setTitle("");
        onError("");
      } catch (err) {
        onError(String(err));
      }
      return;
    }
    const fireAt = toUnixSeconds(date, time);
    if (fireAt === null) return onError("Pick a date and time.");
    try {
      if (repeat === "once") {
        await createReminder(title, fireAt, priority);
      } else {
        const pattern = patternFor(repeat, date, days);
        if (!pattern) return onError("Choose the days it repeats on.");
        const zone = pinned ? Intl.DateTimeFormat().resolvedOptions().timeZone : null;
        await createRecurringReminder(title, pattern, date, time, zone, priority);
      }
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
      {!counting && (
        <>
          <label>
            Date
            <input type="date" value={date} onInput={(e) => setDate(e.currentTarget.value)} required />
          </label>
          <label>
            Time
            <input type="time" value={time} onInput={(e) => setTime(e.currentTarget.value)} required />
          </label>
        </>
      )}
      <label>
        Priority
        <select
          value={priority}
          onChange={(e) => setPriority(e.currentTarget.value as PriorityName)}
        >
          {PRIORITY_CHOICES.map(([value, label]) => (
            <option value={value} key={value}>{label}</option>
          ))}
        </select>
      </label>
      <label>
        Repeat
        <select value={repeat} onChange={(e) => setRepeat(e.currentTarget.value as Repeat)}>
          {REPEATS.map(([value, label]) => (
            <option value={value} key={value}>{label}</option>
          ))}
        </select>
      </label>
      {repeat === "weekly" && (
        <fieldset>
          <legend>On</legend>
          {DAYS.map(([code, label]) => (
            <label key={code}>
              <input
                type="checkbox"
                checked={days.includes(code)}
                onChange={(e) =>
                  setDays(e.currentTarget.checked ? [...days, code] : days.filter((d) => d !== code))
                }
              />
              {label}
            </label>
          ))}
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
              value={amount}
              onInput={(e) => setAmount(Number(e.currentTarget.value))}
            />
          </label>
          <label>
            Unit
            <select value={unit} onChange={(e) => setUnit(e.currentTarget.value as CountdownUnit)}>
              {UNITS.map(([value, label]) => (
                <option value={value} key={value}>{label}</option>
              ))}
            </select>
          </label>
          {hasTimeOfDay(unit) && (
            <label>
              At this time of day (or leave empty to keep the time it was done)
              <input type="time" value={timeOfDay} onInput={(e) => setTimeOfDay(e.currentTarget.value)} />
            </label>
          )}
          <fieldset>
            <legend>When was this last done?</legend>
            {([["now", "Just now"], ["never", "Never (fire at once)"], ["at", "At…"]] as Array<[LastDone, string]>).map(
              ([value, label]) => (
                <label key={value}>
                  <input
                    type="radio"
                    name="last-done"
                    checked={lastDone === value}
                    onChange={() => setLastDone(value)}
                  />
                  {label}
                </label>
              ),
            )}
            {lastDone === "at" && (
              <input
                type="datetime-local"
                aria-label="Last done at"
                value={lastDoneAt}
                onInput={(e) => setLastDoneAt(e.currentTarget.value)}
              />
            )}
          </fieldset>
        </>
      )}
      {(counting ? hasTimeOfDay(unit) && timeOfDay !== "" : repeat !== "once") && (
        <label>
          <input
            type="checkbox"
            checked={pinned}
            onChange={(e) => setPinned(e.currentTarget.checked)}
          />
          Keep to this time zone when I travel
        </label>
      )}
      <button type="submit">Create</button>
    </form>
  );
}
