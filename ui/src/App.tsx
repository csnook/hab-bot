import { useEffect, useRef, useState } from "preact/hooks";
import {
  completeCountdown,
  completeOccurrence,
  deletedReminders,
  filters as loadFilters,
  lists as loadLists,
  setFilters as saveFilters,
  skipCountdown,
  type DeletedReminder,
  type DueItem,
  type Filters,
  type ListInfo,
  dismissNotice,
  inbox,
  type Inbox as InboxSections,
  onOpenOccurrence,
  onStateChanged,
  skipOccurrence,
  snapshot,
  type Snapshot,
} from "./api";
import { formatTime, signInNoticeText } from "./time";
import { setupState, type Setup } from "./api";
import { describeCountdown } from "./countdown";
import { ReminderEditor } from "./ReminderEditor";
import { Settings } from "./Settings";
import { SnoozeMenu } from "./SnoozeMenu";
import { FirstStart } from "./FirstStart";
import { Sidebar } from "./Sidebar";
import { filterInbox, filterSnapshot, hiddenCount, isFiltering, noFilters, pruned } from "./filters";
import { listById, listColour, listName } from "./lists";

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
  const [lists, setLists] = useState<ListInfo[]>([]);
  const [deleted, setDeleted] = useState<DeletedReminder[]>([]);
  // The sidebar's checkboxes, remembered on this device.
  const [filters, setFilters] = useState<Filters>(noFilters);
  // The occurrence a clicked notification asked for.
  const [opened, setOpened] = useState<string | null>(null);
  // The reminder editor: closed, a new reminder, or an existing one.
  const [editing, setEditing] = useState<{ reminderId: string | null } | null>(null);
  const edit = (reminderId: string) => setEditing({ reminderId });

  const refresh = () => {
    snapshot().then(setSnap).catch((e) => setError(String(e)));
    inbox().then(setSections).catch((e) => setError(String(e)));
    loadLists().then(setLists).catch((e) => setError(String(e)));
    deletedReminders().then(setDeleted).catch(() => {});
  };

  // The filters are read once: after that this window is what changes them.
  useEffect(() => {
    loadFilters().then(setFilters).catch(() => {});
  }, []);
  const changeFilters = (f: Filters) => {
    setFilters(f);
    saveFilters(f).catch((e) => setError(String(e)));
  };
  // A list that was deleted needn't stay in the remembered filters.
  useEffect(() => {
    if (lists.length === 0) return;
    const p = pruned(filters, lists);
    if (p !== filters) changeFilters(p);
  }, [lists]);

  const shown = filterInbox(filters, sections);
  const shownSnap = filterSnapshot(filters, snap);
  const hidden = hiddenCount(filters, sections);
  const dot = (listId: string) => {
    const l = listById(lists, listId);
    return l ? <ListDot list={l} lists={lists} /> : null;
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
    <div class="layout">
    <Sidebar
      lists={lists}
      filters={filters}
      onFilters={changeFilters}
      onChanged={refresh}
      onError={setError}
    />
    <main>
      <h1>Inbox</h1>
      <button type="button" onClick={() => setEditing({ reminderId: null })}>
        New reminder
      </button>
      {error && <p class="error" role="alert">{error}</p>}
      {isFiltering(filters) && (
        <p class="muted" role="status">
          Filtered by the sidebar
          {hidden > 0 ? `: ${hidden} open ${hidden === 1 ? "reminder is" : "reminders are"} hidden, and still alert` : ""}.
          <button type="button" onClick={() => changeFilters(noFilters())}>Show everything</button>
        </p>
      )}
      {snap.update_notice && <p class="notice" role="status">{snap.update_notice}</p>}

      <section aria-labelledby="overdue">
        <h2 id="overdue">Overdue</h2>
        {shown.overdue.length === 0 && <p class="empty">Nothing overdue.</p>}
        <ul>
          {shown.overdue.map((d) => (
            <OpenItem key={d.occurrence_id} item={d} done={done} skip={skip} onError={setError} opened={opened === d.occurrence_id} onEdit={edit} dot={dot(d.list_id)} />
          ))}
        </ul>
      </section>

      <section aria-labelledby="due">
        <h2 id="due">Due</h2>
        {shown.due.length === 0 && <p class="empty">Nothing due.</p>}
        <ul>
          {shown.due.map((d) => (
            <OpenItem key={d.occurrence_id} item={d} done={done} skip={skip} onError={setError} opened={opened === d.occurrence_id} onEdit={edit} dot={dot(d.list_id)} />
          ))}
        </ul>
      </section>

      <section aria-labelledby="later-today">
        <h2 id="later-today">Later today</h2>
        {shown.later_today.length === 0 && <p class="empty">Nothing more today.</p>}
        <ul>
          {shown.later_today.map((e) => (
            <li key={`${e.reminder_id}@${e.scheduled_at}`}>
              {dot(e.list_id)}
              <span class="title">{e.title}</span>
              {e.note && <span class="note">{e.note}</span>}
              <span class="when">{formatTime(e.scheduled_at)}</span>
              {e.snoozed_until !== null && (
                <span class="snoozed">snoozed until {formatTime(e.snoozed_until)}</span>
              )}
              <SnoozeMenu
                target={{ reminderId: e.reminder_id, scheduledAt: e.scheduled_at }}
                label="Snooze ahead"
                onError={setError}
              />
              <button onClick={() => edit(e.reminder_id)}>Edit</button>
            </li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="earlier-today">
        <h2 id="earlier-today">Earlier today</h2>
        {shown.earlier_today.length === 0 && <p class="empty">Nothing closed yet today.</p>}
        <ul>
          {shown.earlier_today.map((e) => (
            <li key={e.occurrence_id}>
              {dot(e.list_id)}
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
        {shownSnap.upcoming.length === 0 && <p class="empty">No reminders waiting.</p>}
        <ul>
          {shownSnap.upcoming.map((u) => (
            <li key={u.reminder_id}>
              {dot(u.list_id)}
              <span class="title">{u.title}</span>
              {u.note && <span class="note">{u.note}</span>}
              {u.not_sent && <NotSent />}
              <span class="when">{formatTime(u.fire_at)}</span>
              <button onClick={() => edit(u.reminder_id)}>Edit</button>
            </li>
          ))}
        </ul>
      </section>

      <section aria-labelledby="counting-down">
        <h2 id="counting-down">Counting down</h2>
        {shownSnap.countdowns.length === 0 && <p class="empty">No countdowns.</p>}
        <ul>
          {shownSnap.countdowns.map((c) => (
            <li key={c.reminder_id}>
              {dot(c.list_id)}
              <span class="title">{c.title}</span>
              <span class="priority">{describeCountdown(c.countdown)}</span>
              <button onClick={() => edit(c.reminder_id)}>Edit</button>
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

      {deleted.length > 0 && (
        <details class="deleted">
          <summary>Deleted ({deleted.length})</summary>
          <ul>
            {deleted.map((d) => (
              <li key={d.reminder_id}>
                {dot(d.list_id)}
                <span class="title">{d.title}</span>
                <span class="when">deleted {formatTime(d.deleted_at)}, history kept</span>
              </li>
            ))}
          </ul>
        </details>
      )}

      {editing && (
        <ReminderEditor reminderId={editing.reminderId} onClose={() => setEditing(null)} />
      )}
    </main>
    </div>
  );
}

/** A list's colour as a small dot, named for screen readers by its list. */
function ListDot({ list, lists }: { list: ListInfo; lists: ListInfo[] }) {
  return (
    <span
      class="dot"
      style={{ background: listColour(list, lists) }}
      role="img"
      aria-label={`List: ${listName(list)}`}
      title={listName(list)}
    />
  );
}

function OpenItem({
  item: d,
  done,
  skip,
  onError,
  opened,
  onEdit,
  dot,
}: {
  dot: preact.ComponentChild;
  item: DueItem;
  done: (id: string, doneAt?: number) => void;
  skip: (id: string) => void;
  onError: (message: string) => void;
  opened: boolean;
  onEdit: (reminderId: string) => void;
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
      {dot}
      <span class="title">{d.title}</span>
      {d.note && <span class="note">{d.note}</span>}
      <span class="priority">{d.priority}</span>
      {d.not_sent && <NotSent />}
      <span class="when">{formatTime(d.scheduled_at)}</span>
      {d.snoozed_until !== null && d.snoozed_until * 1000 > Date.now() && (
        <span class="snoozed">snoozed until {formatTime(d.snoozed_until)}</span>
      )}
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
      <SnoozeMenu target={{ occurrenceId: d.occurrence_id }} onError={onError} />
      <button onClick={() => skip(d.occurrence_id)}>Skip</button>
      <button onClick={() => onEdit(d.reminder_id)}>Edit</button>
    </li>
  );
}

/** A change the server hasn't numbered yet: still on this device only. */
function NotSent() {
  return <span class="not-sent">not sent yet</span>;
}
