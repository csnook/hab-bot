import type { Condition, SunEvent, SunTrigger } from "./api";
import { dayName, listDays, type Built } from "./delays";
import { formatDate } from "./summary";

/**
 * Time-based conditions and sun events in the reminder editor (spec: Sources
 * → Time). Pure: the checks mirror the core's, so the editor refuses what
 * the core would, and the wording is tested.
 */

export type ConditionKind = Condition["kind"];

/** What the "Add a condition" picker offers. */
export const CONDITION_KINDS: Array<[ConditionKind, string]> = [
  ["days", "On certain days of the week"],
  ["window", "During a time window"],
  ["dates", "Between two dates"],
  ["season", "In a season of the year"],
  ["daylight", "In daylight"],
  ["darkness", "In darkness"],
];

/** A condition just added, with something sensible filled in. */
export function newCondition(kind: ConditionKind): Condition {
  switch (kind) {
    case "days":
      return { kind, days: ["MO", "TU", "WE", "TH", "FR"] };
    case "window":
      return { kind, from: "08:00", to: "20:00" };
    case "dates":
      return { kind, from: "", to: "" };
    case "season":
      return { kind, from: "06-01", to: "08-31" };
    case "daylight":
    case "darkness":
      return { kind };
  }
}

const TIME = /^([01]\d|2[0-3]):[0-5]\d$/;
const DATE = /^\d{4}-\d{2}-\d{2}$/;

const isDate = (s: string) => {
  if (!DATE.test(s)) return false;
  const [y, m, d] = s.split("-").map(Number);
  const dt = new Date(Date.UTC(y, m - 1, d));
  return dt.getUTCFullYear() === y && dt.getUTCMonth() === m - 1 && dt.getUTCDate() === d;
};

/** "06-01" is a day some year has (29 February is one). */
const isMonthDay = (s: string) => {
  if (!/^\d{2}-\d{2}$/.test(s)) return false;
  const [m, d] = s.split("-").map(Number);
  const dt = new Date(Date.UTC(2024, m - 1, d));
  return dt.getUTCMonth() === m - 1 && dt.getUTCDate() === d;
};

/** The condition as the core will take it, or the first thing wrong. */
export function checkCondition(c: Condition): Built<Condition> {
  switch (c.kind) {
    case "days":
      return c.days.length
        ? { ok: true, value: c }
        : { ok: false, error: "Choose at least one day for the condition." };
    case "window":
      if (!TIME.test(c.from) || !TIME.test(c.to)) {
        return { ok: false, error: "Give the time window a start and an end." };
      }
      if (c.from === c.to) {
        return { ok: false, error: "A time window can't start and end at the same time." };
      }
      return { ok: true, value: c };
    case "dates":
      if (!isDate(c.from) || !isDate(c.to)) {
        return { ok: false, error: "Pick the first and last date of the range." };
      }
      return c.to < c.from
        ? { ok: false, error: "The date range ends before it starts." }
        : { ok: true, value: c };
    case "season":
      return isMonthDay(c.from) && isMonthDay(c.to)
        ? { ok: true, value: c }
        : { ok: false, error: "Give the season a first and last day, such as 06-01." };
    case "daylight":
    case "darkness":
      return { ok: true, value: c };
  }
}

