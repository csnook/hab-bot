import type { Countdown, CountdownUnit } from "./api";

export const UNITS: Array<[CountdownUnit, string]> = [
  ["minutes", "minutes"],
  ["hours", "hours"],
  ["days", "days"],
  ["weeks", "weeks"],
];

/** Minutes and hours count elapsed time; only days and weeks can have a time of day. */
export function hasTimeOfDay(unit: CountdownUnit): boolean {
  return unit === "days" || unit === "weeks";
}

/** How the form's answer to "when was this last done?" is sent: unix seconds, or null for never. */
export type LastDone = "now" | "never" | "at";

export function lastDoneSeconds(
  choice: LastDone,
  now: number,
  at: string,
): { ok: true; value: number | null } | { ok: false; error: string } {
  if (choice === "never") return { ok: true, value: null };
  if (choice === "now") return { ok: true, value: now };
  const ms = new Date(at).getTime();
  if (!at || Number.isNaN(ms)) return { ok: false, error: "Say when it was last done." };
  const seconds = Math.floor(ms / 1000);
  if (seconds > now) return { ok: false, error: "It can't have been done in the future." };
  return { ok: true, value: seconds };
}

/** The countdown the form describes, or an error to show. A time of day only goes with days or weeks. */
export function countdownFor(
  amount: number,
  unit: CountdownUnit,
  timeOfDay: string,
): { ok: true; value: Countdown } | { ok: false; error: string } {
  if (!Number.isInteger(amount) || amount < 1) {
    return { ok: false, error: "Choose how long, as a whole number of at least 1." };
  }
  return {
    ok: true,
    value: { amount, unit, at: hasTimeOfDay(unit) && timeOfDay ? timeOfDay : null },
  };
}

/** "3 days, at 09:00" or "8 hours". */
export function describeCountdown(c: Countdown): string {
  const unit = c.amount === 1 ? c.unit.replace(/s$/, "") : c.unit;
  return c.at ? `${c.amount} ${unit}, at ${c.at}` : `${c.amount} ${unit}`;
}
