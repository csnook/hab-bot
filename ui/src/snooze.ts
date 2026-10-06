import type { SnoozeOption } from "./api";

/** How long before a known expiry the last-chance alert comes, in seconds. */
export const LAST_CHANCE_LEAD = 600;

/** "09:05": the local time of day. */
export function clock(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  const two = (n: number) => String(n).padStart(2, "0");
  return `${two(d.getHours())}:${two(d.getMinutes())}`;
}

/**
 * The first moment after `nowSeconds` that the local clock reads `time`
 * ("19:30"): today if it is still ahead, otherwise tomorrow. Null when
 * `time` isn't a time.
 */
export function nextTimeOfDay(time: string, nowSeconds: number): number | null {
  const m = /^(\d{1,2}):(\d{2})$/.exec(time);
  if (!m) return null;
  const [h, min] = [Number(m[1]), Number(m[2])];
  if (h > 23 || min > 59) return null;
  const d = new Date(nowSeconds * 1000);
  d.setHours(h, min, 0, 0);
  if (d.getTime() / 1000 <= nowSeconds) d.setDate(d.getDate() + 1);
  return Math.floor(d.getTime() / 1000);
}

/** A "datetime-local" value ("2026-10-03T19:30") as unix seconds, if it is in the future. */
export function futureDateTime(value: string, nowSeconds: number): number | null {
  const ms = new Date(value).getTime();
  if (!value || Number.isNaN(ms)) return null;
  const t = Math.floor(ms / 1000);
  return t > nowSeconds ? t : null;
}

/**
 * When the last-chance alert comes for a snooze made at `setAt` that ends at
 * `until`, for an occurrence expiring at `expiresAt`: 10 minutes before the
 * expiry, if that moment falls inside the snooze and the snooze was made
 * before it. The same rule as the core's.
 */
export function lastChanceAt(expiresAt: number | null, setAt: number, until: number): number | null {
  if (expiresAt === null) return null;
  const at = expiresAt - LAST_CHANCE_LEAD;
  return until > at && setAt < at ? at : null;
}

/** "Expires at 23:59", with when the last-chance alert comes. */
export function expiryNote(expiresAt: number | null): string | null {
  if (expiresAt === null) return null;
  return `Expires at ${clock(expiresAt)}; a last-chance alert comes at ${clock(expiresAt - LAST_CHANCE_LEAD)} if it is still snoozed.`;
}

/** What a chosen end time means for a known expiry, or null if nothing to say. */
export function expiryWarning(expiresAt: number | null, setAt: number, until: number): string | null {
  const at = lastChanceAt(expiresAt, setAt, until);
  if (at === null || expiresAt === null) return null;
  return `Expires at ${clock(expiresAt)}: you'll get a last-chance alert at ${clock(at)}.`;
}

/** The menu's label for a timed choice: "1 hour", "10 minutes", "1 day", "Tomorrow morning". */
export function optionLabel(o: Pick<SnoozeOption, "kind" | "seconds">): string {
  if (o.kind === "tomorrow_morning" || o.seconds === null) return "Tomorrow morning";
  const s = o.seconds;
  const unit = (n: number, name: string) => `${n} ${name}${n === 1 ? "" : "s"}`;
  if (s % 86_400 === 0) return unit(s / 86_400, "day");
  if (s % 3_600 === 0) return unit(s / 3_600, "hour");
  return unit(Math.round(s / 60), "minute");
}
