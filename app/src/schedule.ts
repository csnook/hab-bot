/** The common schedule patterns, as iCalendar recurrence rules (RRULE). */

export type Pattern = "once" | "daily" | "weekdays" | "weekly" | "monthly_date" | "monthly_weekday" | "custom";

export const DAYS = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"] as const;
export const DAY_NAMES = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const ORDINALS: [string, number][] = [["first", 1], ["second", 2], ["third", 3], ["fourth", 4], ["last", -1]];
export const ORDINAL_NAMES = ORDINALS.map(([n]) => n);

export interface Choice {
  pattern: Pattern;
  /** Indexes into DAYS, for "weekly". */
  days: number[];
  /** For "monthly_weekday": the ordinal's index and the weekday's index. */
  ordinal: number;
  weekday: number;
  custom: string;
}

/** The RRULE for a choice, or null for "once". The start's date supplies "monthly by date". */
export function rule(c: Choice, start: Date): string | null {
  switch (c.pattern) {
    case "once":
      return null;
    case "daily":
      return "FREQ=DAILY";
    case "weekdays":
      return "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR";
    case "weekly":
      return `FREQ=WEEKLY;BYDAY=${(c.days.length ? c.days : [0]).map((i) => DAYS[i]).join(",")}`;
    case "monthly_date":
      return `FREQ=MONTHLY;BYMONTHDAY=${start.getDate()}`;
    case "monthly_weekday":
      return `FREQ=MONTHLY;BYDAY=${ORDINALS[c.ordinal][1]}${DAYS[c.weekday]}`;
    case "custom":
      return c.custom.trim().replace(/^RRULE:/i, "");
  }
}

/** `2026-10-05T07:00`: the wall-clock time the core expects. */
export function wall(d: Date): string {
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}T${p(d.getHours())}:${p(d.getMinutes())}`;
}
