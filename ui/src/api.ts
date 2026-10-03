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
  /** When it was last acknowledged, in unix seconds. */
  acknowledged_at: number | null;
  priority: PriorityName;
  /** When it goes (or went) overdue, in unix seconds. */
  overdue_at: number;
}

export type PriorityName = "minimum" | "low" | "medium" | "high" | "maximum";
export type AlertStyle = "silent" | "gentle" | "insistent" | "alarm";

/** A built-in priority with every setting in the spec's table. Times in seconds. */
export interface PriorityInfo {
  priority: PriorityName;
  name: string;
  settings: {
    due_style: AlertStyle;
    due_interval: number;
    overdue_steps: Array<{ after: number; style: AlertStyle }>;
    overdue_interval: number;
    ring_duration: number | null;
    server_wait: number | null;
    swipeable: boolean;
    breaks_do_not_disturb: boolean;
  };
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

/** "5 failed sign-ins to your account", told by the server. */
export interface SecurityNotice {
  id: string;
  text: string;
  count: number;
  /** Unix seconds. */
  at: number;
}

export interface Snapshot {
  due: DueItem[];
  upcoming: UpcomingItem[];
  sign_in_notices: SignInNotice[];
  reconciliations: ReconciliationNotice[];
  security_notices: SecurityNotice[];
  /** Set while the list holds changes from a newer app. */
  update_notice: string | null;
}

export interface ExpectedItem {
  reminder_id: string;
  title: string;
  scheduled_at: number;
}

export interface EarlierItem {
  occurrence_id: string;
  title: string;
  scheduled_at: number;
  closed_at: number;
  kind: "missed" | "skipped" | "completed";
}

/** The Inbox's Later today and Earlier today. */
export interface Inbox {
  /** Highest priority first, then the longest overdue. */
  overdue: DueItem[];
  due: DueItem[];
  later_today: ExpectedItem[];
  earlier_today: EarlierItem[];
}

/** The patterns the editor offers; anything else is a written rule. */
export type Pattern =
  | { kind: "daily" }
  | { kind: "weekdays" }
  | { kind: "weekly"; days: string[] }
  | { kind: "monthly_by_date"; day: number }
  | { kind: "monthly_by_weekday"; ordinal: number; weekday: string };

export const snapshot = () => invoke<Snapshot>("snapshot");
export const createReminder = (title: string, fireAt: number, priority: PriorityName) =>
  invoke<void>("create_reminder", { title, fireAt, priority });
export const priorities = () => invoke<PriorityInfo[]>("priorities");
export const appVersion = () => invoke<string>("app_version");
export const inbox = () => invoke<Inbox>("inbox");
/** `date` is "2026-10-03", `time` "09:30". No `zone` makes it floating. */
export const createRecurringReminder = (
  title: string,
  pattern: Pattern,
  date: string,
  time: string,
  zone: string | null,
  priority: PriorityName,
) => invoke<void>("create_recurring_reminder", { title, pattern, date, time, zone, priority });
export const completeOccurrence = (occurrenceId: string) =>
  invoke<void>("complete_occurrence", { occurrenceId });
export const skipOccurrence = (occurrenceId: string, note?: string) =>
  invoke<void>("skip_occurrence", { occurrenceId, note });
/** Silences the current alert on all devices without closing the occurrence. */
export const acknowledgeOccurrence = (occurrenceId: string) =>
  invoke<void>("acknowledge_occurrence", { occurrenceId });
/** Snoozes for `minutes`, or for the priority's snooze length without them. */
export const snoozeOccurrence = (occurrenceId: string, minutes?: number) =>
  invoke<void>("snooze_occurrence", { occurrenceId, minutes });

/** What the alarm window shows. */
export interface OccurrenceView {
  occurrence_id: string;
  title: string;
  /** The list's name; null for the personal list. */
  list_name: string | null;
  priority: PriorityName;
  /** When it was due, in unix seconds. */
  scheduled_at: number;
  overdue_at: number;
  snoozed_until: number | null;
  acknowledged_at: number | null;
}
/** Null once the occurrence has closed. */
export const alarmView = (occurrenceId: string) =>
  invoke<OccurrenceView | null>("alarm_view", { occurrenceId });
export const dismissNotice = (id: string) => invoke<void>("dismiss_notice", { id });
export const onStateChanged = (f: () => void) => listen("state-changed", f);
/** A notification was clicked: the payload is its occurrence's id. */
export const onOpenOccurrence = (f: (occurrenceId: string) => void) =>
  listen<string>("open-occurrence", (e) => f(e.payload));

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
export const changePassword = (password: string) =>
  invoke<void>("change_password", { password });
export const keyStoreName = () => invoke<string>("key_store_name");

/** What the sign-in screen learned about the server from what was typed. */
export interface SignInFound {
  address: string;
  fingerprint: string;
  name: string;
  version: string;
  /** The fingerprint came from a sign-in code, so it is already pinned. */
  fromCode: boolean;
  /** The text was an existing device's approval link: no password is needed. */
  approvalLink: string | null;
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

/** One of the user's devices, for Settings → Account. */
export interface DeviceInfo {
  /** The server's id for it. */
  id: number;
  /** What it calls itself; null until its name has reached this device. */
  name: string | null;
  portable: boolean;
  /** When the server last heard from it, in Unix seconds. */
  last_synced: number | null;
  this_device: boolean;
}
export const listDevices = () => invoke<DeviceInfo[]>("list_devices");
/** Take another device off the account and rotate the keys of every list it held. */
export const removeDevice = (deviceId: number) => invoke<void>("remove_device", { deviceId });

/** A link and QR code (SVG) that one device shows for the other to scan. */
export interface ApprovalCode {
  link: string;
  svg: string;
}
/** The new device behind a code, for the user to confirm by name. */
export interface PendingDevice {
  name: string;
  portable: boolean;
}

/** New device: show a code for an existing device to scan. */
export const approvalShow = (address: string, fingerprint: string, serverName: string, portable: boolean) =>
  invoke<ApprovalCode>("approval_show", { address, fingerprint, serverName, portable });
/** New device: send a request to the existing device whose code was scanned or pasted. */
export const approvalScan = (link: string, portable: boolean) =>
  invoke<void>("approval_scan", { link, portable });
/** New device: has it been approved? If so it is signed in. */
export const finishApproval = () => invoke<Profile | null>("finish_approval");
export const cancelApproval = () => invoke<void>("cancel_approval");
/** Existing device: make a code for a new device to scan. */
export const offerApproval = () => invoke<ApprovalCode>("offer_approval");
/** Existing device: has the device behind this code asked yet? Its name, if so. */
export const pendingApproval = (link: string) =>
  invoke<PendingDevice | null>("pending_approval", { link });
/** Existing device: the user confirmed the device pendingApproval showed. */
export const approvePending = () => invoke<void>("approve_pending");
