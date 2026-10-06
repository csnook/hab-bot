import type { DelaySpec, Pattern, PriorityInfo, PriorityName } from "./api";
import { DAYS } from "./repeat";
import { formatInterval } from "./priorities";

/** Pure helpers for the editor's Overdue and Expiry sections. */

export type DurationUnit = "minutes" | "hours" | "days";

export const DURATION_UNITS: Array<[DurationUnit, string]> = [
  ["minutes", "minutes"],
  ["hours", "hours"],
  ["days", "days"],
];

const UNIT_SECONDS: Record<DurationUnit, number> = { minutes: 60, hours: 3_600, days: 86_400 };

/** A whole number of units, in seconds; null if it isn't one (or isn't positive). */
export function durationSeconds(amount: number, unit: DurationUnit): number | null {
  if (!Number.isInteger(amount) || amount < 1) return null;
  return amount * UNIT_SECONDS[unit];
}

/** The largest unit that divides `seconds` exactly, for filling in the form. */
export function splitDuration(seconds: number): { amount: number; unit: DurationUnit } {
  if (seconds > 0 && seconds % 86_400 === 0) return { amount: seconds / 86_400, unit: "days" };
  if (seconds > 0 && seconds % 3_600 === 0) return { amount: seconds / 3_600, unit: "hours" };
  return { amount: Math.max(1, Math.round(seconds / 60)), unit: "minutes" };
}

/** The next-time choices: which days count. */
export type NextRepeat = "daily" | "weekdays" | "weekly" | "monthly_date" | "monthly_weekday";

export const NEXT_REPEATS: Array<[NextRepeat, string]> = [
  ["daily", "any day"],
  ["weekdays", "a weekday"],
  ["weekly", "chosen days of the week"],
  ["monthly_date", "a day of the month"],
  ["monthly_weekday", "a weekday of the month"],
];

/** What the form holds for "the next time a schedule matches". */
export interface NextForm {
  repeat: NextRepeat;
  /** For chosen days of the week, as MO..SU. */
  days: string[];
  /** For a day of the month: 1 to 31, or -1 for the last. */
  day: number;
  /** For a weekday of the month: 1 to 4, or -1 for the last. */
  ordinal: number;
  weekday: string;
  /** Time of day, "00:00". */
  time: string;
}

export const emptyNext = (): NextForm => ({
  repeat: "daily",
  days: [],
  day: 1,
  ordinal: 1,
  weekday: "MO",
  time: "00:00",
});

export type Built<T> = { ok: true; value: T } | { ok: false; error: string };

/** The pattern a next-time form means. */
export function patternOf(f: NextForm): Built<Pattern> {
  switch (f.repeat) {
    case "daily":
      return { ok: true, value: { kind: "daily" } };
    case "weekdays":
      return { ok: true, value: { kind: "weekdays" } };
    case "weekly":
      return f.days.length
        ? { ok: true, value: { kind: "weekly", days: f.days } }
        : { ok: false, error: "Choose at least one day." };
    case "monthly_date":
      return Number.isInteger(f.day) && ((f.day >= 1 && f.day <= 31) || f.day === -1)
        ? { ok: true, value: { kind: "monthly_by_date", day: f.day } }
        : { ok: false, error: "Choose a day of the month, from 1 to 31 or the last." };
    case "monthly_weekday":
      return [1, 2, 3, 4, -1].includes(f.ordinal) && DAYS.some(([c]) => c === f.weekday)
        ? { ok: true, value: { kind: "monthly_by_weekday", ordinal: f.ordinal, weekday: f.weekday } }
        : { ok: false, error: "Choose which weekday of the month." };
  }
}

/** The delay a next-time form means. */
export function nextSpec(f: NextForm): Built<DelaySpec> {
  if (!/^\d{2}:\d{2}$/.test(f.time)) return { ok: false, error: "Choose a time of day." };
  const pattern = patternOf(f);
  if (!pattern.ok) return pattern;
  return { ok: true, value: { kind: "next", pattern: pattern.value, time: f.time } };
}

