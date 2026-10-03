import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface Occurrence {
  id: string;
  reminder_id: string;
  title: string;
  scheduled_at: number;
  fired_at: number;
  status: "expected" | "due" | "completed" | "missed";
  closed_at: number | null;
}

export type Trigger =
  | { kind: "one_off"; at: number }
  | { kind: "schedule"; rule: string; start: string };

export interface NewReminder {
  title: string;
  triggers: Trigger[];
  /** A named time zone, or null for floating: the clock wherever the device is. */
  tz: string | null;
}

export interface InboxItem {
  section: "due" | "later_today" | "earlier_today";
  occurrence: Occurrence;
}

export const inbox = () => invoke<InboxItem[]>("inbox");
export const createOneOff = (title: string, dueAt: number) =>
  invoke<void>("create_one_off", { title, dueAt });
export const createReminder = (reminder: NewReminder) => invoke<void>("create_reminder", { reminder });
export const complete = (occurrenceId: string) => invoke<void>("complete", { occurrenceId });
/** Calls back whenever the core's state changed (a reminder fired, or an action was taken). */
export const onChanged = (f: () => void) => listen("changed", f);
/** Android only; empty elsewhere. Names: "notifications", "alarms". */
export const missingPermissions = () => invoke<string[]>("missing_permissions");
export const requestPermissions = () => invoke<void>("request_permissions");
