import type { Pause, PauseCause } from "./api";
import type { Built } from "./delays";

/**
 * Pausing a reminder or a list (spec: Reminders -> Acting on occurrences ->
 * Pause). All pure: the editor, the occurrence panel and the sidebar only
 * hold the form and call these.
 *
 * A pause runs from when it is made until a day, which is the start of that
 * day in local time, so "Paused until 3 March" fires 3 March's reminders; or
 * until it is resumed. It only sets things aside while it covers the time: the
 * core keeps a pause that has run out.
 */

const MONTHS = [
  "January",
  "February",
  "March",
  "April",
  "May",
  "June",
  "July",
  "August",
  "September",
  "October",
  "November",
  "December",
];

const pad = (n: number) => String(n).padStart(2, "0");

/** The pause if it covers `now` (unix seconds), otherwise none. */
export function activePause(p: Pause | null, now: number): Pause | null {
  if (!p) return null;
  return now >= p.from && (p.until === null || now < p.until) ? p : null;
}

/** "3 March", with the year when it isn't this year's: "3 March 2027". */
export function formatPauseDate(unix: number, now: number): string {
  const d = new Date(unix * 1000);
  const text = `${d.getDate()} ${MONTHS[d.getMonth()]}`;
  return d.getFullYear() === new Date(now * 1000).getFullYear() ? text : `${text} ${d.getFullYear()}`;
}

/** "until 3 March", or "until resumed". */
export function untilText(until: number | null, now: number): string {
  return until === null ? "until resumed" : `until ${formatPauseDate(until, now)}`;
}

/**
 * The Pause section's one line while it is folded: "Paused until 3 March",
 * "Paused until resumed", "Paused with its list until 3 March" or "Not paused".
 */
export function pauseLine(
  own: Pause | null,
  list: Pause | null,
  now: number,
): { text: string; isDefault: boolean } {
  const mine = activePause(own, now);
  if (mine) return { text: `Paused ${untilText(mine.until, now)}`, isDefault: false };
  const theirs = activePause(list, now);
  if (theirs) {
    return { text: `Paused with its list ${untilText(theirs.until, now)}`, isDefault: false };
  }
  return { text: "Not paused", isDefault: true };
}

/** What a list's pause says in the sidebar, or null when it isn't paused. */
export function listPauseLine(p: Pause | null, now: number): string | null {
  const active = activePause(p, now);
  return active ? `Paused ${untilText(active.until, now)}` : null;
}

/** What put a skipped occurrence under a pause, for the history. */
export function causeText(c: PauseCause, now: number): string {
  return `${c.list ? "the list's pause" : "the pause"} ${untilText(c.until, now)}`;
}

/** The Pause section's form. */
export interface PauseForm {
  mode: "off" | "until" | "resumed";
  /** The day it ends, "2026-03-03", when mode is "until". */
  date: string;
}

/** A day ("2026-03-03") as unix seconds at the start of it in local time. */
export function dayStart(date: string): number | null {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return null;
  const ms = new Date(`${date}T00:00:00`).getTime();
  return Number.isNaN(ms) ? null : Math.floor(ms / 1000);
}

/** The local day ("2026-03-03") a time falls on. */
export function dayOf(unix: number): string {
  const d = new Date(unix * 1000);
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** The day after today, which is the earliest a pause can end. */
export function tomorrowOf(now: number): string {
  const d = new Date(now * 1000);
  d.setDate(d.getDate() + 1);
  return dayOf(Math.floor(d.getTime() / 1000));
}

/** A week from now: where the day picker starts. */
export function nextWeekOf(now: number): string {
  const d = new Date(now * 1000);
  d.setDate(d.getDate() + 7);
  return dayOf(Math.floor(d.getTime() / 1000));
}

/** The form for a reminder's pause as it stands now. */
export function pauseFormOf(own: Pause | null, now: number): PauseForm {
  const mine = activePause(own, now);
  if (!mine) return { mode: "off", date: nextWeekOf(now) };
  return mine.until === null
    ? { mode: "resumed", date: nextWeekOf(now) }
    : { mode: "until", date: dayOf(mine.until) };
}

/** What saving the form asks of the core. */
export type PauseAction =
  | { kind: "none" }
  | { kind: "pause"; until: number | null }
  | { kind: "resume" };

/**
 * What the form means, given the reminder's pause as it stands: nothing if it
 * leaves it as it is, a pause (new, or moved to another end), or a resume.
 * Resuming early is just turning the pause off.
 */
export function pauseAction(form: PauseForm, own: Pause | null, now: number): Built<PauseAction> {
  const mine = activePause(own, now);
  switch (form.mode) {
    case "off":
      return { ok: true, value: mine ? { kind: "resume" } : { kind: "none" } };
    case "resumed":
      return {
        ok: true,
        value: mine && mine.until === null ? { kind: "none" } : { kind: "pause", until: null },
      };
    case "until": {
      const until = dayStart(form.date);
      if (until === null) return { ok: false, error: "Pick the day the pause ends." };
      if (until <= now) {
        return { ok: false, error: "Pick a day after today: the pause ends at the start of it." };
      }
      return {
        ok: true,
        value: mine && mine.until === until ? { kind: "none" } : { kind: "pause", until },
      };
    }
  }
}

/** The end a quick "Pause" asks for: a day, or until resumed. */
export function pauseUntil(
  untilResumed: boolean,
  date: string,
  now: number,
): Built<number | null> {
  if (untilResumed) return { ok: true, value: null };
  const a = pauseAction({ mode: "until", date }, null, now);
  if (!a.ok) return a;
  return { ok: true, value: a.value.kind === "pause" ? a.value.until : null };
}

/** The editor sentence's mention of a pause: "until 3 March", or null if it isn't paused. */
export function pauseSummary(form: PauseForm, now: number): string | null {
  switch (form.mode) {
    case "off":
      return null;
    case "resumed":
      return "until resumed";
    case "until": {
      const until = dayStart(form.date);
      return until === null ? null : untilText(until, now);
    }
  }
}
