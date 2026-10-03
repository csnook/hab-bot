import type { Pattern } from "./api";

export type Repeat =
  | "once"
  | "daily"
  | "weekdays"
  | "weekly"
  | "monthly_date"
  | "monthly_weekday";

export const REPEATS: Array<[Repeat, string]> = [
  ["once", "Doesn't repeat"],
  ["daily", "Every day"],
  ["weekdays", "Every weekday"],
  ["weekly", "Weekly on chosen days"],
  ["monthly_date", "Monthly, on this date"],
  ["monthly_weekday", "Monthly, on this weekday"],
];

/** iCalendar's day codes, Monday first. */
export const DAYS: Array<[string, string]> = [
  ["MO", "Mon"],
  ["TU", "Tue"],
  ["WE", "Wed"],
  ["TH", "Thu"],
  ["FR", "Fri"],
  ["SA", "Sat"],
  ["SU", "Sun"],
];

/**
 * The pattern a repeat choice means for a start date ("2026-10-03"), or null
 * when it doesn't repeat or is missing something. Monthly by weekday takes the
 * date's weekday and which one in the month it is, with the fifth being "the
 * last".
 */
export function patternFor(repeat: Repeat, date: string, days: string[]): Pattern | null {
  switch (repeat) {
    case "once":
      return null;
    case "daily":
      return { kind: "daily" };
    case "weekdays":
      return { kind: "weekdays" };
    case "weekly":
      return days.length ? { kind: "weekly", days } : null;
    case "monthly_date": {
      const d = Number(date.split("-")[2]);
      return d >= 1 && d <= 31 ? { kind: "monthly_by_date", day: d } : null;
    }
    case "monthly_weekday": {
      const [y, m, d] = date.split("-").map(Number);
      if (!y || !m || !d) return null;
      const weekday = DAYS[(new Date(y, m - 1, d).getDay() + 6) % 7][0];
      const nth = Math.floor((d - 1) / 7) + 1;
      return { kind: "monthly_by_weekday", ordinal: nth > 4 ? -1 : nth, weekday };
    }
  }
}
