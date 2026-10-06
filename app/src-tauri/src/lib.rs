//! The Linux desktop app: a Tauri 2 shell around `hab-core`.
//!
//! The scheduler runs in a Rust thread, not in the window, so reminders fire
//! with the window closed. Closing the window hides it; Quit in the tray exits.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hab_client::{
    check_password, join, parse_target, sign_in, suggest_passphrase, tls, ApprovalLink, Checks,
    DeviceInfo, JoinRequest, KeyStore, NewDevice, PasswordCheck, PendingDevice, Pinned, Profile,
    Setup, SetupFile, SignInCode, SignInRequest, SignInTarget, Syncer,
};
use hab_core::{Alerter, Core, Snapshot};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tokio::sync::{watch, Notify};

/// How long the scheduler sleeps at most, so it notices clock changes and
/// resumes from suspend promptly.
const MAX_SLEEP: Duration = Duration::from_secs(30);
const STATE_CHANGED: &str = "state-changed";
/// Sent to the window when a notification is clicked; the payload is the
/// occurrence's id.
const OPEN_OCCURRENCE: &str = "open-occurrence";
const OPEN_SETTINGS: &str = "open-settings";
const OPEN_SNOOZE_ALL: &str = "open-snooze-all";
/// Sent to the window to open Settings at This device (the tray's "More choices…").
const OPEN_THIS_DEVICE: &str = "open-this-device";

mod alarm;
mod autostart;
mod badge;
mod launch;
mod notify;
mod sound;
mod this_device;
mod tray;
mod tray_model;
mod wayland;
use notify::{Delivery, UserAction};
use tray::Tray;
use tray_model::TrayAction;

/// What the alarm windows need to tell the alarm they were closed.
struct AlarmState {
    delivery: Arc<Delivery>,
    registry: Arc<alarm::Registry>,
}

struct App {
    core: Arc<Mutex<Core>>,
    wake: Sender<()>,
    /// How the scheduler's alerts check with the server (see
    /// `hab_core::ServerCheck`); the sync loop answers.
    checks: Arc<Checks>,
    /// Tells the sync loop there is a change to send.
    sync_wake: Arc<Notify>,
    /// Stops the sync loop; set when the app quits.
    sync_stop: watch::Sender<bool>,
    /// Where `setup.json` and the key file live.
    data_dir: PathBuf,
    /// How this device is set up; None until the first-start choice is made.
    setup: Mutex<Option<Setup>>,
    /// The running sync, once there is one: Settings → Account asks it for
    /// the device list and has it remove a device.
    syncer: SyncerSlot,
    /// This device, while it waits for another of the user's devices to
    /// approve it (Sign in → approve from another device).
    waiting: tokio::sync::Mutex<Option<Arc<NewDevice>>>,
    /// The new device the user is looking at on an existing device, to
    /// confirm it by name before approving.
    pending: Mutex<Option<PendingDevice>>,
}

type SyncerSlot = Arc<Mutex<Option<Arc<Syncer>>>>;

/// What the sync loop shares with the rest of the app.
struct SyncShared {
    core: Arc<Mutex<Core>>,
    wake: Arc<Notify>,
    /// Wakes the scheduler, so what the server delivers takes effect at once
    /// rather than at its next look.
    scheduler: Sender<()>,
    checks: Arc<Checks>,
    stop: watch::Receiver<bool>,
    slot: SyncerSlot,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[tauri::command]
fn snapshot(app: tauri::State<'_, App>) -> Snapshot {
    app.core.lock().unwrap().snapshot()
}

/// Gives a reminder just made the priority the editor chose.
fn set_priority(
    core: &mut Core,
    id: &str,
    priority: Option<hab_core::Priority>,
) -> Result<(), String> {
    let edit = hab_core::EditReminder {
        priority,
        ..Default::default()
    };
    core.edit_reminder(id, edit, now())
        .map_err(|e| e.to_string())
}

/// A reminder as the editor shows it.
#[tauri::command]
fn reminder_view(
    app: tauri::State<'_, App>,
    reminder_id: String,
) -> Result<hab_core::ReminderView, String> {
    let core = app.core.lock().unwrap();
    core.reminder_view(&reminder_id).map_err(|e| e.to_string())
}

/// What the editor changed; anything left out stays as it is.
#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct EditArgs {
    title: Option<String>,
    note: Option<String>,
    priority: Option<hab_core::Priority>,
    /// A one-off reminder's time.
    fire_at: Option<i64>,
    /// Replaces a schedule reminder's schedules with this one.
    schedule: Option<SchedulePick>,
    /// Pins the reminder to this time zone; `floating` unpins it.
    zone: Option<String>,
    floating: bool,
    countdown: Option<hab_core::Countdown>,
    /// The overdue override, or with `follow_priority` none.
    overdue: Option<hab_core::DelaySpec>,
    follow_priority: bool,
    /// All the expiries, replacing the reminder's.
    expiry: Option<Vec<hab_core::DelaySpec>>,
    /// All the sun-event triggers, replacing the reminder's.
    suns: Option<Vec<hab_core::SunTrigger>>,
    /// All the time-based conditions, replacing the reminder's.
    conditions: Option<Vec<hab_core::Condition>>,
}

#[derive(serde::Deserialize)]
struct SchedulePick {
    pattern: hab_core::Pattern,
    date: String,
    time: String,
}

/// Applies the editor's changes to a reminder.
#[tauri::command]
fn edit_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    edit: EditArgs,
) -> Result<(), String> {
    let delay = |d: &hab_core::DelaySpec| d.to_delay();
    let overdue = if edit.follow_priority {
        Some(None)
    } else {
        match &edit.overdue {
            Some(d) => Some(Some(delay(d)?)),
            None => None,
        }
    };
    let expiry = match &edit.expiry {
        Some(v) => Some(v.iter().map(delay).collect::<Result<Vec<_>, _>>()?),
        None => None,
    };
    let schedules = match &edit.schedule {
        Some(p) => Some(vec![hab_core::Schedule::from_pattern(
            &p.pattern, &p.date, &p.time,
        )?]),
        None => None,
    };
    let zone = if edit.floating {
        Some(None)
    } else {
        edit.zone.clone().map(Some)
    };
    {
        let mut core = app.core.lock().unwrap();
        let _ = core.use_system_zone();
        core.edit_reminder(
            &reminder_id,
            hab_core::EditReminder {
                title: edit.title,
                fire_at: edit.fire_at,
                note: edit.note,
                schedules,
                zone,
                priority: edit.priority,
                overdue,
                expiry,
                countdown: edit.countdown,
                suns: edit.suns,
                conditions: edit.conditions,
            },
            now(),
        )
        .map_err(|e| e.to_string())?;
    }
    changed(&app, &handle);
    Ok(())
}

/// The built-in priorities, for the editor and Settings → Priorities.
#[tauri::command]
fn priorities() -> Vec<hab_core::PriorityInfo> {
    hab_core::built_in_priorities()
}

/// The app's version, for Settings → About.
#[tauri::command]
fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