/** Fills the next-time form from a pattern the editor offers. */
export function nextFormOf(pattern: Pattern, time: string): NextForm {
  const f = { ...emptyNext(), time };
  switch (pattern.kind) {
    case "daily":
    case "weekdays":
      return { ...f, repeat: pattern.kind };
    case "weekly":
      return { ...f, repeat: "weekly", days: pattern.days };
    case "monthly_by_date":
      return { ...f, repeat: "monthly_date", day: pattern.day };
    case "monthly_by_weekday":
      return { ...f, repeat: "monthly_weekday", ordinal: pattern.ordinal, weekday: pattern.weekday };
  }
}

const DAY_NAMES: Record<string, string> = {
  MO: "Monday",
  TU: "Tuesday",
  WE: "Wednesday",
  TH: "Thursday",
  FR: "Friday",
  SA: "Saturday",
  SU: "Sunday",
};

export const dayName = (code: string) => DAY_NAMES[code] ?? code;

/** "1st", "2nd", "3rd", "4th", "11th", "21st". */
export function ordinalWord(n: number): string {
  const mod100 = n % 100;
  if (mod100 >= 11 && mod100 <= 13) return `${n}th`;
  switch (n % 10) {
    case 1:
      return `${n}st`;
    case 2:
      return `${n}nd`;
    case 3:
      return `${n}rd`;
    default:
      return `${n}th`;
  }
}

/** "Monday", "Monday and Thursday", "Monday, Thursday and Saturday". */
export function listDays(codes: string[]): string {
  const names = codes.map(dayName);
  if (names.length <= 1) return names.join("");
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/**
 * "the next 23:59", "the next weekday at 09:00", "the next 1st at 00:00",
 * "the next Monday or Thursday at 09:00", "the next 2nd Tuesday at 18:00".
 */
export function describeNext(pattern: Pattern, time: string): string {
  switch (pattern.kind) {
    case "daily":
      return `the next ${time}`;
    case "weekdays":
      return `the next weekday at ${time}`;
    case "weekly":
      return `the next ${pattern.days.map(dayName).join(" or ")} at ${time}`;
    case "monthly_by_date":
      return pattern.day === -1
        ? `the next last day of the month at ${time}`
        : `the next ${ordinalWord(pattern.day)} at ${time}`;
    case "monthly_by_weekday":
      return pattern.ordinal === -1
        ? `the next last ${dayName(pattern.weekday)} of the month at ${time}`
        : `the next ${ordinalWord(pattern.ordinal)} ${dayName(pattern.weekday)} at ${time}`;
  }
}

/** "after 1 h", "after 90 min", "at once", "the next 1st at 00:00". */
export function describeDelay(d: DelaySpec): string {
  switch (d.kind) {
    case "after":
      return d.seconds === 0 ? "at once" : `after ${formatInterval(d.seconds)}`;
    case "next":
      return describeNext(d.pattern, d.time);
    case "other":
      return `the next time ${d.rule} matches`;
  }
}

/** "a, or b" for several: the first to come. */
export function describeExpiries(expiries: DelaySpec[]): string {
  return expiries.map(describeDelay).join(", or ");
}

const NAMES: Record<PriorityName, string> = {
  minimum: "Minimum",
  low: "Low",
  medium: "Medium",
  high: "High",
  maximum: "Maximum",
};

export const priorityName = (p: PriorityName) => NAMES[p];

/**
 * The default overdue time with its source, shown greyed: "after 1 h (Medium)".
 * `infos` is the built-in table; without the priority in it, only the source.
 */
export function defaultOverdueText(priority: PriorityName, infos: PriorityInfo[]): string {
  const info = infos.find((i) => i.priority === priority);
  if (!info) return `follows the priority (${NAMES[priority]})`;
  const seconds = info.settings.due_interval;
  const when = seconds === 0 ? "at once" : `after ${formatInterval(seconds)}`;
  return `${when} (${NAMES[priority]})`;
}

/** The default expiry, shown greyed. */
export const DEFAULT_EXPIRY_TEXT = "when it fires again";
