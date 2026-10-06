import type { BoardCard, Faking, Filters, PauseCause } from "./api";
import { PRIORITIES, shows } from "./filters";
import { untilText } from "./pause";

/**
 * The Board's logic: every reminder is a card, in the column its state puts
 * it in (spec: Desktop -> Board). The core hands over the facts for each
 * reminder (Core::board); which column a card belongs in is decided here.
 *
 * Waiting is in the column order but nothing is ever put in it: the core has
 * no waiting state yet (a condition never makes a reminder wait, ADR 0012), so
 * the column stays empty, and empty columns are hidden. Watching holds what
 * fires only on an event trigger; there are none yet, so it holds only a
 * reminder that has nothing open, nothing expected and was never closed.
 *
 * The sidebar's filters apply to the unfiltered cards.
 */

export type ColumnId =
  | "overdue"
  | "due"
  | "waiting"
  | "expected"
  | "watching"
  | "paused"
  | "finished";

export const COLUMNS: ColumnId[] = [
  "overdue",
  "due",
  "waiting",
  "expected",
  "watching",
  "paused",
  "finished",
];

const TITLES: Record<ColumnId, string> = {
  overdue: "Overdue",
  due: "Due",
  waiting: "Waiting",
  expected: "Expected",
  watching: "Watching",
  paused: "Paused",
  finished: "Finished",
};

export const columnTitle = (c: ColumnId): string => TITLES[c];

/** Finished shows what was closed in this many days; "Show older" the rest. */
export const FINISHED_DAYS = 7;
const DAY = 86_400;

/** The pause if it still holds at `now`: the core's is as of when it was asked. */
export function holdingPause(p: PauseCause | null, now: number): PauseCause | null {
  return p && (p.until === null || now < p.until) ? p : null;
}

/** The column a reminder's card is in. */
export function columnOf(card: BoardCard, now: number): ColumnId {
  // An open occurrence keeps the card in Overdue or Due until it closes, even
  // under a pause.
  if (card.open) return card.open.overdue_at <= now ? "overdue" : "due";
  if (!card.next) return card.last_closed_at !== null ? "finished" : "watching";
  if (holdingPause(card.pause, now)) return "paused";
  return "expected";
}

export interface Column {
  id: ColumnId;
  title: string;
  cards: BoardCard[];
  /** Finished only: closed more than a week ago and not shown. */
  olderHidden: number;
}

const rank = (c: BoardCard) => PRIORITIES.indexOf(c.priority);

/** Higher priority first, then the card's own time, then a stable order. */
function compare(id: ColumnId): (a: BoardCard, b: BoardCard) => number {
  const tail = (a: BoardCard, b: BoardCard) =>
    a.title.localeCompare(b.title) || a.reminder_id.localeCompare(b.reminder_id);
  switch (id) {
    case "overdue":
      // As in the Inbox: highest priority first, then the longest overdue.
      return (a, b) =>
        rank(a) - rank(b) || a.open!.overdue_at - b.open!.overdue_at || tail(a, b);
    case "due":
      return (a, b) => a.open!.fired_at - b.open!.fired_at || rank(a) - rank(b) || tail(a, b);
    case "expected":
      return (a, b) =>
        a.next!.scheduled_at - b.next!.scheduled_at || rank(a) - rank(b) || tail(a, b);
    case "finished":
      return (a, b) => (b.last_closed_at ?? 0) - (a.last_closed_at ?? 0) || tail(a, b);
    default:
      return (a, b) => rank(a) - rank(b) || tail(a, b);
  }
}

/**
 * The columns with cards in them, in the spec's order; empty ones are left
 * out. Finished holds the last 7 days unless `showOlder`.
 */
