import type {
  Countdown,
  CountdownUnit,
  DelaySpec,
  EditArgs,
  Pattern,
  PriorityName,
  ReminderView,
} from "./api";
import { countdownFor, hasTimeOfDay, lastDoneSeconds, type LastDone } from "./countdown";
import {
  durationSeconds,
  emptyNext,
  nextFormOf,
  nextSpec,
  splitDuration,
  type Built,
  type DurationUnit,
  type NextForm,
} from "./delays";
import { pauseAction, pauseFormOf, pauseSummary, type PauseAction, type PauseForm } from "./pause";
import { patternFor, type Repeat } from "./repeat";
import type { SummaryInput, SummaryTrigger } from "./summary";
import { toUnixSeconds } from "./time";

/**
 * The reminder editor's form state and what it turns into. All pure: the
 * component only holds the state and calls these.
 */

/** The Overdue section: following the priority, or overridden. */
export type OverdueForm =
  | { mode: "default" }
  | { mode: "after"; amount: number; unit: DurationUnit }
  | { mode: "next"; next: NextForm }
  /** A hand-written schedule the editor shows but can't edit. */
  | { mode: "keep"; spec: DelaySpec };

/** One expiry added in the Expiry section. */
export type ExpiryForm =
  | { kind: "after"; amount: number; unit: DurationUnit }
  | { kind: "next"; next: NextForm }
  | { kind: "keep"; spec: DelaySpec };

export interface EditorState {
  title: string;
  /** The list the reminder is in or goes in; "" until the lists have loaded (the personal list). */
  listId: string;
  note: string;
  priority: PriorityName;
  /** The trigger. Editing keeps its kind: a one-off, a schedule or a countdown. */
  repeat: Repeat;
  date: string;
  time: string;
  days: string[];
  /** Keep to the zone it was made in when travelling. */
  pinned: boolean;
  amount: number;
  unit: CountdownUnit;
  timeOfDay: string;
  lastDone: LastDone;
  lastDoneAt: string;
  overdue: OverdueForm;
  expiries: ExpiryForm[];
  /** An existing reminder's When the editor can't show or change. */
  whenLocked: string | null;
  /** The user has changed the When, so an edit sends it. */
  whenTouched: boolean;
  /** The Pause section: only an existing reminder has one. */
  pause: PauseForm;
}

export function newState(listId = ""): EditorState {
  return {
    title: "",
    listId,
    note: "",
    priority: "medium",
    repeat: "once",
    date: "",
    time: "",
    days: [],
    pinned: false,
    amount: 3,
    unit: "days",
    timeOfDay: "",
    lastDone: "now",
    lastDoneAt: "",
    overdue: { mode: "default" },
    expiries: [],
    whenLocked: null,
    whenTouched: false,
    pause: { mode: "off", date: "" },
  };
}

const pad = (n: number) => String(n).padStart(2, "0");

/** A unix time as the local date ("2026-10-03") and time ("09:30"). */
export function localDateTime(unix: number): { date: string; time: string } {
  const d = new Date(unix * 1000);
  return {
    date: `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`,
    time: `${pad(d.getHours())}:${pad(d.getMinutes())}`,
  };
}

/** The repeat choice a pattern is. */
export function repeatOf(p: Pattern): Repeat {
  switch (p.kind) {
    case "daily":
    case "weekdays":
    case "weekly":
      return p.kind;
    case "monthly_by_date":
      return "monthly_date";
    case "monthly_by_weekday":
      return "monthly_weekday";
  }
}

export function overdueFormOf(spec: DelaySpec | null): OverdueForm {
  if (!spec) return { mode: "default" };
  switch (spec.kind) {
    case "after":
      return { mode: "after", ...splitDuration(spec.seconds) };
    case "next":
      return { mode: "next", next: nextFormOf(spec.pattern, spec.time) };
    case "other":
      return { mode: "keep", spec };
  }
}

export function expiryFormOf(spec: DelaySpec): ExpiryForm {
  switch (spec.kind) {
    case "after":
      return { kind: "after", ...splitDuration(spec.seconds) };
    case "next":
      return { kind: "next", next: nextFormOf(spec.pattern, spec.time) };
    case "other":
      return { kind: "keep", spec };
  }
}