/** All the conditions, checked; the first one wrong is the error. */
export function buildConditions(cs: Condition[]): Built<Condition[]> {
  const out: Condition[] = [];
  for (const c of cs) {
    const b = checkCondition(c);
    if (!b.ok) return b;
    out.push(b.value);
  }
  return { ok: true, value: out };
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** "1 Jun" for "06-01". */
export function formatMonthDay(md: string): string {
  const m = /^(\d{2})-(\d{2})$/.exec(md);
  const month = m ? MONTHS[Number(m[1]) - 1] : undefined;
  return m && month ? `${Number(m[2])} ${month}` : md;
}

const WEEKDAYS = ["MO", "TU", "WE", "TH", "FR"];

/** "on weekdays", "on Saturdays and Sundays", "on Mondays". */
function describeDays(days: string[]): string {
  const set = new Set(days);
  const is = (codes: string[]) => set.size === codes.length && codes.every((c) => set.has(c));
  if (is(WEEKDAYS)) return "on weekdays";
  if (is(["SA", "SU"])) return "on weekends";
  const order = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"].filter((d) => set.has(d));
  return `on ${listDays(order).replace(/\b(\w+day)\b/g, "$1s")}`;
}

/** One condition as a phrase for the sentence: "between 22:00 and 06:00". */
export function describeCondition(c: Condition): string {
  switch (c.kind) {
    case "days":
      return describeDays(c.days);
    case "window":
      return `between ${c.from} and ${c.to}`;
    case "dates":
      return `from ${formatDate(c.from)} to ${formatDate(c.to)}`;
    case "season":
      return `from ${formatMonthDay(c.from)} to ${formatMonthDay(c.to)} each year`;
    case "daylight":
      return "in daylight";
    case "darkness":
      return "in darkness";
  }
}

/** "on weekdays, between 08:00 and 20:00 and in daylight". Empty for none. */
export function describeConditions(cs: Condition[]): string {
  const parts = cs.map(describeCondition);
  if (parts.length <= 1) return parts.join("");
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/** Whether the conditions or sun events need the home location. */
export function needsHome(cs: Condition[], suns: SunTrigger[]): boolean {
  return suns.length > 0 || cs.some((c) => c.kind === "daylight" || c.kind === "darkness");
}

// ---- Sun events ----

export const SUN_EVENTS: Array<[SunEvent, string]> = [
  ["sunrise", "sunrise"],
  ["sunset", "sunset"],
  ["civil_dawn", "civil dawn"],
  ["civil_dusk", "civil dusk"],
];

export const sunEventName = (e: SunEvent) => SUN_EVENTS.find(([k]) => k === e)?.[1] ?? e;

export type SunDirection = "at" | "before" | "after";

/** The editor's form for one sun-event trigger. */
export interface SunForm {
  event: SunEvent;
  direction: SunDirection;
  /** Minutes before or after; ignored for "at". */
  minutes: number;
}

export const newSun = (): SunForm => ({ event: "sunset", direction: "at", minutes: 30 });

/** The most an event can be offset by: 12 hours either way. */
export const MAX_OFFSET_MINUTES = 12 * 60;

export function sunFormOf(t: SunTrigger): SunForm {
  if (t.offset_minutes === 0) return { event: t.event, direction: "at", minutes: 30 };
  return {
    event: t.event,
    direction: t.offset_minutes < 0 ? "before" : "after",
    minutes: Math.abs(t.offset_minutes),
  };
}

export function buildSun(f: SunForm): Built<SunTrigger> {
  if (f.direction === "at") return { ok: true, value: { event: f.event, offset_minutes: 0 } };
  if (!Number.isInteger(f.minutes) || f.minutes < 1 || f.minutes > MAX_OFFSET_MINUTES) {
    return { ok: false, error: "Give the offset as a whole number of minutes, up to 720." };
  }
  const sign = f.direction === "before" ? -1 : 1;
  return { ok: true, value: { event: f.event, offset_minutes: sign * f.minutes } };
}

export function buildSuns(fs: SunForm[]): Built<SunTrigger[]> {
  const out: SunTrigger[] = [];
  for (const f of fs) {
    const b = buildSun(f);
    if (!b.ok) return b;
    out.push(b.value);
  }
  return { ok: true, value: out };
}

/** "30 minutes before sunset", "at sunrise", "1 hour 30 minutes after civil dusk". */
export function describeSun(t: SunTrigger): string {
  const name = sunEventName(t.event);
  if (t.offset_minutes === 0) return `at ${name}`;
  const m = Math.abs(t.offset_minutes);
  const h = Math.floor(m / 60);
  const rest = m % 60;
  const unit = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
  const amount = [h ? unit(h, "hour") : "", rest ? unit(rest, "minute") : ""].filter(Boolean).join(" ");
  return `${amount} ${t.offset_minutes < 0 ? "before" : "after"} ${name}`;
}

/** "30 minutes before sunset and at sunrise". */
export function describeSuns(ts: SunTrigger[]): string {
  const parts = ts.map(describeSun);
  if (parts.length <= 1) return parts.join("");
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

export { dayName };
