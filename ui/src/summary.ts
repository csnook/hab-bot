import type { Countdown, DelaySpec, Pattern, PriorityName } from "./api";
import { describeDelay, describeExpiries, dayName, listDays, ordinalWord, priorityName } from "./delays";

/**
 * The live, read-only sentence at the top of the reminder editor (spec:
 * Desktop -> The reminder editor). A pure function of what the form holds, so
 * every wording is tested.
 */

export type SummaryTrigger =
  /** One time, on a date ("2026-10-03") at a time ("09:30"). */
  | { kind: "once"; date: string; time: string }
  | { kind: "schedule"; pattern: Pattern; time: string }
  | { kind: "countdown"; countdown: Countdown }
  /** Not filled in yet. */
  | { kind: "incomplete" };

export interface SummaryInput {
  title: string;
  /** The reminder list's name. */
  list: string;
  priority: PriorityName;
  trigger: SummaryTrigger;
  /** The time zone it is pinned to, if it is (and it matters). */
  zone: string | null;
  /** The overdue override; null follows the priority and isn't mentioned. */
  overdue: DelaySpec | null;
  /** The expiries added; none isn't mentioned. */
  expiries: DelaySpec[];
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "3 Oct 2026", or the text as given if it isn't a date. */
export function formatDate(date: string): string {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  if (!m) return date;
  const month = MONTHS[Number(m[2]) - 1];
  return month ? `${Number(m[3])} ${month} ${m[1]}` : date;
}

/** Lowercases the first letter of "Take the bins out", but not of "NASA call" or "iPhone". */
export function lowerFirst(title: string): string {
  if (title.length < 2) return title.toLowerCase();
  const second = title[1];
  if (second === second.toUpperCase() && second !== second.toLowerCase()) return title;
  return title[0].toLowerCase() + title.slice(1);
}

/** "every weekday at 09:30", "on the 2nd Tuesday of every month at 18:00". */
export function describeSchedule(pattern: Pattern, time: string): string {
  switch (pattern.kind) {
    case "daily":
      return `every day at ${time}`;
    case "weekdays":
      return `every weekday at ${time}`;
    case "weekly":
      return `every ${listDays(pattern.days)} at ${time}`;
    case "monthly_by_date":
      return pattern.day === -1
        ? `on the last day of every month at ${time}`
        : `on the ${ordinalWord(pattern.day)} of every month at ${time}`;
    case "monthly_by_weekday":
      return pattern.ordinal === -1
        ? `on the last ${dayName(pattern.weekday)} of every month at ${time}`
        : `on the ${ordinalWord(pattern.ordinal)} ${dayName(pattern.weekday)} of every month at ${time}`;
  }
}

/** "3 days after it was last done, at 09:00". */
export function describeCountdownWhen(c: Countdown): string {
  const unit = c.amount === 1 ? c.unit.replace(/s$/, "") : c.unit;
  return `${c.amount} ${unit} after it was last done${c.at ? `, at ${c.at}` : ""}`;
}

function when(t: SummaryTrigger): string {
  switch (t.kind) {
    case "once":
      return `on ${formatDate(t.date)} at ${t.time}`;
    case "schedule":
      return describeSchedule(t.pattern, t.time);
    case "countdown":
      return describeCountdownWhen(t.countdown);
    case "incomplete":
      return "at a time still to choose";
  }
}

/**
 * "In Personal, remind me to take the bins out every Wednesday at 18:00, at
 * Medium priority. It goes overdue after 3 h. It expires after 1 h, or the next
 * 23:59, whichever comes first."
 *
 * An overdue time or expiry is mentioned only when the reminder overrides it.
 */
export function summarize(i: SummaryInput): string {
  const title = i.title.trim();
  const what = title ? lowerFirst(title) : "…";
  const zone = i.zone ? ` (in ${i.zone} time)` : "";
  const parts = [
    `In ${i.list}, remind me to ${what} ${when(i.trigger)}${zone}, at ${priorityName(i.priority)} priority.`,
  ];
  if (i.overdue) parts.push(`It goes overdue ${describeDelay(i.overdue)}.`);
  if (i.expiries.length === 1) {
    parts.push(`It expires ${describeExpiries(i.expiries)}.`);
  } else if (i.expiries.length > 1) {
    parts.push(`It expires ${describeExpiries(i.expiries)}, whichever comes first.`);
  }
  return parts.join(" ");
}

/** One line for the Overdue section while it is closed. */
export function overdueLine(
  overdue: DelaySpec | null,
  defaultText: string,
): { text: string; isDefault: boolean } {
  return overdue
    ? { text: describeDelay(overdue), isDefault: false }
    : { text: defaultText, isDefault: true };
}

/** One line for the Expiry section while it is closed. */
export function expiryLine(
  expiries: DelaySpec[],
  defaultText: string,
): { text: string; isDefault: boolean } {
  return expiries.length
    ? { text: `${describeExpiries(expiries)}, or when it fires again`, isDefault: false }
    : { text: defaultText, isDefault: true };
}

/** One line for the Note section while it is closed. */
export function noteLine(note: string): { text: string; isDefault: boolean } {
  const first = note.trim().split("\n")[0];
  if (!first) return { text: "No note", isDefault: true };
  return { text: first.length > 60 ? `${first.slice(0, 59)}…` : first, isDefault: false };
}