#[tauri::command]
fn create_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    title: String,
    fire_at: i64,
    priority: Option<hab_core::Priority>,
    list_id: Option<String>,
) -> Result<String, String> {
    let new_id = {
        let mut core = app.core.lock().unwrap();
        let list = list_or_personal(&core, list_id);
        let id = core
            .create_reminder_in(&list, &title, fire_at, now())
            .map_err(|e| e.to_string())?;
        set_priority(&mut core, &id, priority)?;
        id
    };
    app.sync_wake.notify_one();
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(new_id)
}

/// A reminder that repeats: a common pattern from a date, at a time of day,
/// and/or at sun events, only where the conditions hold, pinned to a time
/// zone or, with none, floating. It needs a pattern or a sun event.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn create_recurring_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    title: String,
    pattern: Option<hab_core::Pattern>,
    date: Option<String>,
    time: Option<String>,
    suns: Option<Vec<hab_core::SunTrigger>>,
    conditions: Option<Vec<hab_core::Condition>>,
    zone: Option<String>,
    priority: Option<hab_core::Priority>,
    list_id: Option<String>,
) -> Result<String, String> {
    let schedules = match (&pattern, &date, &time) {
        (Some(p), Some(d), Some(t)) => vec![hab_core::Schedule::from_pattern(p, d, t)?],
        (None, _, _) => Vec::new(),
        _ => return Err("a schedule needs a date and a time".into()),
    };
    let new_id = {
        let mut core = app.core.lock().unwrap();
        let _ = core.use_system_zone();
        let list = list_or_personal(&core, list_id);
        let id = core
            .create_repeating_reminder_in(
                &list,
                &title,
                schedules,
                suns.unwrap_or_default(),
                conditions.unwrap_or_default(),
                zone.as_deref(),
                now(),
            )
            .map_err(|e| e.to_string())?;
        set_priority(&mut core, &id, priority)?;
        id
    };
    app.sync_wake.notify_one();
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(new_id)
}

/// The user's home location (their Home place), if they have set one.
#[tauri::command]
fn home_location(app: tauri::State<'_, App>) -> Option<hab_core::Place> {
    app.core.lock().unwrap().home()
}