/** The form for an existing reminder. */
export function stateFromView(
  v: ReminderView,
  now = Math.floor(Date.now() / 1000),
): EditorState {
  const s: EditorState = {
    ...newState(v.list_id),
    pause: pauseFormOf(v.pause, now),
    title: v.title,
    note: v.note,
    priority: v.priority,
    pinned: v.zone !== null,
    overdue: overdueFormOf(v.overdue),
    expiries: v.expiries.map(expiryFormOf),
  };
  const t = v.trigger;
  if (t.kind === "one_off") {
    return { ...s, repeat: "once", ...localDateTime(t.fire_at) };
  }
  if (t.kind === "countdown") {
    return {
      ...s,
      repeat: "countdown",
      amount: t.countdown.amount,
      unit: t.countdown.unit,
      timeOfDay: t.countdown.at ?? "",
    };
  }
  if (t.schedules.length === 1 && t.schedules[0].parts) {
    const p = t.schedules[0].parts;
    return {
      ...s,
      repeat: repeatOf(p.pattern),
      date: p.date,
      time: p.time,
      days: p.pattern.kind === "weekly" ? p.pattern.days : [],
    };
  }
  const rules = t.schedules.map((x) => x.rule).join("; ");
  return {
    ...s,
    repeat: "daily",
    whenLocked:
      t.schedules.length === 1
        ? `Repeats on a rule written by hand (${rules}), which can't be changed here.`
        : `Repeats on ${t.schedules.length} schedules, which can't be changed here.`,
  };
}

function durationOrError(amount: number, unit: DurationUnit): Built<number> {
  const s = durationSeconds(amount, unit);
  return s === null
    ? { ok: false, error: "Choose how long, as a whole number of at least 1." }
    : { ok: true, value: s };
}

/** The overdue override the form means: null follows the priority. */
export function overdueSpec(f: OverdueForm): Built<DelaySpec | null> {
  switch (f.mode) {
    case "default":
      return { ok: true, value: null };
    case "after": {
      const d = durationOrError(f.amount, f.unit);
      return d.ok ? { ok: true, value: { kind: "after", seconds: d.value } } : d;
    }
    case "next":
      return nextSpec(f.next);
    case "keep":
      return { ok: true, value: f.spec };
  }
}

/** The expiries the form means, all of them. */
export function expirySpecs(forms: ExpiryForm[]): Built<DelaySpec[]> {
  const out: DelaySpec[] = [];
  for (const f of forms) {
    if (f.kind === "keep") {
      out.push(f.spec);
    } else if (f.kind === "after") {
      const d = durationOrError(f.amount, f.unit);
      if (!d.ok) return d;
      out.push({ kind: "after", seconds: d.value });
    } else {
      const n = nextSpec(f.next);
      if (!n.ok) return n;
      out.push(n.value);
    }
  }
  return { ok: true, value: out };
}

export const newExpiry = (): ExpiryForm => ({ kind: "after", amount: 1, unit: "hours" });
export const newNextExpiry = (): ExpiryForm => ({
  kind: "next",
  next: { ...emptyNext(), time: "23:59" },
});

/** What creating the reminder takes: the trigger, then the settings made after it. */
export type CreatePlan =
  | { kind: "once"; title: string; fireAt: number }
  | { kind: "schedule"; title: string; pattern: Pattern; date: string; time: string; zone: string | null }
  | { kind: "countdown"; title: string; countdown: Countdown; lastDone: number | null; zone: string | null };

export interface NewReminder {
  plan: CreatePlan;
  priority: PriorityName;
  /** Overdue, expiry and note, set once the reminder exists; null if none. */
  extras: EditArgs | null;
}

/** Everything the form asks, checked, or the first thing wrong. */
export function buildNew(
  s: EditorState,
  nowSeconds: number,
  zoneName: string,
): Built<NewReminder> {
  const title = s.title.trim();
  if (!title) return { ok: false, error: "Give the reminder a title." };
  const extras = extrasOf(s);
  if (!extras.ok) return extras;
  let plan: CreatePlan;
  if (s.repeat === "countdown") {
    const c = countdownFor(s.amount, s.unit, s.timeOfDay);
    if (!c.ok) return c;
    const done = lastDoneSeconds(s.lastDone, nowSeconds, s.lastDoneAt);
    if (!done.ok) return done;
    plan = {
      kind: "countdown",
      title,
      countdown: c.value,
      lastDone: done.value,
      zone: s.pinned && c.value.at ? zoneName : null,
    };
  } else {
    const fireAt = toUnixSeconds(s.date, s.time);
    if (fireAt === null) return { ok: false, error: "Pick a date and time." };
    if (s.repeat === "once") {
      plan = { kind: "once", title, fireAt };
    } else {
      const pattern = patternFor(s.repeat, s.date, s.days);
      if (!pattern) return { ok: false, error: "Choose the days it repeats on." };
      plan = {
        kind: "schedule",
        title,
        pattern,
        date: s.date,
        time: s.time,
        zone: s.pinned ? zoneName : null,
      };
    }
  }
  const e = extras.value;
  return {
    ok: true,
    value: { plan, priority: s.priority, extras: Object.keys(e).length ? e : null },
  };
}

