import type { QuietHours, Scope, SnoozeAllView } from "./api";
import { clock, nextTimeOfDay } from "./snooze";

/** What the Snooze all dialog says it does (spec: Alerts → Snooze all). */
export const SNOOZE_ALL_NOTE =
  "Snoozing all quiets what is open now and anything that fires before it ends, on all your " +
  "devices. Reminders still go overdue on schedule, and last-chance alerts still come. " +
  "Maximum priority is left out unless you include it. You can end it early.";

/** The days of the week as the core names them, Monday first. */
export const DAYS: Array<[string, string]> = [
  ["MO", "Mon"],
  ["TU", "Tue"],
  ["WE", "Wed"],
  ["TH", "Thu"],
  ["FR", "Fri"],
  ["SA", "Sat"],
  ["SU", "Sun"],
];
const WEEKNIGHTS = ["MO", "TU", "WE", "TH", "FR"];

export const allScope: Scope = { kind: "all" };

/** The scope a `<select>` value stands for: "" is everything, otherwise a list id. */
export const scopeOfValue = (value: string): Scope =>
  value === "" ? allScope : { kind: "list", list_id: value };
export const valueOfScope = (s: Scope): string => (s.kind === "all" ? "" : s.list_id);

/** What a snooze-all lasts: a length from a menu, tomorrow morning, or a time of day. */
export type Length =
  | { kind: "minutes"; minutes: number }
  | { kind: "tomorrow_morning" }
  | { kind: "time"; time: string };

/** The dialog's quick lengths. */
export const LENGTHS: Array<[string, Length]> = [
  ["30 minutes", { kind: "minutes", minutes: 30 }],
  ["1 hour", { kind: "minutes", minutes: 60 }],
  ["2 hours", { kind: "minutes", minutes: 120 }],
  ["4 hours", { kind: "minutes", minutes: 240 }],
  ["Until tomorrow morning", { kind: "tomorrow_morning" }],
];

/** The hour "tomorrow morning" means: the same as the snooze menu's. */
export const TOMORROW_MORNING = "08:00";

/** When a length ends, counted from `nowSeconds`; null for a time that isn't one. */
export function lengthEnd(length: Length, nowSeconds: number): number | null {
  switch (length.kind) {
    case "minutes":
      return length.minutes > 0 ? nowSeconds + length.minutes * 60 : null;
    case "time":
      return nextTimeOfDay(length.time, nowSeconds);
    case "tomorrow_morning": {
      const d = new Date(nowSeconds * 1000);
      d.setDate(d.getDate() + 1);
      d.setHours(8, 0, 0, 0);
      return Math.floor(d.getTime() / 1000);
    }
  }
}

/** "19:00", or "Tue 19:00" when it isn't today. */
export function untilLabel(until: number, nowSeconds: number): string {
  const day = (t: number) => new Date(t * 1000).toDateString();
  if (day(until) === day(nowSeconds)) return clock(until);
  const name = new Date(until * 1000).toLocaleDateString([], { weekday: "short" });
  return `${name} ${clock(until)}`;
}

/** What a snooze-all or quiet hours cover, for a chip: "All", or the list's name. */
export function scopeName(v: Pick<SnoozeAllView, "scope" | "list_name">): string {
  if (v.scope.kind === "all") return "All";
  return v.list_name ?? "A list";
}

/** The toolbar chip's text: "All snoozed until 19:00". */
export function chipText(v: SnoozeAllView, nowSeconds: number): string {
  const max = v.include_maximum ? ", Maximum included" : "";
  return `${scopeName(v)} snoozed until ${untilLabel(v.until, nowSeconds)}${max}`;
}

/** The snooze-alls to show as chips (quiet hours aren't ended from there). */
export function chips(holding: SnoozeAllView[], nowSeconds: number): SnoozeAllView[] {
  return holding.filter((h) => h.source === "snooze_all" && h.id !== null && h.until > nowSeconds);
}

/** The quiet hours stretch in progress, for the sidebar footer, if any. */
export function quietNow(holding: SnoozeAllView[], nowSeconds: number): SnoozeAllView | null {
  return holding.find((h) => h.source === "quiet_hours" && h.until > nowSeconds) ?? null;
}

/** "weeknights", "every night", "Sat and Sun nights", "Mon, Wed and Fri nights". */
export function describeDays(days: string[]): string {
  const set = new Set(days);
  const ordered = DAYS.filter(([d]) => set.has(d));
  if (ordered.length === 7) return "every night";
  if (ordered.length === 5 && WEEKNIGHTS.every((d) => set.has(d))) return "weeknights";
  if (ordered.length === 2 && set.has("SA") && set.has("SU")) return "weekend nights";
  const names = ordered.map(([, n]) => n);
  if (names.length === 0) return "no nights";
  if (names.length === 1) return `${names[0]} nights`;
  return `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]} nights`;
}

/** "22:00 to 07:00 on weeknights, for all reminders, Maximum left out". */
export function describeQuietHours(q: QuietHours, listName?: string | null): string {
  const what = q.scope.kind === "all" ? "all reminders" : (listName ?? "one list");
  const max = q.include_maximum ? "Maximum included" : "Maximum left out";
  return `${q.from} to ${q.to} on ${describeDays(q.days)}, for ${what}, ${max}`;
}

/** A new rule: 22:00 to 07:00 on weeknights, for everything, Maximum left out. */
export function newQuietHours(): QuietHours {
  return {
    days: [...WEEKNIGHTS],
    from: "22:00",
    to: "07:00",
    scope: allScope,
    include_maximum: false,
  };
}

/** Turns a day on or off, keeping the week's order. */
export function toggleDay(days: string[], day: string, on: boolean): string[] {
  const set = new Set(days);
  if (on) set.add(day);
  else set.delete(day);
  return DAYS.map(([d]) => d).filter((d) => set.has(d));
}

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;

/** Why the rule can't be used, in the core's terms, or null. */
export function checkQuietHours(q: QuietHours): string | null {
  if (q.days.length === 0) return "Choose at least one day.";
  if (!TIME.test(q.from) || !TIME.test(q.to)) return "Quiet hours take times such as 22:00.";
  if (q.from === q.to) return "Quiet hours can't start and end at the same time.";
  return null;
}

/** The sidebar footer's line about quiet hours. */
export function quietLine(rules: QuietHours[], holding: SnoozeAllView[], nowSeconds: number): string {
  const now = quietNow(holding, nowSeconds);
  if (now) return `Quiet hours until ${untilLabel(now.until, nowSeconds)}`;
  if (rules.length === 0) return "No quiet hours";
  const r = rules[0];
  const more = rules.length > 1 ? ` and ${rules.length - 1} more` : "";
  return `Quiet hours ${r.from}–${r.to}${more}`;
}
