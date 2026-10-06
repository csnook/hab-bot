import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export interface DueItem {
  occurrence_id: string;
  reminder_id: string;
  /** The list the reminder is in. */
  list_id: string;
  title: string;
  /** The reminder's note, shown on its occurrences. */
  note: string;
  scheduled_at: number;
  fired_at: number;
  /** The server hasn't received a change to it yet. */
  not_sent: boolean;
  snoozed_until: number | null;
  snoozed_at: number | null;
  acknowledged: boolean;
  /** When it was last acknowledged, in unix seconds. */
  acknowledged_at: number | null;
  priority: PriorityName;
  /** When it goes (or went) overdue, in unix seconds. */
  overdue_at: number;
  /** When it is missed for want of action, if it has such an expiry. */
  expires_at: number | null;
}

/**
 * A pause: set aside from `from` until `until` (unix seconds), or until it is
 * resumed when `until` is null. It sets things aside only while it covers the
 * time, so one that has run out is still sent but means nothing.
 */
export interface Pause {
  from: number;
  until: number | null;
}

/** What put an occurrence under a pause: the reminder's own, or its list's. */
export interface PauseCause {
  until: number | null;
  /** The pause is the list's, not the reminder's own. */
  list: boolean;
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
  list_id: string;
  priority: PriorityName;
  title: string;
  note: string;
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

export type CountdownUnit = "minutes" | "hours" | "days" | "weeks";

/** Fires `amount` `unit`s after the last occurrence closed; `at` is a time of day ("09:00") for days and weeks. */
export interface Countdown {
  amount: number;
  unit: CountdownUnit;
  at: string | null;
}

/** A countdown reminder and when it fires next (null while an occurrence is open). */
export interface CountdownItem {
  reminder_id: string;
  list_id: string;
  priority: PriorityName;
  title: string;
  countdown: Countdown;
  next_at: number | null;
}

export interface Snapshot {
  due: DueItem[];
  upcoming: UpcomingItem[];
  countdowns: CountdownItem[];
  sign_in_notices: SignInNotice[];
  reconciliations: ReconciliationNotice[];
  security_notices: SecurityNotice[];
  /** Set while the list holds changes from a newer app. */
  update_notice: string | null;
}

export interface ExpectedItem {
  reminder_id: string;
  list_id: string;
  priority: PriorityName;
  title: string;
  note: string;
  scheduled_at: number;
  /** Snoozed ahead of time until then: it fires at its time, quietly. */
  snoozed_until: number | null;
  expires_at: number | null;
  /** Complete early and Skip ahead are offered: it is the next one expected. */
  can_close_early: boolean;
}

/** An open occurrence of a paused reminder: quiet, and not in Overdue or Due. */
export interface PausedOpen {
  item: DueItem;
  pause: PauseCause;
}

/** A reminder paused now, for the Board's Paused column. */
export interface PausedReminder {
  reminder_id: string;
  list_id: string;
  title: string;
  priority: PriorityName;
  pause: PauseCause;
  /** When that pause began, in unix seconds. */
  from: number;
  /** It has an open occurrence, so its card stays in Overdue or Due. */
  has_open: boolean;
}

export interface EarlierItem {
  occurrence_id: string;
  list_id: string;
  priority: PriorityName;
  title: string;
  scheduled_at: number;
  closed_at: number;
  kind: "missed" | "skipped" | "completed";
  /** A completion or skip can be undone; a miss is corrected instead. */
  can_undo: boolean;
  /** How it was closed was changed afterwards. */
  corrected: boolean;
  /** Skipped because it fell in a pause. */
  paused: PauseCause | null;
}

export type Outcome = "done_on_time" | "done_late" | "skipped" | "missed";

/** One closing, correction or undo in a closed occurrence's history. */
export interface ClosedEntry {
  event_id: string;
  what: "completed" | "skipped" | "missed" | "reopened" | "expected";
  /** The time the user said it was done or skipped. */
  at: number | null;
  note: string | null;
  by: string;
  /** When it was tapped. */
  tapped_at: number;
  /** When the server received it, once it has. */
  received_at: number | null;
  /** A later correction or undo took its place: the original, kept. */
  superseded: boolean;
  /** A skip the pause made: the history attributes it to the pause. */
  paused: PauseCause | null;
}

/** A closed occurrence as the details panel shows it. */
export interface ClosedView {
  occurrence_id: string;
  reminder_id: string;
  list_id: string;
  title: string;
  priority: PriorityName;
  scheduled_at: number;
  kind: "missed" | "skipped" | "completed";
  outcome: Outcome;
  at: number;
  note: string | null;
  can_undo: boolean;
  corrected: boolean;
  /** Skipped because it fell in a pause. */
  paused: PauseCause | null;
  history: ClosedEntry[];
}

/** How an undo left the occurrence. */
export type UndoOutcome =
  | { outcome: "reopened" }
  | { outcome: "expected" }
  | { outcome: "missed"; at: number };

/** The Inbox's Later today and Earlier today. */
export interface Inbox {
  /** Highest priority first, then the longest overdue. */
  overdue: DueItem[];
  due: DueItem[];
  later_today: ExpectedItem[];
  earlier_today: EarlierItem[];
  /** Open occurrences of reminders paused now: set aside, quiet. */
  paused: PausedOpen[];
}

/** The patterns the editor offers; anything else is a written rule. */
export type Pattern =
  | { kind: "daily" }
  | { kind: "weekdays" }
  | { kind: "weekly"; days: string[] }
  | { kind: "monthly_by_date"; day: number }
  | { kind: "monthly_by_weekday"; ordinal: number; weekday: string };

export const snapshot = () => invoke<Snapshot>("snapshot");
/** The create calls return the new reminder's id. */
export const createReminder = (
  title: string,
  fireAt: number,
  priority: PriorityName,
  listId: string | null,
) => invoke<string>("create_reminder", { title, fireAt, priority, listId });
export const priorities = () => invoke<PriorityInfo[]>("priorities");
export const appVersion = () => invoke<string>("app_version");
export const inbox = () => invoke<Inbox>("inbox");
/** `date` is "2026-10-03", `time` "09:30". No `zone` makes it floating. */
export const createRecurringReminder = (
  title: string,
  schedule: { pattern: Pattern; date: string; time: string } | null,
  suns: SunTrigger[],
  conditions: Condition[],
  zone: string | null,
  priority: PriorityName,
  listId: string | null,
) =>
  invoke<string>("create_recurring_reminder", {
    title,
    pattern: schedule?.pattern ?? null,
    date: schedule?.date ?? null,
    time: schedule?.time ?? null,
    suns,
    conditions,
    zone,
    priority,
    listId,
  });

/** A moment in the Sun's day. */
export type SunEvent = "sunrise" | "sunset" | "civil_dawn" | "civil_dusk";

/** A sun event and an offset from it: negative minutes are before it. */
export interface SunTrigger {
  event: SunEvent;
  offset_minutes: number;
}

/** A time-based condition. `days` are MO..SU, times "HH:MM", dates "2026-12-20", seasons "06-01". */
export type Condition =
  | { kind: "days"; days: string[] }
  | { kind: "window"; from: string; to: string }
  | { kind: "dates"; from: string; to: string }
  | { kind: "season"; from: string; to: string }
  | { kind: "daylight" }
  | { kind: "darkness" };

/** The user's Home place: a centre and a radius. */
export interface Place {
  name: string;
  latitude: number;
  longitude: number;
  radius_metres: number;
}

/** The home location, or null if none is set. */
export const homeLocation = () => invoke<Place | null>("home_location");
/** Sets the home location in degrees, north and east positive. */
export const setHomeLocation = (latitude: number, longitude: number) =>
  invoke<void>("set_home_location", { latitude, longitude });
export const clearHomeLocation = () => invoke<void>("clear_home_location");
/**
 * Creates a reminder that fires a set time after it was last done. `lastDone`
 * is unix seconds, or null for never (it fires at once). No `zone` makes a
 * time of day floating.
 */
export const createCountdownReminder = (
  title: string,
  countdown: Countdown,
  lastDone: number | null,
  zone: string | null,
  priority: PriorityName,
  listId: string | null,
) =>
  invoke<string>("create_countdown_reminder", {
    title,
    amount: countdown.amount,
    unit: countdown.unit,
    at: countdown.at,
    lastDone,
    zone,
    priority,
    listId,
  });
/**
 * How an overdue time or an expiry is counted from the scheduled time: a
 * duration, or the next time a pattern matches ("the next 1st at 00:00").
 * `other` is a hand-written schedule the editor shows and leaves alone.
 */
export type DelaySpec =
  | { kind: "after"; seconds: number }
  | { kind: "next"; pattern: Pattern; time: string }
  | { kind: "other"; rule: string };

/** A schedule as the editor can show it, if it is one of its patterns. */
export interface ScheduleParts {
  pattern: Pattern;
  /** The first date, "2026-10-03". */
  date: string;
  /** The time of day, "09:30". */
  time: string;
}

export type TriggerView =
  | { kind: "one_off"; fire_at: number }
  | {
      kind: "schedules";
      schedules: Array<{ parts: ScheduleParts | null; start: string; rule: string }>;
      /** Sun events it also fires at. */
      suns: SunTrigger[];
    }
  | { kind: "countdown"; countdown: Countdown };

/** A reminder as the editor shows it. */
export interface ReminderView {
  reminder_id: string;
  list_id: string;
  /** The list's name; null for the personal list. */
  list_name: string | null;
  /** The list's colour, "#rrggbb", if it has one. */
  list_colour: string | null;
  title: string;
  note: string;
  priority: PriorityName;
  trigger: TriggerView;
  /** The zone it is pinned to; null is floating. */
  zone: string | null;
  /** Seconds after the scheduled time it goes overdue by its priority. */
  default_overdue_seconds: number;
  /** The overdue override; null follows the priority. */
  overdue: DelaySpec | null;
  /** The expiries added; firing again always expires it too. */
  expiries: DelaySpec[];
  /** Its own pause, if it was paused and not resumed. */
  pause: Pause | null;
  /** Its list's pause, which also sets it aside. */
  list_pause: Pause | null;
  /** The time-based conditions it fires only within. */
  conditions: Condition[];
  /** It uses a sun event or daylight and no home location is set. */
  needs_home: boolean;
}

/** What the editor changed; anything left out stays as it is. */
export interface EditArgs {
  title?: string;
  note?: string;
  priority?: PriorityName;
  fire_at?: number;
  /** Replaces a schedule reminder's schedules with this one. */
  schedule?: { pattern: Pattern; date: string; time: string };
  /** Pins it to this zone; `floating` unpins it. */
  zone?: string;
  floating?: boolean;
  countdown?: Countdown;
  overdue?: DelaySpec;
  /** Go back to following the priority's overdue time. */
  follow_priority?: boolean;
  /** All the expiries, replacing the reminder's. */
  expiry?: DelaySpec[];
  /** All the sun events, replacing the reminder's. */
  suns?: SunTrigger[];
  /** All the conditions, replacing the reminder's. */
  conditions?: Condition[];
}
export const reminderView = (reminderId: string) =>
  invoke<ReminderView>("reminder_view", { reminderId });
export const editReminder = (reminderId: string, edit: EditArgs) =>
  invoke<void>("edit_reminder", { reminderId, edit });

/**
 * Completes the reminder's next expected occurrence before it fires, as done
 * at `doneAt` (now by default): it never fires, and a countdown restarts from
 * then. Returns the occurrence, to undo.
 */
export const completeEarly = (reminderId: string, doneAt?: number) =>
  invoke<string>("complete_early", { reminderId, doneAt });
/** Skips the next expected occurrence ahead of time, with an optional note. */
export const skipAhead = (reminderId: string, note?: string) =>
  invoke<string>("skip_ahead", { reminderId, note });
/** Undoes a completion or skip: reopened, expected again, or missed. */
export const undoOccurrence = (occurrenceId: string) =>
  invoke<UndoOutcome>("undo_occurrence", { occurrenceId });
/** Changes how a closed occurrence, a missed one too, was closed. */
export const correctOccurrence = (
  occurrenceId: string,
  kind: "completed" | "skipped",
  at: number,
  note?: string,
) => invoke<void>("correct_occurrence", { occurrenceId, kind, at, note });
export const closedOccurrence = (occurrenceId: string) =>
  invoke<ClosedView | null>("closed_occurrence", { occurrenceId });
/** Notes given when skipping, most recent first. */
export const recentSkipNotes = () => invoke<string[]>("recent_skip_notes");
/** `doneAt` is when it was done, if not just now: a countdown restarts from it. */
export const completeOccurrence = (occurrenceId: string, doneAt?: number) =>
  invoke<void>("complete_occurrence", { occurrenceId, doneAt });
export const skipOccurrence = (occurrenceId: string, note?: string) =>
  invoke<void>("skip_occurrence", { occurrenceId, note });
/** Silences the current alert on all devices without closing the occurrence. */
export const acknowledgeOccurrence = (occurrenceId: string) =>
  invoke<void>("acknowledge_occurrence", { occurrenceId });
/**
 * Snoozes for `minutes` or until `until` (unix seconds), or for the
 * priority's snooze length without either.
 */
export const snoozeOccurrence = (occurrenceId: string, minutes?: number, until?: number) =>
  invoke<void>("snooze_occurrence", { occurrenceId, minutes, until });
/** Snoozes an expected occurrence ahead of time. */
export const snoozeExpected = (reminderId: string, scheduledAt: number, until: number) =>
  invoke<void>("snooze_expected", { reminderId, scheduledAt, until });

export type SnoozeKind = "interval" | "hour" | "tomorrow_morning";
/** One of the menu's timed choices. */
export interface SnoozeOption {
  kind: SnoozeKind;
  /** When it ends, in unix seconds. */
  until: number;
  /** The length of an interval or an hour, in seconds. */
  seconds: number | null;
  /** A last-chance alert would come inside it, before the expiry. */
  last_chance: boolean;
}
/** What the snooze menu shows. */
export interface SnoozePicker {
  options: SnoozeOption[];
  /** "Expires at 23:59", if the occurrence has a known expiry. */
  expires_at: number | null;
  /** When the last-chance alert comes before it. */
  last_chance_at: number | null;
  /** An expected occurrence: the choices count from `from`, when it fires. */
  ahead: boolean;
  from: number;
}
export const snoozePicker = (occurrenceId: string) =>
  invoke<SnoozePicker>("snooze_picker", { occurrenceId });
export const snoozePickerExpected = (reminderId: string, scheduledAt: number) =>
  invoke<SnoozePicker>("snooze_picker_expected", { reminderId, scheduledAt });

/** One snooze in the history: what it was set to end on and how it ended. */
export interface SnoozeView {
  occurrence_id: string;
  set_at: number;
  until: number;
  ahead: boolean;
  ended_at: number | null;
  ended: "elapsed" | "replaced" | "closed" | null;
}
export const snoozeHistory = (occurrenceId: string) =>
  invoke<SnoozeView[]>("snooze_history", { occurrenceId });

/** What the alarm window shows. */
export interface OccurrenceView {
  occurrence_id: string;
  title: string;
  note: string;
  /** The list's name; null for the personal list. */
  list_name: string | null;
  priority: PriorityName;
  /** When it was due, in unix seconds. */
  scheduled_at: number;
  overdue_at: number;
  snoozed_until: number | null;
  acknowledged_at: number | null;
  expires_at: number | null;
}
/** Null once the occurrence has closed. */
export const alarmView = (occurrenceId: string) =>
  invoke<OccurrenceView | null>("alarm_view", { occurrenceId });
/** A reminder list this device holds. */
export interface ListInfo {
  id: string;
  /** The personal list has no name of its own. */
  name: string | null;
  /** "#rrggbb", if it has been given one. */
  colour: string | null;
  /** The account's personal list: the default, and it can't be deleted. */
  personal: boolean;
  /** How many reminders it holds, not counting deleted ones. */
  reminders: number;
  /** The list's pause, if it was paused and not resumed. */
  pause: Pause | null;
}

/** A reminder that was deleted with its history kept. */
export interface DeletedReminder {
  reminder_id: string;
  list_id: string;
  title: string;
  /** Unix seconds. */
  deleted_at: number;
  deleted_by: string;
}

/**
 * What the sidebar's checkboxes hide, remembered on this device. What is
 * hidden is listed, so a list or priority made later shows. Alerts never
 * look at it.
 */
export interface Filters {
  hidden_lists: string[];
  hidden_priorities: PriorityName[];
}

export const lists = () => invoke<ListInfo[]>("lists");
export const createList = (name: string, colour: string | null) =>
  invoke<string>("create_list", { name, colour });
export const renameList = (listId: string, name: string) =>
  invoke<void>("rename_list", { listId, name });
export const colourList = (listId: string, colour: string) =>
  invoke<void>("colour_list", { listId, colour });
/** Only an empty list that isn't the personal one. */
export const deleteList = (listId: string) => invoke<void>("delete_list", { listId });
/** Moves a reminder to another list, keeping its history. */
export const moveReminder = (reminderId: string, listId: string) =>
  invoke<void>("move_reminder", { reminderId, listId });
/** `withHistory` forgets everything about it; otherwise its history is kept, marked deleted. */
export const deleteReminder = (reminderId: string, withHistory: boolean) =>
  invoke<void>("delete_reminder", { reminderId, withHistory });
export const deletedReminders = () => invoke<DeletedReminder[]>("deleted_reminders");
/** Pauses a reminder from now until `until` (unix seconds), or until resumed when null. */
export const pauseReminder = (reminderId: string, until: number | null) =>
  invoke<void>("pause_reminder", { reminderId, until });
export const resumeReminder = (reminderId: string) =>
  invoke<void>("resume_reminder", { reminderId });
/** Pauses every reminder in a list. */
export const pauseList = (listId: string, until: number | null) =>
  invoke<void>("pause_list", { listId, until });
export const resumeList = (listId: string) => invoke<void>("resume_list", { listId });
/** The reminders paused now (the Board's Paused column). */
export const pausedReminders = () => invoke<PausedReminder[]>("paused_reminders");
export const filters = () => invoke<Filters>("filters");
export const setFilters = (f: Filters) => invoke<void>("set_filters", { filters: f });

export const dismissNotice = (id: string) => invoke<void>("dismiss_notice", { id });
export const onStateChanged = (f: () => void) => listen("state-changed", f);
/** A notification was clicked: the payload is its occurrence's id. */
export const onOpenOccurrence = (f: (occurrenceId: string) => void) =>
  listen<string>("open-occurrence", (e) => f(e.payload));

/** The tray's Settings entry (or a second `--settings` launch): open Settings. */
export const onOpenSettings = (f: () => void) => listen("open-settings", f);

/** Settings → This device: whether the app starts at login (an XDG autostart entry). */
export const autostartEnabled = () => invoke<boolean>("autostart_enabled");
export const setAutostart = (enabled: boolean) => invoke<void>("set_autostart", { enabled });

/** "Quiet this device until…": silent here until then. Never synced; not a snooze. */
export interface DeviceQuiet {
  until: number;
  include_maximum: boolean;
}

/** The loudest alert style this device uses; Maximum is exempt unless `caps_maximum`. */
export interface LoudestAlert {
  style: AlertStyle;
  caps_maximum: boolean;
}

/** Settings → This device. The name and portable are the user's, the rest stays here. */
export interface ThisDevice {
  name: string;
  portable: boolean;
  loudest: LoudestAlert;
  quiet: DeviceQuiet | null;
}

export const thisDevice = () => invoke<ThisDevice>("this_device");
export const setDeviceName = (name: string) => invoke<void>("set_device_name", { name });
export const setDevicePortable = (portable: boolean) =>
  invoke<void>("set_device_portable", { portable });
export const setLoudestAlert = (style: AlertStyle, capsMaximum: boolean) =>
  invoke<void>("set_loudest_alert", { style, capsMaximum });
/** Quiet this device until `until` (unix seconds). Other devices still alert. */
export const quietDevice = (until: number, includeMaximum: boolean) =>
  invoke<void>("quiet_device", { until, includeMaximum });
export const endQuietDevice = () => invoke<void>("end_quiet_device");
/** The tray's "Quiet this device… More choices…": open Settings → This device. */
export const onOpenThisDevice = (f: () => void) => listen("open-this-device", f);

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

/** What a snooze-all or quiet hours cover: everything, or one list. */
export type Scope = { kind: "all" } | { kind: "list"; list_id: string };

/** A snooze-all or a stretch of quiet hours holding alerts now. */
export interface SnoozeAllView {
  /** What ends a snooze-all early; quiet hours have none. */
  id: string | null;
  source: "snooze_all" | "quiet_hours";
  scope: Scope;
  /** The list's name for a list scope. */
  list_name: string | null;
  include_maximum: boolean;
  from: number;
  /** When alerts resume, in unix seconds. */
  until: number;
}

/**
 * Quiet hours: a snooze-all that recurs. `days` are the days a stretch starts
 * on (`MO`..`SU`); `from` and `to` are times of day (`22:00`, `07:00`), and a
 * `to` that is not later than `from` runs into the next morning.
 */
export interface QuietHours {
  days: string[];
  from: string;
  to: string;
  scope: Scope;
  include_maximum: boolean;
}

/** Snoozes everything (or one list) until `until`, in unix seconds. Returns its id. */
export const snoozeAll = (listId: string | null, until: number, includeMaximum: boolean) =>
  invoke<string>("snooze_all", { listId, until, includeMaximum });
export const endSnoozeAll = (id: string) => invoke<void>("end_snooze_all", { id });
/** What holds alerts now: snooze-alls not yet over, and quiet hours in progress. */
export const holding = () => invoke<SnoozeAllView[]>("holding");
export const quietHours = () => invoke<QuietHours[]>("quiet_hours");
export const setQuietHours = (rules: QuietHours[]) => invoke<void>("set_quiet_hours", { rules });
/** The tray's Snooze all, More choices…: open the Snooze all dialog. */
export const onOpenSnoozeAll = (f: () => void) => listen("open-snooze-all", f);