/** The note, overdue override and expiries, as far as they differ from none. */
function extrasOf(s: EditorState): Built<EditArgs> {
  const out: EditArgs = {};
  if (s.note.trim()) out.note = s.note.trim();
  const o = overdueSpec(s.overdue);
  if (!o.ok) return o;
  if (o.value) out.overdue = o.value;
  const x = expirySpecs(s.expiries);
  if (!x.ok) return x;
  if (x.value.length) out.expiry = x.value;
  return { ok: true, value: out };
}

/**
 * The list an edit moves the reminder to, or null if it stays where it is.
 * Moving keeps its history.
 */
export function listMove(s: EditorState, v: ReminderView): string | null {
  return s.listId && s.listId !== v.list_id ? s.listId : null;
}

/** The list a new reminder is made in: the one chosen, or null for the personal list. */
export function newReminderList(s: EditorState): string | null {
  return s.listId || null;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);

/** The changes between an existing reminder and the form, or nothing if there are none. */
export function buildEdit(
  s: EditorState,
  v: ReminderView,
  zoneName: string,
): Built<EditArgs> {
  const title = s.title.trim();
  if (!title) return { ok: false, error: "Give the reminder a title." };
  const edit: EditArgs = {};
  if (title !== v.title) edit.title = title;
  if (s.note.trim() !== v.note.trim()) edit.note = s.note.trim();
  if (s.priority !== v.priority) edit.priority = s.priority;

  const o = overdueSpec(s.overdue);
  if (!o.ok) return o;
  if (!same(o.value, v.overdue)) {
    if (o.value) edit.overdue = o.value;
    else edit.follow_priority = true;
  }
  const x = expirySpecs(s.expiries);
  if (!x.ok) return x;
  if (!same(x.value, v.expiries)) edit.expiry = x.value;

  if (s.whenTouched && !s.whenLocked) {
    const t = v.trigger;
    if (t.kind === "countdown") {
      const c = countdownFor(s.amount, s.unit, s.timeOfDay);
      if (!c.ok) return c;
      if (!same(c.value, t.countdown)) edit.countdown = c.value;
    } else {
      const fireAt = toUnixSeconds(s.date, s.time);
      if (fireAt === null) return { ok: false, error: "Pick a date and time." };
      if (t.kind === "one_off") {
        if (fireAt !== t.fire_at) edit.fire_at = fireAt;
      } else {
        const pattern = patternFor(s.repeat, s.date, s.days);
        if (!pattern) return { ok: false, error: "Choose the days it repeats on." };
        edit.schedule = { pattern, date: s.date, time: s.time };
      }
    }
  }
  const hadZone = v.zone !== null;
  if (s.pinned !== hadZone) {
    if (s.pinned) edit.zone = zoneName;
    else edit.floating = true;
  }
  return { ok: true, value: edit };
}

/**
 * What the Pause section asks of the core when the form is saved, given the
 * reminder as it was opened. Pausing is not one of the edited settings: it
 * skips what falls in the period, so the core does it as its own action.
 */
export function buildPause(s: EditorState, v: ReminderView, now: number): Built<PauseAction> {
  return pauseAction(s.pause, v.pause, now);
}

/** The reminder's trigger, for the summary sentence. */
function summaryTrigger(s: EditorState): SummaryTrigger {
  if (s.repeat === "countdown") {
    const c = countdownFor(s.amount, s.unit, s.timeOfDay);
    return c.ok ? { kind: "countdown", countdown: c.value } : { kind: "incomplete" };
  }
  if (!s.date || !s.time) return { kind: "incomplete" };
  if (s.repeat === "once") return { kind: "once", date: s.date, time: s.time };
  const pattern = patternFor(s.repeat, s.date, s.days);
  return pattern ? { kind: "schedule", pattern, time: s.time } : { kind: "incomplete" };
}

/** What the live sentence at the top of the editor is built from. */
export function summaryInput(
  s: EditorState,
  listName: string,
  zoneName: string | null,
  now = Math.floor(Date.now() / 1000),
): SummaryInput {
  const o = overdueSpec(s.overdue);
  const x = expirySpecs(s.expiries);
  const zonePinned =
    s.pinned && (s.repeat === "countdown" ? hasTimeOfDay(s.unit) && s.timeOfDay !== "" : s.repeat !== "once");
  return {
    title: s.title,
    list: listName,
    priority: s.priority,
    trigger: summaryTrigger(s),
    zone: zonePinned ? zoneName : null,
    overdue: o.ok ? o.value : null,
    expiries: x.ok ? x.value : [],
    paused: pauseSummary(s.pause, now),
  };
}
