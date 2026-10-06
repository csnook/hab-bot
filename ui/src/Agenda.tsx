import { useEffect, useState } from "preact/hooks";
import type { ComponentChild } from "preact";
import {
  completeEarly,
  completeOccurrence,
  skipAhead,
  skipOccurrence,
  undoOccurrence,
  type Agenda as AgendaData,
  type DueItem,
  type Filters,
} from "./api";
import { buildAgenda, closedWord, dayHeading, emptyDay, rowKey, type AgendaRow, type NowRow } from "./agenda";
import { SnoozeMenu } from "./SnoozeMenu";
import { untilText } from "./pause";
import { formatTime } from "./time";
import type { PanelTarget } from "./OccurrencePanel";

const clock = (t: number) =>
  new Date(t * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
const dayDate = (t: number) =>
  new Date(t * 1000).toLocaleDateString([], { weekday: "long", month: "short", day: "numeric" });

/**
 * The Agenda: the "Now" band of open occurrences, then one list from
 * yesterday to tomorrow with a rule marking now. A row expands in place to
 * show its details and buttons; "Details…" opens the shared panel.
 *
 * Its logic is in agenda.ts (unit-tested). This component is type-checked
 * only: it has never been run in a real window.
 */
export function Agenda({
  data,
  filters,
  dot,
  onDetails,
  onEdit,
  onError,
}: {
  data: AgendaData;
  filters: Filters;
  dot: (listId: string) => ComponentChild;
  onDetails: (t: PanelTarget) => void;
  onEdit: (reminderId: string) => void;
  onError: (message: string) => void;
}) {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  // One row open at a time.
  const [expanded, setExpanded] = useState<string | null>(null);
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(t);
  }, []);
  const view = buildAgenda(filters, data, now);
  const toggle = (key: string) => setExpanded((k) => (k === key ? null : key));
  const fail = (p: Promise<unknown>) => p.catch((e) => onError(String(e)));

  return (
    <div class="agenda">
      <section aria-labelledby="agenda-now" class="now-band">
        <h2 id="agenda-now">Now</h2>
        {view.now.length === 0 && <p class="empty">Nothing needs you right now.</p>}
        <ul>
          {view.now.map((r) => {
            const key = rowKey(r);
            const open = expanded === key;
            return (
              <li key={key} class={`agenda-row ${r.kind}`}>
                <RowHead
                  open={open}
                  onToggle={() => toggle(key)}
                  dot={dot(r.item.list_id)}
                  title={r.item.title}
                  when={nowWhen(r, now)}
                  tag={r.kind === "overdue" ? "overdue" : r.kind === "paused" ? "paused" : ""}
                />
                {open && (
                  <div class="agenda-detail">
                    {r.item.note && <p class="note">{r.item.note}</p>}
                    <p class="muted">
                      {r.item.priority} priority, scheduled {formatTime(r.item.scheduled_at)}
                      {r.item.snoozed_until !== null && r.item.snoozed_until > now
                        ? `, snoozed until ${formatTime(r.item.snoozed_until)}`
                        : ""}
                    </p>
                    <OpenButtons item={r.item} fail={fail} onError={onError} />
                    <button onClick={() => onEdit(r.item.reminder_id)}>Edit</button>
                    <button onClick={() => onDetails({ state: "open", item: r.item })}>Details…</button>
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </section>

      {view.days.map((day) => (
        <section key={day.name} aria-labelledby={`agenda-${day.name}`}>
          <h2 id={`agenda-${day.name}`}>
            {dayHeading(day.name)} <span class="muted">{dayDate(day.start)}</span>
          </h2>
          {day.rows.length === 0 && day.ruleBefore === null && <p class="empty">{emptyDay(day.name)}</p>}
          <ul>
            {day.rows.map((row, i) => (
              <>
                {day.ruleBefore === i && <NowRule now={now} />}
                <li key={row.key} class={`agenda-row ${row.kind}`}>
                  <RowHead
                    open={expanded === row.key}
                    onToggle={() => toggle(row.key)}
                    dot={dot(row.item.list_id)}
                    title={row.item.title}
                    when={clock(row.at)}
                    tag={row.kind === "closed" ? closedWord(row.item) : ""}
                  />
                  {expanded === row.key && (
                    <RowDetail row={row} fail={fail} onError={onError} onEdit={onEdit} onDetails={onDetails} />
                  )}
                </li>
              </>
            ))}
            {day.ruleBefore !== null && day.ruleBefore >= day.rows.length && <NowRule now={now} />}
          </ul>
        </section>
      ))}
    </div>
  );
}

function nowWhen(r: NowRow, now: number): string {
  if (r.kind === "paused") {
    return `${r.pause.list ? "its list is paused " : "paused "}${untilText(r.pause.until, now)}`;
  }
  return clock(r.item.scheduled_at);
}

function NowRule({ now }: { now: number }) {
  return (
    <li class="now-rule" role="separator" aria-label="Now">
      <span>Now {clock(now)}</span>
    </li>
  );
}

function RowHead({
  open,
  onToggle,
  dot,
  title,
  when,
  tag,
}: {
  open: boolean;
  onToggle: () => void;
  dot: ComponentChild;
  title: string;
  when: string;
  tag: string;
}) {
  return (
    <button type="button" class="agenda-head" aria-expanded={open} onClick={onToggle}>
      {dot}
      <span class="when">{when}</span>
      <span class="title">{title}</span>
      {tag && <span class="tag">{tag}</span>}
    </button>
  );
}

function OpenButtons({
  item,
  fail,
  onError,
}: {
  item: DueItem;
  fail: (p: Promise<unknown>) => void;
  onError: (m: string) => void;
}) {
  return (
    <>
      <button onClick={() => fail(completeOccurrence(item.occurrence_id))}>Done</button>
      <SnoozeMenu target={{ occurrenceId: item.occurrence_id }} onError={onError} />
      <button onClick={() => fail(skipOccurrence(item.occurrence_id))}>Skip</button>
    </>
  );
}

function RowDetail({
  row,
  fail,
  onError,
  onEdit,
  onDetails,
}: {
  row: AgendaRow;
  fail: (p: Promise<unknown>) => void;
  onError: (m: string) => void;
  onEdit: (reminderId: string) => void;
  onDetails: (t: PanelTarget) => void;
}) {
  if (row.kind === "expected") {
    const e = row.item;
    return (
      <div class="agenda-detail">
        {e.note && <p class="note">{e.note}</p>}
        <p class="muted">
          Expected {formatTime(e.scheduled_at)}, {e.priority} priority
          {e.snoozed_until !== null ? `, snoozed until ${formatTime(e.snoozed_until)}` : ""}
        </p>
        {e.can_close_early && (
          <>
            <button onClick={() => fail(completeEarly(e.reminder_id))}>Complete early</button>
            <button onClick={() => fail(skipAhead(e.reminder_id))}>Skip ahead</button>
          </>
        )}
        <SnoozeMenu
          target={{ reminderId: e.reminder_id, scheduledAt: e.scheduled_at }}
          label="Snooze ahead"
          onError={onError}
        />
        <button onClick={() => onEdit(e.reminder_id)}>Edit</button>
        <button onClick={() => onDetails({ state: "expected", item: e })}>Details…</button>
      </div>
    );
  }
  const c = row.item;
  return (
    <div class="agenda-detail">
      <p class="muted">
        {closedWord(c)} {formatTime(c.closed_at)}
        {c.corrected ? ", corrected" : ""}, scheduled {formatTime(c.scheduled_at)}
      </p>
      {c.can_undo && <button onClick={() => fail(undoOccurrence(c.occurrence_id))}>Undo</button>}
      <button onClick={() => onDetails({ state: "closed", item: c })}>Details…</button>
    </div>
  );
}
