import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface DueItem {
  occurrence_id: string;
  title: string;
  scheduled_at: number;
  fired_at: number;
}

export interface UpcomingItem {
  reminder_id: string;
  title: string;
  fire_at: number;
}

export interface Snapshot {
  due: DueItem[];
  upcoming: UpcomingItem[];
}

export const snapshot = () => invoke<Snapshot>("snapshot");
export const createReminder = (title: string, fireAt: number) =>
  invoke<void>("create_reminder", { title, fireAt });
export const completeOccurrence = (occurrenceId: string) =>
  invoke<void>("complete_occurrence", { occurrenceId });
export const onStateChanged = (f: () => void) => listen("state-changed", f);
