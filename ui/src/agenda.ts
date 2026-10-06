import type { Agenda, DueItem, EarlierItem, ExpectedItem, Filters, PausedOpen } from "./api";
import { shows } from "./filters";

/**
 * The Agenda view's logic: a "Now" band of open occurrences, then one list
 * from yesterday to tomorrow in time order with a rule marking now.
 *
 * The core hands over the day boundaries (in the device's zone), so a day
 * that is 23 or 25 hours long is grouped correctly. The sidebar's filters
 * (filters.ts) apply here exactly as they do in the Inbox; they are applied
 * to the unfiltered data, never in the core.
 */

export type NowRow =
  | { kind: "overdue"; item: DueItem }
  | { kind: "due"; item: DueItem }
  | { kind: "paused"; item: DueItem; pause: PausedOpen["pause"] };

export type AgendaRow =
  | { kind: "expected"; at: number; key: string; item: ExpectedItem }
  | { kind: "closed"; at: number; key: string; item: EarlierItem };

export type DayName = "yesterday" | "today" | "tomorrow";

export interface AgendaDay {
  name: DayName;
  start: number;
  end: number;
  rows: AgendaRow[];
  /** The rule marking now goes before this row (rows.length: after them all), only in today. */
  ruleBefore: number | null;
}

export interface AgendaView {
  now: NowRow[];
  days: AgendaDay[];
}

const NAMES: DayName[] = ["yesterday", "today", "tomorrow"];

export function dayHeading(name: DayName): string {
  return name === "yesterday" ? "Yesterday" : name === "today" ? "Today" : "Tomorrow";
}

/** What an empty day says. */
export function emptyDay(name: DayName): string {
  return name === "yesterday"
    ? "Nothing happened yesterday."
    : name === "today"
      ? "Nothing else today."
      : "Nothing expected tomorrow.";
}

export function buildAgenda(filters: Filters, data: Agenda, now: number): AgendaView {
  const nowRows: NowRow[] = [
    ...data.overdue.filter((d) => shows(filters, d)).map((item): NowRow => ({ kind: "overdue", item })),
    ...data.due.filter((d) => shows(filters, d)).map((item): NowRow => ({ kind: "due", item })),
    ...data.paused
      .filter((p) => shows(filters, p.item))
      .map((p): NowRow => ({ kind: "paused", item: p.item, pause: p.pause })),
  ];

  const rows: AgendaRow[] = [
    ...data.closed
      .filter((e) => shows(filters, e))
      .map((item): AgendaRow => ({
        kind: "closed",
        at: item.scheduled_at,
        key: `c:${item.occurrence_id}`,
        item,
      })),
    ...data.expected
      .filter((e) => shows(filters, e))
      .map((item): AgendaRow => ({
        kind: "expected",
        at: item.scheduled_at,
        key: `e:${item.reminder_id}@${item.scheduled_at}`,
        item,
      })),
  ].sort((a, b) => a.at - b.at || a.key.localeCompare(b.key));

  const days: AgendaDay[] = data.days.slice(0, 3).map(([start, end], i) => {
    // An occurrence scheduled before yesterday began isn't in any day.
    const inDay = rows.filter((r) => r.at >= start && r.at < end);
    const isToday = now >= start && now < end;
    let ruleBefore: number | null = null;
    if (isToday) {
      const i2 = inDay.findIndex((r) => r.at > now);
      ruleBefore = i2 === -1 ? inDay.length : i2;
    }
    return { name: NAMES[i] ?? "today", start, end, rows: inDay, ruleBefore };
  });
  return { now: nowRows, days };
}

/** The key that identifies a row, for expanding it in place. */
export function rowKey(row: NowRow): string {
  return `o:${row.item.occurrence_id}`;
}

/** How a closed row reads: "Done", "Skipped", "Missed", with "(paused)" for a pause's skip. */
export function closedWord(e: EarlierItem): string {
  const word = e.kind === "completed" ? "Done" : e.kind === "skipped" ? "Skipped" : "Missed";
  return e.paused ? `${word} (paused)` : word;
}
