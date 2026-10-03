import type { OccurrenceView, PriorityName } from "./api";
import { ago, formatTime } from "./time";

const NAMES: Record<PriorityName, string> = {
  minimum: "Minimum",
  low: "Low",
  medium: "Medium",
  high: "High",
  maximum: "Maximum",
};

/** "Ringing · Household · High"; the personal list is "Personal". */
export function ringingHeading(v: Pick<OccurrenceView, "list_name" | "priority">): string {
  return `Ringing · ${v.list_name ?? "Personal"} · ${NAMES[v.priority]}`;
}

/** "Due 09:30 (5 minutes ago)". */
export function dueLine(scheduledAt: number, nowSeconds: number): string {
  return `Due ${formatTime(scheduledAt)} (${ago(scheduledAt, nowSeconds)})`;
}

/** The occurrence id an alarm window was opened for, from its page's query. */
export function alarmIdOf(search: string): string | null {
  return new URLSearchParams(search).get("alarm");
}

/** The snooze choices on the alarm window, in minutes. */
export const SNOOZE_MINUTES = [5, 10, 30] as const;
