import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface DueItem {
  occurrence_id: string;
  title: string;
  scheduled_at: number;
  fired_at: number;
  /** The server hasn't received a change to it yet. */
  not_sent: boolean;
}

export interface UpcomingItem {
  reminder_id: string;
  title: string;
  fire_at: number;
  not_sent: boolean;
}

export interface Snapshot {
  due: DueItem[];
  upcoming: UpcomingItem[];
  /** Set while the list holds changes from a newer app. */
  update_notice: string | null;
}

export const snapshot = () => invoke<Snapshot>("snapshot");
export const createReminder = (title: string, fireAt: number) =>
  invoke<void>("create_reminder", { title, fireAt });
export const completeOccurrence = (occurrenceId: string) =>
  invoke<void>("complete_occurrence", { occurrenceId });
export const onStateChanged = (f: () => void) => listen("state-changed", f);

export interface Profile {
  server_address: string;
  server_fingerprint: string;
  server_name: string;
  username: string;
  display_name: string;
  admin: boolean;
  account_id: number;
  device_id: number;
  device_name: string;
  portable: boolean;
}

/** How this device is set up. null means the first start has not happened. */
export type Setup = { mode: "standalone" } | ({ mode: "joined" } & Profile);

export interface FoundServer {
  fingerprint: string;
  name: string;
  version: string;
}

export interface PasswordCheck {
  score: number;
  ok: boolean;
  warning: string | null;
  suggestions: string[];
}

export interface JoinArgs {
  address: string;
  fingerprint: string;
  serverName: string;
  setupCode: string;
  username: string;
  displayName: string;
  password: string;
  portable: boolean;
}

export const setupState = () => invoke<Setup | null>("setup_state");
export const chooseStandalone = () => invoke<void>("choose_standalone");
export const probeServer = (address: string) =>
  invoke<FoundServer>("probe_server", { address });
export const passwordCheck = (password: string, username: string, displayName: string) =>
  invoke<PasswordCheck>("password_check", { password, username, displayName });
export const passphraseSuggestion = () => invoke<string>("passphrase_suggestion");
export const joinServer = (args: JoinArgs) => invoke<Profile>("join_server", { args });
export const keyStoreName = () => invoke<string>("key_store_name");
