import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface DueItem {
  occurrence_id: string;
  title: string;
  scheduled_at: number;
  fired_at: number;
  /** The server hasn't received a change to it yet. */
  not_sent: boolean;
  snoozed_until: number | null;
  acknowledged: boolean;
}

export interface UpcomingItem {
  reminder_id: string;
  title: string;
  fire_at: number;
  not_sent: boolean;
}

/** Another device of this user signed in. */
export interface SignInNotice {
  id: string;
  device_id: string;
  device_name: string;
  /** Unix seconds. */
  at: number;
}

/** An action of one of this user's devices lost to a completion on another. */
export interface ReconciliationNotice {
  id: string;
  occurrence_id: string;
  device_id: string;
  device_name: string | null;
  /** "Your phone skipped “Bins”. It counts as completed." */
  text: string;
}

export interface Snapshot {
  due: DueItem[];
  upcoming: UpcomingItem[];
  sign_in_notices: SignInNotice[];
  reconciliations: ReconciliationNotice[];
  /** Set while the list holds changes from a newer app. */
  update_notice: string | null;
}

export const snapshot = () => invoke<Snapshot>("snapshot");
export const createReminder = (title: string, fireAt: number) =>
  invoke<void>("create_reminder", { title, fireAt });
export const completeOccurrence = (occurrenceId: string) =>
  invoke<void>("complete_occurrence", { occurrenceId });
export const skipOccurrence = (occurrenceId: string) =>
  invoke<void>("skip_occurrence", { occurrenceId });
export const dismissNotice = (id: string) => invoke<void>("dismiss_notice", { id });
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

/** What the sign-in screen learned about the server from what was typed. */
export interface SignInFound {
  address: string;
  fingerprint: string;
  name: string;
  version: string;
  /** The fingerprint came from a sign-in code, so it is already pinned. */
  fromCode: boolean;
}

export interface SignInArgs {
  address: string;
  fingerprint: string;
  serverName: string;
  username: string;
  password: string;
  portable: boolean;
}

export const readSignIn = (text: string) => invoke<SignInFound>("read_sign_in", { text });
export const signInServer = (args: SignInArgs) => invoke<Profile>("sign_in_server", { args });

/** Settings → Account: this server's sign-in code, as a link and a QR code (SVG). */
export interface SignInCode {
  link: string;
  svg: string;
}
export const signInCode = () => invoke<SignInCode>("sign_in_code");