export function buildBoard(
  filters: Filters,
  cards: BoardCard[],
  now: number,
  showOlder = false,
): Column[] {
  const by = new Map<ColumnId, BoardCard[]>(COLUMNS.map((c) => [c, []]));
  for (const c of cards) {
    if (shows(filters, c)) by.get(columnOf(c, now))!.push(c);
  }
  const out: Column[] = [];
  for (const id of COLUMNS) {
    let list = by.get(id)!.sort(compare(id));
    let olderHidden = 0;
    if (id === "finished" && !showOlder) {
      const since = now - FINISHED_DAYS * DAY;
      const recent = list.filter((c) => (c.last_closed_at ?? 0) >= since);
      olderHidden = list.length - recent.length;
      list = recent;
    }
    if (list.length > 0 || olderHidden > 0) {
      out.push({ id, title: columnTitle(id), cards: list, olderHidden });
    }
  }
  return out;
}

const pad = (n: number) => String(n).padStart(2, "0");
const clock = (d: Date) => `${pad(d.getHours())}:${pad(d.getMinutes())}`;
const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

const sameDay = (a: Date, b: Date) =>
  a.getFullYear() === b.getFullYear() && a.getMonth() === b.getMonth() && a.getDate() === b.getDate();

/** "today 19:00", "tomorrow 07:00", "yesterday 21:30", "Thu 15:00" within a week, else "3 Nov 09:00". */
export function dayTime(unix: number, now: number): string {
  const t = new Date(unix * 1000);
  const n = new Date(now * 1000);
  const shifted = (days: number) => {
    const d = new Date(n);
    d.setDate(d.getDate() + days);
    return d;
  };
  if (sameDay(t, n)) return `today ${clock(t)}`;
  if (sameDay(t, shifted(1))) return `tomorrow ${clock(t)}`;
  if (sameDay(t, shifted(-1))) return `yesterday ${clock(t)}`;
  const ahead = (unix - now) / DAY;
  if (ahead > 0 && ahead < 7) return `${WEEKDAYS[t.getDay()]} ${clock(t)}`;
  const year = t.getFullYear() === n.getFullYear() ? "" : ` ${t.getFullYear()}`;
  return `${t.getDate()} ${MONTHS[t.getMonth()]}${year} ${clock(t)}`;
}

/** The card's status line: "Next: today 19:00", "Overdue since …", "Paused until 3 March". */
export function statusLine(card: BoardCard, now: number): string {
  const column = columnOf(card, now);
  switch (column) {
    case "overdue":
      return `Overdue: was due ${dayTime(card.open!.scheduled_at, now)}${pausedNote(card, now)}`;
    case "due":
      return `Due ${dayTime(card.open!.scheduled_at, now)}${pausedNote(card, now)}`;
    case "expected":
      return `Next: ${dayTime(card.next!.scheduled_at, now)}`;
    case "paused": {
      const p = holdingPause(card.pause, now)!;
      return `Paused ${p.list ? "with its list " : ""}${untilText(p.until, now)}`;
    }
    case "finished":
      return `Finished ${dayTime(card.last_closed_at!, now)}`;
    case "watching":
      return "Fires when something happens";
    case "waiting":
      return "Waiting";
  }
}

function pausedNote(card: BoardCard, now: number): string {
  const p = holdingPause(card.pause, now);
  return p ? ` (paused ${p.list ? "with its list " : ""}${untilText(p.until, now)})` : "";
}

/** The small badge's text. */
export function fakingText(f: Faking): string {
  return f === "none" ? "can't be faked" : `${f} to fake`;
}

/** What the card offers for pausing: Pause, Resume, or "Resume list" when its list pauses it. */
export type PauseOffer =
  | { kind: "pause" }
  | { kind: "resume" }
  | { kind: "resume-list"; listId: string };

export function pauseOffer(card: BoardCard, now: number): PauseOffer {
  const p = holdingPause(card.pause, now);
  if (!p) return { kind: "pause" };
  // Resuming the reminder does nothing while its list is paused.
  return p.list ? { kind: "resume-list", listId: card.list_id } : { kind: "resume" };
}

/** How many cards the filters hide, for the "filtered" line. */
export function hiddenCards(filters: Filters, cards: BoardCard[]): number {
  return cards.filter((c) => !shows(filters, c)).length;
}
