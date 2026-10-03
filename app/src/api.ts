import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Priority = "minimum" | "low" | "medium" | "high" | "maximum";

export interface PrioritySettings {
  priority: Priority;
  name: string;
  due_style: string;
  overdue: { after: number; style: string }[];
  due_interval: number;
  overdue_interval: number;
  ring_duration: number | null;
  server_wait: number | null;
  swipeable: boolean;
  breaks_do_not_disturb: boolean;
}

export interface Occurrence {
  id: string;
  reminder_id: string;
  title: string;
  scheduled_at: number;
  fired_at: number;
  priority: Priority;
  overdue_at: number;
  snoozed_until: number | null;
  expires_at: number | null;
  status: "expected" | "due" | "completed" | "skipped" | "missed";
  closed_at: number | null;
  tapped_at: number | null;
  note: string | null;
  corrected_from: "expected" | "due" | "completed" | "skipped" | "missed" | null;
}

export type Trigger =
  | { kind: "one_off"; at: number }
  | { kind: "schedule"; rule: string; start: string }
  | { kind: "countdown"; unit: "hours" | "days"; amount: number; at: string | null; last_done: number | null };

export interface NewReminder {
  title: string;
  triggers: Trigger[];
  /** A named time zone, or null for floating: the clock wherever the device is. */
  tz: string | null;
  priority: Priority;
  /** How long after each scheduled time its occurrence expires, in milliseconds. */
  expiry: number | null;
}

export interface InboxItem {
  section: "overdue" | "overdue_folded" | "due" | "later_today" | "earlier_today";
  occurrence: Occurrence;
}

export const inbox = () => invoke<InboxItem[]>("inbox");
export const createOneOff = (title: string, dueAt: number) =>
  invoke<void>("create_one_off", { title, dueAt });
export const createReminder = (reminder: NewReminder) => invoke<void>("create_reminder", { reminder });
export const skip = (occurrenceId: string, note: string | null = null) => invoke<void>("skip", { occurrenceId, note });
export const skipAhead = (occurrenceId: string, note: string | null = null) => invoke<void>("skip_ahead", { occurrenceId, note });
export const completeAt = (occurrenceId: string, at: number) => invoke<void>("complete_at", { occurrenceId, at });
export const undo = (occurrenceId: string) => invoke<void>("undo", { occurrenceId });
export const correct = (occurrenceId: string, to: "completed" | "skipped", at: number, note: string | null) =>
  invoke<void>("correct", { occurrenceId, to, at, note });
export const recentSkipNotes = () => invoke<string[]>("recent_skip_notes");
export const completeEarly = (reminderId: string) => invoke<void>("complete_early", { reminderId });
export const complete = (occurrenceId: string) => invoke<void>("complete", { occurrenceId });
/** Calls back whenever the core's state changed (a reminder fired, or an action was taken). */
export const onChanged = (f: () => void) => listen("changed", f);
/** Android only; empty elsewhere. Names: "notifications", "alarms". */
export const missingPermissions = () => invoke<string[]>("missing_permissions");
export const requestPermissions = () => invoke<void>("request_permissions");
export const priorities = () => invoke<PrioritySettings[]>("priorities");
export const about = () => invoke<string>("about");
/** A notification was clicked: the occurrence to show. */
export const onOpen = (f: (occurrenceId: string) => void) => listen<string>("open", (e) => f(e.payload));
export interface SnoozePicker {
  options: { label: string; until: number }[];
  expires_at: number | null;
}
export const snooze = (occurrenceId: string) => invoke<number>("snooze", { occurrenceId });
export const snoozeUntil = (occurrenceId: string, until: number) => invoke<void>("snooze_until", { occurrenceId, until });
export const snoozePicker = (occurrenceId: string) => invoke<SnoozePicker | null>("snooze_picker", { occurrenceId });
export const skipOlderQuiet = () => invoke<number>("skip_older_quiet");