/// Sets the home location, in degrees north and east. It syncs to the user's
/// other devices as a personal setting.
#[tauri::command]
fn set_home_location(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    latitude: f64,
    longitude: f64,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_home(latitude, longitude, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Clears the home location.
#[tauri::command]
fn clear_home_location(app: tauri::State<'_, App>, handle: AppHandle) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .clear_home(now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Snoozes all of the user's reminders, or one list's, until `until` (unix
/// seconds). Maximum is left out unless `include_maximum`. A personal setting:
/// it holds on all of the user's devices. Returns its id.
#[tauri::command]
fn snooze_all(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    list_id: Option<String>,
    until: i64,
    include_maximum: bool,
) -> Result<String, String> {
    let scope = list_id.map_or(hab_core::Scope::All, hab_core::Scope::List);
    let id = app
        .core
        .lock()
        .unwrap()
        .snooze_all(scope, until, include_maximum, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(id)
}

/// Ends a snooze-all early: what it held back alerts at its current level.
#[tauri::command]
fn end_snooze_all(app: tauri::State<'_, App>, handle: AppHandle, id: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .end_snooze_all(&id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// What holds alerts now: snooze-alls not yet over, and quiet hours in
/// progress.
#[tauri::command]
fn holding(app: tauri::State<'_, App>) -> Vec<hab_core::SnoozeAllView> {
    app.core.lock().unwrap().holding(now())
}

/// The user's quiet hours.
#[tauri::command]
fn quiet_hours(app: tauri::State<'_, App>) -> Vec<hab_core::QuietHours> {
    app.core.lock().unwrap().quiet_hours()
}

/// Sets the user's quiet hours (all the rules; none clears them). A personal
/// setting: it holds on all of the user's devices.
#[tauri::command]
fn set_quiet_hours(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    rules: Vec<hab_core::QuietHours>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_quiet_hours(rules, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// A reminder that fires a set time after its last occurrence closed.
/// `last_done` is when it was last done in Unix seconds (the window sends now
/// by default); none means never, and it fires at once. `at` is a time of day
/// for countdowns of days or weeks.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn create_countdown_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    title: String,
    amount: u32,
    unit: hab_core::CountdownUnit,
    at: Option<String>,
    last_done: Option<i64>,
    zone: Option<String>,
    priority: Option<hab_core::Priority>,
    list_id: Option<String>,
) -> Result<String, String> {
    let new_id = {
        let mut core = app.core.lock().unwrap();
        let _ = core.use_system_zone();
        let list = list_or_personal(&core, list_id);
        let countdown = hab_core::Countdown { amount, unit, at };
        let id = core
            .create_countdown_reminder_in(
                &list,
                &title,
                countdown,
                zone.as_deref(),
                last_done,
                now(),
            )
            .map_err(|e| e.to_string())?;
        set_priority(&mut core, &id, priority)?;
        id
    };
    changed(&app, &handle);
    Ok(new_id)
}

/// Completes a reminder's next expected occurrence before it fires, as done
/// at `done_at` (now if none): it never fires, and a countdown restarts from
/// then. Returns the occurrence, for Undo.
#[tauri::command]
fn complete_early(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    done_at: Option<i64>,
) -> Result<String, String> {
    let now = now();
    let id = app
        .core
        .lock()
        .unwrap()
        .complete_expected(&reminder_id, done_at.unwrap_or(now), now)
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(id)
}

/// Skips a reminder's next expected occurrence ahead of time, with an
/// optional note. Returns the occurrence, for Undo.
#[tauri::command]
fn skip_ahead(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    note: Option<String>,
) -> Result<String, String> {
    let id = app
        .core
        .lock()
        .unwrap()
        .skip_expected(&reminder_id, note.as_deref(), now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(id)
}

/// Undoes a completion or skip: reopened if it would still be open, expected
/// again if it was closed ahead of its time, otherwise missed.
#[tauri::command]
fn undo_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
) -> Result<hab_core::UndoOutcome, String> {
    let outcome = app
        .core
        .lock()
        .unwrap()
        .undo(&occurrence_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(outcome)
}

/// Changes how a closed occurrence, a missed one too, was closed. `kind` is
/// "completed" or "skipped"; `at` is when it was done (unix seconds).
#[tauri::command]
fn correct_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
    kind: hab_core::Correction,
    at: i64,
    note: Option<String>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .correct(&occurrence_id, kind, at, note.as_deref(), now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// A closed occurrence with its history, for the details panel.
#[tauri::command]
fn closed_occurrence(
    app: tauri::State<'_, App>,
    occurrence_id: String,
) -> Option<hab_core::ClosedView> {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.closed_occurrence(&occurrence_id)
}

/// Notes given when skipping, most recent first, to offer again.
#[tauri::command]
fn recent_skip_notes(app: tauri::State<'_, App>) -> Vec<String> {
    app.core.lock().unwrap().recent_skip_notes(5)
}

/// Overdue, Due, Later today and Earlier today.
#[tauri::command]
fn inbox(app: tauri::State<'_, App>) -> hab_core::Inbox {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.inbox(now())
}

/// The Agenda: open occurrences, and what is expected or closed from
/// yesterday to the end of tomorrow. Unfiltered, like the Inbox.
#[tauri::command]
fn agenda(app: tauri::State<'_, App>) -> hab_core::Agenda {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.agenda(now())
}

/// Every live reminder as a Board card, unfiltered.
#[tauri::command]
fn board(app: tauri::State<'_, App>) -> Vec<hab_core::BoardCard> {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.board(now())
}

/// Copies a reminder in its list; returns the copy's id.
#[tauri::command]
fn duplicate_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
) -> Result<String, String> {
    let id = app
        .core
        .lock()
        .unwrap()
        .duplicate_reminder(&reminder_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(id)
}

/// The view the window was last left on (Inbox the first time).
#[tauri::command]
fn view(app: tauri::State<'_, App>) -> String {
    app.core.lock().unwrap().view()
}

/// Remembers the view on this device.
#[tauri::command]
fn set_view(app: tauri::State<'_, App>, view: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_view(&view)
        .map_err(|e| e.to_string())
}

/// Whether the first-run tip about the view switcher was dismissed.
#[tauri::command]
fn view_tip_seen(app: tauri::State<'_, App>) -> bool {
    app.core.lock().unwrap().view_tip_seen()
}

/// "Got it" (`true`) dismisses the tip; Help (`false`) shows it again.
#[tauri::command]
fn set_view_tip_seen(app: tauri::State<'_, App>, seen: bool) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_view_tip_seen(seen)
        .map_err(|e| e.to_string())
}

/// The list a new reminder goes in: the one the editor chose, or the
/// personal list.
fn list_or_personal(core: &Core, list_id: Option<String>) -> String {
    list_id
        .filter(|l| !l.is_empty())
        .unwrap_or_else(|| core.personal_list_id().to_string())
}

/// The lists, the personal list first.
#[tauri::command]
fn lists(app: tauri::State<'_, App>) -> Vec<hab_core::ListInfo> {
    app.core.lock().unwrap().lists()
}

/// Makes a list and returns its id. The sync loop registers it with the
/// server before sending its events.
#[tauri::command]
fn create_list(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    name: String,
    colour: Option<String>,
) -> Result<String, String> {
    let id = app
        .core
        .lock()
        .unwrap()
        .create_list(&name, colour.as_deref(), now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(id)
}

#[tauri::command]
fn rename_list(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    list_id: String,
    name: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .rename_list(&list_id, &name, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

#[tauri::command]
fn colour_list(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    list_id: String,
    colour: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .colour_list(&list_id, &colour, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Deletes an empty list. The personal list can't be deleted.
#[tauri::command]
fn delete_list(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    list_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .delete_list(&list_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Moves a reminder to another list, keeping its history.
#[tauri::command]
fn move_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    list_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .move_reminder(&reminder_id, &list_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Deletes a reminder: with `with_history` everything about it goes,
/// otherwise its history is kept, marked deleted.
#[tauri::command]
fn delete_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    with_history: bool,
) -> Result<(), String> {
    {
        let mut core = app.core.lock().unwrap();
        if with_history {
            core.purge_reminder(&reminder_id, now())
        } else {
            core.delete_reminder(&reminder_id, now())
        }
        .map_err(|e| e.to_string())?;
    }
    changed(&app, &handle);
    Ok(())
}

/// Pauses a reminder from now until `until` (unix seconds), or until it is
/// resumed when `until` is not given.
#[tauri::command]
fn pause_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    until: Option<i64>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .pause_reminder(&reminder_id, until, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Resumes a paused reminder.
#[tauri::command]
fn resume_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .resume_reminder(&reminder_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Pauses every reminder in a list from now until `until`, or until resumed.
#[tauri::command]
fn pause_list(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    list_id: String,
    until: Option<i64>,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .pause_list(&list_id, until, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Resumes a paused list.
#[tauri::command]
fn resume_list(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    list_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .resume_list(&list_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// The reminders paused now, for the Board's Paused column.
#[tauri::command]
fn paused_reminders(app: tauri::State<'_, App>) -> Vec<hab_core::PausedReminder> {
    app.core.lock().unwrap().paused_reminders(now())
}

/// Reminders deleted with their history kept.
#[tauri::command]
fn deleted_reminders(app: tauri::State<'_, App>) -> Vec<hab_core::DeletedReminder> {
    app.core.lock().unwrap().deleted_reminders()
}

/// The sidebar's list and priority checkboxes, as this device remembers them.
#[tauri::command]
fn filters(app: tauri::State<'_, App>) -> hab_core::Filters {
    app.core.lock().unwrap().filters()
}

/// Remembers the sidebar's checkboxes. They only decide what the window
/// lists: alerts are not affected.
#[tauri::command]
fn set_filters(app: tauri::State<'_, App>, filters: hab_core::Filters) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_filters(&filters)
        .map_err(|e| e.to_string())
}

/// Tells the scheduler something changed, so alerts follow at once: an
/// occurrence that was acted on takes its notification, sound and alarm
/// window down, rather than at the next scheduled look.
fn changed(app: &App, handle: &AppHandle) {
    app.sync_wake.notify_one();
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
}

#[tauri::command]
fn complete_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
    done_at: Option<i64>,
) -> Result<(), String> {
    let now = now();
    app.core
        .lock()
        .unwrap()
        .complete_at(&occurrence_id, done_at.unwrap_or(now), now)
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

#[tauri::command]
fn skip_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
    note: Option<String>,
) -> Result<(), String> {
    let note = note.filter(|n| !n.trim().is_empty());
    app.core
        .lock()
        .unwrap()
        .skip(&occurrence_id, note.as_deref(), now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Silences the occurrence's current alert on all of the user's devices
/// without closing it.
#[tauri::command]
fn acknowledge_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .acknowledge(&occurrence_id, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Snoozes until `until` (unix seconds) or for `minutes`, or, with neither,
/// for the priority's snooze length.
#[tauri::command]
fn snooze_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
    minutes: Option<i64>,
    until: Option<i64>,
) -> Result<(), String> {
    {
        let mut core = app.core.lock().unwrap();
        let now = now();
        let result = match (until, minutes) {
            (Some(u), _) => core.snooze(&occurrence_id, u, now),
            (None, Some(m)) if m > 0 => core.snooze(&occurrence_id, now + m * 60, now),
            _ => core.snooze_for_interval(&occurrence_id, now).map(|_| ()),
        };
        result.map_err(|e| e.to_string())?;
    }
    changed(&app, &handle);
    Ok(())
}

/// Snoozes an expected occurrence ahead of time: it still fires at its time,
/// quietly, and alerts when the snooze ends.
#[tauri::command]
fn snooze_expected(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    reminder_id: String,
    scheduled_at: i64,
    until: i64,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .snooze_expected(&reminder_id, scheduled_at, until, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// The snooze menu for an open occurrence.
#[tauri::command]
fn snooze_picker(
    app: tauri::State<'_, App>,
    occurrence_id: String,
) -> Result<hab_core::SnoozePicker, String> {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.snooze_picker(&occurrence_id, now())
        .map_err(|e| e.to_string())
}

/// The snooze menu for an expected occurrence (snooze ahead).
#[tauri::command]
fn snooze_picker_expected(
    app: tauri::State<'_, App>,
    reminder_id: String,
    scheduled_at: i64,
) -> Result<hab_core::SnoozePicker, String> {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.snooze_picker_expected(&reminder_id, scheduled_at, now())
        .map_err(|e| e.to_string())
}

/// Every snooze of an occurrence: what it was set to end on and how it ended.
#[tauri::command]
fn snooze_history(app: tauri::State<'_, App>, occurrence_id: String) -> Vec<hab_core::SnoozeView> {
    app.core
        .lock()
        .unwrap()
        .snooze_history(&occurrence_id, now())
}

/// What the alarm window shows; `None` once the occurrence has closed.
#[tauri::command]
fn alarm_view(
    app: tauri::State<'_, App>,
    occurrence_id: String,
) -> Option<hab_core::OccurrenceView> {
    app.core.lock().unwrap().occurrence_view(&occurrence_id)
}

/// Brings the main window forward. `token` is an xdg-activation token from
/// the click that asked (the tray's panel gives one), which Wayland needs
/// before it lets the window take focus.
fn show_window(app: &AppHandle, token: Option<&str>) {
    if let Some(w) = app.get_webview_window("main") {
        alarm::raise(&w, token);
    }
}

/// What this device was set up with, before it said anything in the personal
/// list: its name and whether it is portable (the profile's, or the host name
/// and a check for a battery on a standalone device).
fn device_defaults(app: &App) -> (String, bool) {
    match &*app.setup.lock().unwrap() {
        Some(Setup::Joined(p)) => (p.device_name.clone(), p.portable),
        _ => (
            device_name(),
            this_device::detect_portable(std::path::Path::new(this_device::POWER_SUPPLY_DIR)),
        ),
    }
}

/// Settings → This device: its name, portable or stationary, loudest alert
/// and "Quiet this device until…".
#[tauri::command]
fn this_device(app: tauri::State<'_, App>) -> this_device::ThisDevice {
    let (name, portable) = device_defaults(&app);
    this_device::view(&app.core.lock().unwrap(), &name, portable, now())
}

/// Renames this device. The name is in the user's personal settings, so
/// their other devices see it.
#[tauri::command]
fn set_device_name(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    name: String,
) -> Result<(), String> {
    let kept = this_device::rename(&mut app.core.lock().unwrap(), &name, now())?;
    // The sign-in profile says what the device is called too.
    let mut setup = app.setup.lock().unwrap();
    if let Some(Setup::Joined(p)) = setup.as_mut() {
        p.device_name = kept;
        if let Err(e) = SetupFile::in_dir(&app.data_dir).save(&Setup::Joined(p.clone())) {
            eprintln!("could not save the device's new name: {e}");
        }
    }
    drop(setup);
    changed(&app, &handle);
    Ok(())
}

/// Says whether this device is portable, in the user's personal settings.
#[tauri::command]
fn set_device_portable(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    portable: bool,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_portable(portable, now())
        .map_err(|e| e.to_string())?;
    changed(&app, &handle);
    Ok(())
}

/// Caps the loudest alert style this device uses. Stays on this device.
#[tauri::command]
fn set_loudest_alert(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    style: hab_core::AlertStyle,
    caps_maximum: bool,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .set_loudest_alert(hab_core::LoudestAlert {
            style,
            caps_maximum,
        })
        .map_err(|e| e.to_string())?;
    // Alerts standing in a louder style are replaced at the next look.
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

/// Quiets everything on this device until `until` (unix seconds). Maximum is
/// left out unless `include_maximum`. Not a snooze: other devices still alert.
#[tauri::command]
fn quiet_device(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    until: i64,
    include_maximum: bool,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .quiet_device(until, include_maximum, now())
        .map_err(|e| e.to_string())?;
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

/// Ends "Quiet this device" early.
#[tauri::command]
fn end_quiet_device(app: tauri::State<'_, App>, handle: AppHandle) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .end_quiet_device()
        .map_err(|e| e.to_string())?;
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

/// Whether Settings → This device has the app starting at login.
#[tauri::command]
fn autostart_enabled() -> bool {
    autostart::Autostart::for_this_device().is_some_and(|a| a.is_enabled())
}

/// Turns starting at login on or off for this device.
#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    let autostart = autostart::Autostart::for_this_device()
        .ok_or_else(|| "this device has no autostart folder".to_string())?;
    autostart.set(enabled).map_err(|e| e.to_string())
}

/// What the tray shows now: the badge, its tooltip and the menu.
fn tray_view(core: &Core, now: i64) -> (tray_model::Badge, String, Vec<tray_model::Entry>) {
    let inbox = core.inbox(now);
    let zone = hab_core::zone(&core.device_zone());
    let clock = move |at: i64| {
        zone.as_ref()
            .map(|z| hab_core::clock_time(z, at))
            .unwrap_or_default()
    };
    (
        tray_model::badge(&inbox),
        tray_model::tooltip(&inbox),
        // Nothing waits yet: conditions aren't built, so there are no
        // waiting reminders to list.
        tray_model::menu(
            &inbox,
            &[],
            &core.holding(now),
            core.device_quiet(now),
            now,
            &clock,
        ),
    )
}

fn db_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(p) = std::env::var("HAB_BOT_DB") {
        return Ok(PathBuf::from(p));
    }
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("hab-bot.db"))
}

fn data_dir_of(db: &std::path::Path) -> PathBuf {
    match db.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    }
}

fn device_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "This computer".into())
}

/// What the window shows at start: the first-start choice, or the app.
#[tauri::command]
fn setup_state(app: tauri::State<'_, App>) -> Option<Setup> {
    app.setup.lock().unwrap().clone()
}

/// "This device only": everything keeps working with no server.
#[tauri::command]
fn choose_standalone(app: tauri::State<'_, App>) -> Result<(), String> {
    SetupFile::in_dir(&app.data_dir)
        .save(&Setup::Standalone)
        .map_err(|e| e.to_string())?;
    *app.setup.lock().unwrap() = Some(Setup::Standalone);
    Ok(())
}

#[derive(Serialize)]
struct Found {
    fingerprint: String,
    name: String,
    version: String,
}

/// Look at the server: its certificate, then (over a connection pinned to
/// that certificate) its name and version. Nothing is stored and the setup
/// code is not sent. The user confirms what this returns.
#[tauri::command]
async fn probe_server(address: String) -> Result<Found, String> {
    let fingerprint = tls::probe(&address).await.map_err(|e| e.to_string())?;
    let info = Pinned::new(&address, &fingerprint)
        .info()
        .await
        .map_err(|e| e.to_string())?;
    Ok(Found {
        fingerprint,
        name: info.name,
        version: info.version,
    })
}

#[tauri::command]
fn password_check(password: String, username: String, display_name: String) -> PasswordCheck {
    check_password(&password, &[&username, &display_name])
}

#[tauri::command]
fn passphrase_suggestion() -> String {
    suggest_passphrase()
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct JoinArgs {
    address: String,
    fingerprint: String,
    server_name: String,
    setup_code: String,
    username: String,
    display_name: String,
    password: String,
    portable: bool,
}

/// How this device comes to sync.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Start {
    /// It was set up before and the app has restarted.
    Resume,
    /// It has just created the account. What it made while standalone is
    /// uploaded first, history included, and it names itself.
    FirstDevice,
    /// It has just signed in to an existing account: it takes the account's
    /// personal list, announces itself to the user's other devices, and
    /// downloads everything.
    SignedIn,
}

/// Sync this device's personal list with its server until the app quits.
/// The loop carries on from the same place after every restart, and uploads
/// anything made offline when the server can be reached again.
async fn start_sync(
    handle: AppHandle,
    profile: Profile,
    data_dir: PathBuf,
    shared: SyncShared,
    start: Start,
) -> Result<(), String> {
    let SyncShared {
        core,
        wake,
        scheduler,
        checks,
        stop,
        slot,
    } = shared;
    {
        let mut core = core.lock().unwrap();
        // A sign-in always takes the id the server just gave this device, even
        // if an earlier attempt that failed halfway left another.
        if start == Start::SignedIn || !core.is_joined().map_err(|e| e.to_string())? {
            core.join(
                &format!("u{}", profile.account_id),
                &profile.device_id.to_string(),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let store = KeyStore::open(&data_dir).await;
    let notify = handle.clone();
    // What other devices do reaches the scheduler as a state change: an
    // occurrence closed, snoozed or acknowledged elsewhere takes the
    // notification, sound and alarm window down within seconds, instead of at
    // the scheduler's next look (up to 30 s away).
    let wake_scheduler = Mutex::new(scheduler.clone());
    checks.on_progress(move || {
        let _ = scheduler.send(());
    });
    let syncer = Arc::new(
        Syncer::new(&profile, &store, core.clone(), wake, move || {
            let _ = wake_scheduler.lock().unwrap().send(());
            let _ = notify.emit(STATE_CHANGED, ());
        })
        .await
        .map_err(|e| e.to_string())?
        .with_checks(checks),
    );
    match start {
        Start::Resume => {}
        Start::FirstDevice => {
            core.lock()
                .unwrap()
                .name_device(&profile.device_name, now())
                .map_err(|e| e.to_string())?;
            let _ = handle.emit(STATE_CHANGED, ());
            // If this fails the loop below does it again when it connects.
            if let Err(e) = syncer.upload_standalone().await {
                eprintln!("uploading the standalone history failed: {e}");
            }
        }
        Start::SignedIn => {
            syncer
                .adopt_account_list()
                .await
                .map_err(|e| e.to_string())?;
            core.lock()
                .unwrap()
                .announce_sign_in(&profile.device_name, now())
                .map_err(|e| e.to_string())?;
        }
    }
    let _ = handle.emit(STATE_CHANGED, ());
    *slot.lock().unwrap() = Some(syncer.clone());
    tauri::async_runtime::spawn(async move { syncer.run(stop).await });
    Ok(())
}

/// Create the first account. On success the pin and profile are remembered.
#[tauri::command]
async fn join_server(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    args: JoinArgs,
) -> Result<Profile, String> {
    let store = KeyStore::open(&app.data_dir).await;
    let joined = join(
        JoinRequest {
            address: args.address,
            fingerprint: args.fingerprint,
            server_name: args.server_name,
            setup_code: args.setup_code,
            username: args.username,
            display_name: args.display_name,
            password: args.password,
            device_name: device_name(),
            portable: args.portable,
        },
        &store,
    )
    .await
    .map_err(|e| e.to_string())?;
    let setup = Setup::Joined(joined.profile.clone());
    SetupFile::in_dir(&app.data_dir)
        .save(&setup)
        .map_err(|e| e.to_string())?;
    *app.setup.lock().unwrap() = Some(setup);
    start_sync(
        handle,
        joined.profile.clone(),
        app.data_dir.clone(),
        SyncShared {
            core: app.core.clone(),
            wake: app.sync_wake.clone(),
            scheduler: app.wake.clone(),
            checks: app.checks.clone(),
            stop: app.sync_stop.subscribe(),
            slot: app.syncer.clone(),
        },
        Start::FirstDevice,
    )
    .await?;
    Ok(joined.profile)
}

/// What the sign-in screen found out about the server from what was typed.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SignInFound {
    address: String,
    fingerprint: String,
    name: String,
    version: String,
    /// The fingerprint came from a sign-in code, so there is nothing to
    /// compare by eye.
    from_code: bool,
    /// The text was an existing device's approval link: no username or
    /// password is needed, and this is the link to send the request to.
    approval_link: Option<String>,
}

/// Read what the user typed into the sign-in screen: a sign-in code, or a
/// server address. A code's fingerprint is pinned straight away, and the
/// server has to present it. An address is probed, and the user confirms the
/// fingerprint it shows.
#[tauri::command]
async fn read_sign_in(text: String) -> Result<SignInFound, String> {
    match parse_target(&text).map_err(|e| e.to_string())? {
        SignInTarget::Code(code) => {
            let info = Pinned::new(&code.address, &code.fingerprint)
                .info()
                .await
                .map_err(|e| e.to_string())?;
            Ok(SignInFound {
                address: code.address,
                fingerprint: code.fingerprint,
                // What the code said, which is what the user knows it by.
                name: if code.server_name.is_empty() {
                    info.name
                } else {
                    code.server_name
                },
                version: info.version,
                from_code: true,
                approval_link: None,
            })
        }
        SignInTarget::Approval(link) => {
            let info = Pinned::new(&link.address, &link.fingerprint)
                .info()
                .await
                .map_err(|e| e.to_string())?;
            Ok(SignInFound {
                address: link.address.clone(),
                fingerprint: link.fingerprint.clone(),
                name: if link.server_name.is_empty() {
                    info.name
                } else {
                    link.server_name.clone()
                },
                version: info.version,
                from_code: true,
                approval_link: Some(link.link()),
            })
        }
        SignInTarget::Address(address) => {
            let fingerprint = tls::probe(&address).await.map_err(|e| e.to_string())?;
            let info = Pinned::new(&address, &fingerprint)
                .info()
                .await
                .map_err(|e| e.to_string())?;
            Ok(SignInFound {
                address,
                fingerprint,
                name: info.name,
                version: info.version,
                from_code: false,
                approval_link: None,
            })
        }
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignInArgs {
    address: String,
    fingerprint: String,
    server_name: String,
    username: String,
    password: String,
    portable: bool,
}

/// Sign in on this device with the username and password. On success the pin
/// and profile are remembered and the device starts syncing.
#[tauri::command]
async fn sign_in_server(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    args: SignInArgs,
) -> Result<Profile, String> {
    let store = KeyStore::open(&app.data_dir).await;
    let signed = sign_in(
        SignInRequest {
            address: args.address,
            fingerprint: args.fingerprint,
            server_name: args.server_name,
            username: args.username,
            password: args.password,
            device_name: device_name(),
            portable: args.portable,
        },
        &store,
    )
    .await
    .map_err(|e| e.to_string())?;
    finish_sign_in(&app, handle, signed.profile).await
}

/// A device that has signed in, by password or by approval, starts syncing
/// as a new device of the account. Taking the account's list can fail, such
/// as when no other device has synced yet. Only when it works is this device
/// remembered as signed in.
async fn finish_sign_in(app: &App, handle: AppHandle, profile: Profile) -> Result<Profile, String> {
    start_sync(
        handle,
        profile.clone(),
        app.data_dir.clone(),
        SyncShared {
            core: app.core.clone(),
            wake: app.sync_wake.clone(),
            scheduler: app.wake.clone(),
            checks: app.checks.clone(),
            stop: app.sync_stop.subscribe(),
            slot: app.syncer.clone(),
        },
        Start::SignedIn,
    )
    .await?;
    let setup = Setup::Joined(profile.clone());
    SetupFile::in_dir(&app.data_dir)
        .save(&setup)
        .map_err(|e| e.to_string())?;
    *app.setup.lock().unwrap() = Some(setup);
    Ok(profile)
}

/// An approval's link and QR code, shown on one device for the other to scan.
#[derive(Serialize)]
struct ApprovalView {
    link: String,
    svg: String,
}

impl ApprovalView {
    fn of(link: &ApprovalLink) -> ApprovalView {
        ApprovalView {
            link: link.link(),
            svg: link.qr_svg(),
        }
    }
}

fn read_link(text: &str) -> Result<ApprovalLink, String> {
    ApprovalLink::parse(text).map_err(|e| e.to_string())
}

/// Sign in on this device without typing the password, showing a code for an
/// existing device to scan or be given. The window then waits with
/// [`finish_approval`].
#[tauri::command]
async fn approval_show(
    app: tauri::State<'_, App>,
    address: String,
    fingerprint: String,
    server_name: String,
    portable: bool,
) -> Result<ApprovalView, String> {
    let new = NewDevice::show(
        &address,
        &fingerprint,
        &server_name,
        &device_name(),
        portable,
    )
    .await
    .map_err(|e| e.to_string())?;
    let view = ApprovalView::of(new.link());
    *app.waiting.lock().await = Some(Arc::new(new));
    Ok(view)
}

/// Sign in without the password by scanning (or pasting) an existing device's
/// code. The existing device then shows this device's name and asks.
#[tauri::command]
async fn approval_scan(
    app: tauri::State<'_, App>,
    link: String,
    portable: bool,
) -> Result<(), String> {
    let new = NewDevice::scan(&read_link(&link)?, &device_name(), portable)
        .await
        .map_err(|e| e.to_string())?;
    *app.waiting.lock().await = Some(Arc::new(new));
    Ok(())
}

/// Has an existing device approved this one yet? When it has, this device
/// starts syncing like after a password sign-in and the profile is returned.
#[tauri::command]
async fn finish_approval(
    app: tauri::State<'_, App>,
    handle: AppHandle,
) -> Result<Option<Profile>, String> {
    let waiting = app.waiting.lock().await.clone();
    let Some(new) = waiting else {
        return Err("Nothing is waiting for approval. Start again.".into());
    };
    let store = KeyStore::open(&app.data_dir).await;
    match new.collect(&store).await {
        Ok(None) => Ok(None),
        Ok(Some(signed)) => {
            *app.waiting.lock().await = None;
            finish_sign_in(&app, handle, signed.profile).await.map(Some)
        }
        Err(e) => {
            *app.waiting.lock().await = None;
            Err(e.to_string())
        }
    }
}

/// Stop waiting for approval, such as when the user goes back.
#[tauri::command]
async fn cancel_approval(app: tauri::State<'_, App>) -> Result<(), String> {
    *app.waiting.lock().await = None;
    Ok(())
}

/// Settings → Account: make a code for a new device to scan.
#[tauri::command]
async fn offer_approval(app: tauri::State<'_, App>) -> Result<ApprovalView, String> {
    let syncer = running_sync(&app)?;
    let link = syncer.offer_approval().await.map_err(|e| e.to_string())?;
    *app.pending.lock().unwrap() = None;
    Ok(ApprovalView::of(&link))
}

/// The new device behind a code, as the user confirms it: its name.
#[derive(Serialize)]
struct PendingView {
    name: String,
    portable: bool,
}

/// Has the new device behind this code (shown here, or pasted from there)
/// asked yet? When it has, its name is returned for the user to confirm.
#[tauri::command]
async fn pending_approval(
    app: tauri::State<'_, App>,
    link: String,
) -> Result<Option<PendingView>, String> {
    let syncer = running_sync(&app)?;
    let pending = syncer
        .pending_device(&read_link(&link)?)
        .await
        .map_err(|e| e.to_string())?;
    let view = pending.as_ref().map(|p| PendingView {
        name: p.name.clone(),
        portable: p.portable,
    });
    *app.pending.lock().unwrap() = pending;
    Ok(view)
}

/// The user confirmed the device [`pending_approval`] showed: sign it, give
/// it the keys and tell the other devices.
#[tauri::command]
async fn approve_pending(app: tauri::State<'_, App>) -> Result<(), String> {
    let syncer = running_sync(&app)?;
    let pending = app.pending.lock().unwrap().take();
    let Some(pending) = pending else {
        return Err("Check the new device's name first.".into());
    };
    syncer
        .approve_device(&pending)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// The sign-in code for Settings → Account, as a link and a QR code.
#[derive(Serialize)]
struct SignInCodeView {
    link: String,
    svg: String,
}

#[tauri::command]
fn sign_in_code(app: tauri::State<'_, App>) -> Result<SignInCodeView, String> {
    match app.setup.lock().unwrap().as_ref() {
        Some(Setup::Joined(p)) => {
            let code = SignInCode::for_profile(p);
            Ok(SignInCodeView {
                link: code.link(),
                svg: code.qr_svg(),
            })
        }
        _ => Err("This device is not signed in to a server.".into()),
    }
}

/// The user has seen a "New device signed in" notice.
#[tauri::command]
fn dismiss_notice(app: tauri::State<'_, App>, handle: AppHandle, id: String) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .dismiss_notice(&id)
        .map_err(|e| e.to_string())?;
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

/// The running sync, or why there is none.
fn running_sync(app: &App) -> Result<Arc<Syncer>, String> {
    app.syncer
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "This device is not syncing with a server.".to_string())
}

/// Settings → Account: the user's devices, with when each last synced.
#[tauri::command]
async fn list_devices(app: tauri::State<'_, App>) -> Result<Vec<DeviceInfo>, String> {
    let syncer = running_sync(&app)?;
    syncer.devices().await.map_err(|e| e.to_string())
}

/// Remove another of the user's devices and rotate the keys it held.
#[tauri::command]
async fn remove_device(app: tauri::State<'_, App>, device_id: i64) -> Result<(), String> {
    let syncer = running_sync(&app)?;
    syncer
        .remove_device(device_id)
        .await
        .map_err(|e| e.to_string())
}

/// Settings → Account → Change password. A device that is still signed in
/// can set a new password without the old one, so this is also what a
/// forgotten password needs. The new password has to pass the strength check.
#[tauri::command]
async fn change_password(app: tauri::State<'_, App>, password: String) -> Result<(), String> {
    let syncer = running_sync(&app)?;
    syncer
        .change_password(&password)
        .await
        .map_err(|e| e.to_string())
}

/// For Settings → Account and This device: where the keys are kept.
#[tauri::command]
async fn key_store_name(app: tauri::State<'_, App>) -> Result<String, String> {
    Ok(KeyStore::open(&app.data_dir).await.backend().to_string())
}

/// Fires what is due, alerts about every open occurrence in the style its
/// priority calls for right now (see `hab_core::Alerter`), and sleeps until
/// the next reminder, escalation, repeat or end of a snooze. The first pass
/// fires reminders whose time passed while the app was closed.
///
/// Alerting looks at every open occurrence rather than only what `tick` just
/// fired: that covers escalation, repeats, snoozes ending and catching up
/// after Do Not Disturb, and it still alerts a late firing only for the
/// latest instance because `tick` closes the earlier ones as missed.
fn run_scheduler(
    app: AppHandle,
    core: Arc<Mutex<Core>>,
    sync_wake: Arc<Notify>,
    woken: mpsc::Receiver<()>,
    delivery: Arc<Delivery>,
    checks: Arc<Checks>,
    tray: Option<Arc<Tray>>,
) {
    // When an open occurrence next goes overdue: the window then has to move
    // it from Due to Overdue.
    let mut overdue_at: Option<i64> = None;
    // What held alerts at the last pass: when a snooze-all ends, or quiet
    // hours start or stop, the window's chip and footer follow.
    let mut held: Vec<(Option<String>, i64)> = Vec::new();
    let mut alerter = Alerter::new();
    loop {
        // Asked of the notification server before taking the core's lock.
        let inhibited = delivery.inhibited();
        let (fired, next, next_overdue, pass, tray_view, now_held) = {
            let mut core = core.lock().unwrap();
            // The user may have travelled: floating reminders follow.
            let _ = core.use_system_zone();
            let fired = core.tick(now()).unwrap_or_else(|e| {
                eprintln!("tick failed: {e}");
                Vec::new()
            });
            let pass = alerter
                .pass_with(&mut core, now(), inhibited, &*checks)
                .unwrap_or_else(|e| {
                    eprintln!("alerting failed: {e}");
                    Default::default()
                });
            // The tray follows every pass: a firing, a snooze ending, an
            // occurrence going overdue (the pass is scheduled for it) and
            // any action or synced change (which wake the scheduler).
            let view = tray.as_ref().map(|_| tray_view(&core, now()));
            let now_held: Vec<(Option<String>, i64)> = core
                .holding(now())
                .into_iter()
                .map(|h| (h.id, h.until))
                .collect();
            (
                fired,
                core.next_fire_at(),
                core.next_overdue_at(now()),
                pass,
                view,
                now_held,
            )
        };
        if let (Some(tray), Some((badge, tooltip, menu))) = (&tray, tray_view) {
            tray.update(badge, tooltip, menu);
        }
        if now_held != held {
            held = now_held;
            let _ = app.emit(STATE_CHANGED, ());
        }
        let went_overdue = overdue_at.is_some_and(|t| t <= now());
        overdue_at = next_overdue;
        if went_overdue {
            let _ = app.emit(STATE_CHANGED, ());
        }
        let next = next
            .into_iter()
            .chain(next_overdue)
            .chain(pass.next_at)
            .min();
        delivery.apply(&pass.commands);
        // Alerts are in the history, which syncs.
        if !fired.is_empty() || !pass.commands.is_empty() {
            sync_wake.notify_one();
        }
        if !fired.is_empty() {
            let _ = app.emit(STATE_CHANGED, ());
        }
        let wait = next
            .map(|t| Duration::from_secs((t - now()).max(0) as u64))
            .map_or(MAX_SLEEP, |d| d.min(MAX_SLEEP));
        match woken.recv_timeout(wait.max(Duration::from_millis(200))) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// What a click on a notification or one of its buttons does. It works with
/// the window closed: Done, Snooze, Acknowledge and Skip act on the core
/// directly.
fn act_on_notification(
    action: UserAction,
    app: &AppHandle,
    core: &Mutex<Core>,
    sync_wake: &Notify,
    wake: &Sender<()>,
    delivery: &Delivery,
) {
    let result = match &action {
        UserAction::Done(id) => core.lock().unwrap().complete(id, now()),
        UserAction::Skip(id) => core.lock().unwrap().skip(id, None, now()),
        UserAction::Snooze(id) => core
            .lock()
            .unwrap()
            .snooze_for_interval(id, now())
            .map(|_| ()),
        UserAction::Acknowledge(id) => core.lock().unwrap().acknowledge(id, now()),
        UserAction::Open(id) => {
            // An alarm opens its own window, with the click's token so that
            // Wayland lets it take focus; anything else opens the app.
            if !delivery.raise_alarm(id) {
                show_window(app, None);
                let _ = app.emit(OPEN_OCCURRENCE, id);
            }
            return;
        }
    };
    if let Err(e) = result {
        // Most likely closed elsewhere a moment ago.
        eprintln!("notification action failed: {e}");
    }
    sync_wake.notify_one();
    // The scheduler takes the notification (and an alarm's sound and window)
    // down.
    let _ = wake.send(());
    let _ = app.emit(STATE_CHANGED, ());
}

/// A later launch of the app, handed to the running one by the
/// single-instance plugin (see [`launch`]): bring the window forward, at an
/// occurrence or Settings if it asked. An autostart launch (`--hidden`) asks
/// for nothing.
fn later_launch(app: &AppHandle, args: &[String]) {
    let launch = launch::parse(args);
    if launch.hidden && launch.open.is_none() && !launch.settings {
        return;
    }
    // The plugin forwards only the arguments, not the launcher's
    // XDG_ACTIVATION_TOKEN, so on Wayland this raise carries no token.
    show_window(app, None);
    if let Some(id) = &launch.open {
        let _ = app.emit(OPEN_OCCURRENCE, id);
    }
    if launch.settings {
        let _ = app.emit(OPEN_SETTINGS, ());
    }
}

/// After a tray action on the core: sync, and wake the scheduler so that
/// alerts, the badge and the menu follow at once.
fn finished_tray_action(
    result: hab_core::Result<()>,
    app: &AppHandle,
    sync_wake: &Notify,
    wake: &Sender<()>,
) {
    if let Err(e) = result {
        eprintln!("tray action failed: {e}");
    }
    sync_wake.notify_one();
    let _ = wake.send(());
    let _ = app.emit(STATE_CHANGED, ());
}

/// What a click on the tray or its menu does.
fn act_on_tray(
    action: TrayAction,
    token: Option<&str>,
    app: &AppHandle,
    core: &Mutex<Core>,
    sync_wake: &Notify,
    wake: &Sender<()>,
    delivery: &Delivery,
) {
    let on_occurrence = match action {
        TrayAction::OpenWindow => return show_window(app, token),
        TrayAction::OpenOccurrence(id) => {
            show_window(app, token);
            let _ = app.emit(OPEN_OCCURRENCE, id);
            return;
        }
        TrayAction::Settings => {
            show_window(app, token);
            let _ = app.emit(OPEN_SETTINGS, ());
            return;
        }
        TrayAction::SnoozeAllDialog => {
            show_window(app, token);
            let _ = app.emit(OPEN_SNOOZE_ALL, ());
            return;
        }
        TrayAction::SnoozeAll(choice) => {
            let result =
                core.lock()
                    .unwrap()
                    .snooze_all_for(hab_core::Scope::All, choice, false, now());
            return finished_tray_action(result.map(|_| ()), app, sync_wake, wake);
        }
        TrayAction::EndSnoozeAll(id) => {
            let result = core.lock().unwrap().end_snooze_all(&id, now());
            return finished_tray_action(result, app, sync_wake, wake);
        }
        TrayAction::QuietDevice(choice) => {
            let result = core.lock().unwrap().quiet_device_for(choice, false, now());
            return finished_tray_action(result, app, sync_wake, wake);
        }
        TrayAction::EndQuietDevice => {
            let result = core.lock().unwrap().end_quiet_device();
            return finished_tray_action(result, app, sync_wake, wake);
        }
        TrayAction::QuietDeviceDialog => {
            show_window(app, token);
            let _ = app.emit(OPEN_THIS_DEVICE, ());
            return;
        }
        TrayAction::Quit => return app.exit(0),
        TrayAction::Done(id) => UserAction::Done(id),
        TrayAction::Snooze(id) => UserAction::Snooze(id),
        TrayAction::Skip(id) => UserAction::Skip(id),
    };
    act_on_notification(on_occurrence, app, core, sync_wake, wake, delivery);
}

pub fn run() {
    let hidden = launch::parse(&std::env::args().collect::<Vec<_>>()).hidden;
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            later_launch(app, &args);
        }))
        .invoke_handler(tauri::generate_handler![
            snapshot,
            agenda,
            board,
            duplicate_reminder,
            view,
            set_view,
            view_tip_seen,
            set_view_tip_seen,
            snooze_all,
            end_snooze_all,
            holding,
            quiet_hours,
            set_quiet_hours,
            lists,
            create_list,
            rename_list,
            colour_list,
            delete_list,
            move_reminder,
            delete_reminder,
            pause_reminder,
            resume_reminder,
            pause_list,
            resume_list,
            paused_reminders,
            deleted_reminders,
            filters,
            set_filters,
            create_reminder,
            reminder_view,
            edit_reminder,
            create_recurring_reminder,
            home_location,
            set_home_location,
            clear_home_location,
            create_countdown_reminder,
            complete_early,
            skip_ahead,
            undo_occurrence,
            correct_occurrence,
            closed_occurrence,
            recent_skip_notes,
            priorities,
            app_version,
            inbox,
            complete_occurrence,
            skip_occurrence,
            acknowledge_occurrence,
            snooze_occurrence,
            snooze_expected,
            snooze_picker,
            snooze_picker_expected,
            snooze_history,
            alarm_view,
            setup_state,
            choose_standalone,
            probe_server,
            password_check,
            passphrase_suggestion,
            join_server,
            read_sign_in,
            sign_in_server,
            sign_in_code,
            approval_show,
            approval_scan,
            finish_approval,
            cancel_approval,
            offer_approval,
            pending_approval,
            approve_pending,
            dismiss_notice,
            list_devices,
            remove_device,
            change_password,
            key_store_name,
            autostart_enabled,
            set_autostart,
            this_device,
            set_device_name,
            set_device_portable,
            set_loudest_alert,
            quiet_device,
            end_quiet_device
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            let db = db_path(&handle)?;
            let core = Arc::new(Mutex::new(Core::open(&db)?));
            let data_dir = data_dir_of(&db);
            let setup = SetupFile::in_dir(&data_dir).load()?;
            let (wake, woken) = mpsc::channel();
            let wake_scheduler = wake.clone();
            let wake_scheduler_for_tray = wake.clone();
            let wake_for_sync = wake.clone();
            // A device with a server starts out about to connect: a firing it
            // missed while the app was closed checks with the server before
            // it alerts, rather than racing the first connection.
            let checks = Arc::new(if matches!(setup, Some(Setup::Joined(_))) {
                Checks::connecting()
            } else {
                Checks::new()
            });
            let sync_wake = Arc::new(Notify::new());
            let (sync_stop, stop) = watch::channel(false);
            let syncer: SyncerSlot = Arc::new(Mutex::new(None));
            if let Some(Setup::Joined(profile)) = &setup {
                let (handle, profile, data_dir) =
                    (handle.clone(), profile.clone(), data_dir.clone());
                let (core, sync_wake, slot) = (core.clone(), sync_wake.clone(), syncer.clone());
                let checks = checks.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = start_sync(
                        handle,
                        profile,
                        data_dir,
                        SyncShared {
                            core,
                            wake: sync_wake,
                            scheduler: wake_for_sync,
                            checks,
                            stop,
                            slot,
                        },
                        Start::Resume,
                    )
                    .await
                    {
                        eprintln!("sync did not start: {e}");
                    }
                });
            }
            app.manage(App {
                core: core.clone(),
                wake,
                checks: checks.clone(),
                sync_wake: sync_wake.clone(),
                sync_stop,
                data_dir,
                setup: Mutex::new(setup),
                syncer,
                waiting: tokio::sync::Mutex::new(None),
                pending: Mutex::new(None),
            });

            // Started at login (`--hidden`): the tray and no window. Otherwise
            // the window opens, as it is created hidden (tauri.conf.json) so
            // that a login start never flashes it.
            if !hidden {
                show_window(&handle, None);
            }
            // The entry follows the program if it moved (an update).
            if let Some(a) = autostart::Autostart::for_this_device() {
                if let Err(e) = a.repair() {
                    eprintln!("could not repair the autostart entry: {e}");
                }
            }

            let registry = Arc::new(alarm::Registry::default());
            let delivery = Arc::new(Delivery::new(
                Box::new(notify::DbusNotifier::session()),
                Box::new(sound::LoopingSound::bundled()),
                Box::new(alarm::TauriWindows::new(handle.clone(), registry.clone())),
                Box::new(alarm::WaylandActivation {
                    app_id: notify::DESKTOP_ENTRY,
                }),
            ));
            app.manage(AlarmState {
                delivery: delivery.clone(),
                registry,
            });
            {
                let (delivery, handle, core, sync_wake, wake) = (
                    delivery.clone(),
                    handle.clone(),
                    core.clone(),
                    sync_wake.clone(),
                    wake_scheduler,
                );
                notify::listen(None, move |signal| {
                    // A change to Do Not Disturb is noticed at once.
                    if signal == notify::Signal::PropertiesChanged {
                        let _ = wake.send(());
                    }
                    if let Some(action) = delivery.on_signal(&signal) {
                        act_on_notification(action, &handle, &core, &sync_wake, &wake, &delivery);
                    }
                })?;
            }
            // The tray is our own StatusNotifierItem (see `tray`). Without a
            // session bus the app still runs, just without it.
            let tray = {
                let (handle, core, sync_wake, wake, delivery) = (
                    handle.clone(),
                    core.clone(),
                    sync_wake.clone(),
                    wake_scheduler_for_tray,
                    delivery.clone(),
                );
                match Tray::start(
                    None,
                    Arc::new(move |action, token| {
                        act_on_tray(
                            action,
                            token.as_deref(),
                            &handle,
                            &core,
                            &sync_wake,
                            &wake,
                            &delivery,
                        )
                    }),
                ) {
                    Ok(t) => Some(Arc::new(t)),
                    Err(e) => {
                        eprintln!("no tray: {e}");
                        None
                    }
                }
            };
            std::thread::spawn(move || {
                run_scheduler(handle, core, sync_wake, woken, delivery, checks, tray)
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            let WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            if window.label().starts_with(alarm::LABEL_PREFIX) {
                // Closing an alarm window silences the alarm, notification
                // and sound too. The occurrence stays open: it rings again at
                // its next repeat.
                if let Some(state) = window.try_state::<AlarmState>() {
                    if let Some(occurrence) = state.registry.occurrence_of(window.label()) {
                        state.delivery.silence(&occurrence, true);
                        state.registry.forget(&occurrence);
                    }
                }
                return;
            }
            // Closing the main window leaves the app running in the tray.
            api.prevent_close();
            let _ = window.hide();
        })
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
