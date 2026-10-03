//! The Tauri shell: commands for the UI, a clock thread that fires reminders, and the tray.

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "linux")]
mod linux;

use hab_core::{Core, InboxItem, Millis, NewReminder, Trigger};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;
#[cfg(desktop)]
use tauri::WindowEvent;
use tauri::{AppHandle, Emitter, Manager};
#[cfg(not(any(target_os = "android", target_os = "linux")))]
use tauri_plugin_notification::NotificationExt;

/// The personal user's name in the skeleton, until accounts exist.
const USER: &str = "me";

struct App {
    core: Mutex<Core>,
}

fn now() -> Millis {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as Millis)
        .unwrap_or(0)
}

/// Tells the core the device's current time zone, which floating reminders follow.
fn sync_zone(core: &mut Core) {
    if let Ok(zone) = iana_time_zone::get_timezone() {
        core.set_zone(&zone);
    }
}

/// The alarm path on Android writes to the same database from a second core, so the
/// commands reload the stream first.
fn fresh(app: &App) -> std::sync::MutexGuard<'_, Core> {
    let mut core = app.core.lock().unwrap();
    let _ = core.refresh();
    sync_zone(&mut core);
    core
}

#[tauri::command]
fn inbox(app: tauri::State<App>) -> Vec<InboxItem> {
    fresh(&app).inbox(now())
}

#[tauri::command]
fn create_one_off(app: tauri::State<App>, title: String, due_at: Millis) -> Result<(), String> {
    create(
        app,
        NewReminder {
            title,
            triggers: vec![Trigger::OneOff { at: due_at }],
            tz: None,
            priority: Default::default(),
            expiry: None,
        },
    )
}

/// Makes a reminder with any triggers: one-offs and RRULE schedules, floating or pinned.
#[tauri::command]
fn create_reminder(app: tauri::State<App>, reminder: NewReminder) -> Result<(), String> {
    create(app, reminder)
}

fn create(app: tauri::State<App>, reminder: NewReminder) -> Result<(), String> {
    let mut core = fresh(&app);
    core.create(reminder, now()).map_err(|e| e.to_string())?;
    #[cfg(target_os = "android")]
    android::refresh();
    Ok(())
}

/// One tap on Snooze: the priority's current interval.
#[tauri::command]
fn snooze(app: tauri::State<App>, occurrence_id: String) -> Result<Millis, String> {
    fresh(&app)
        .snooze_default(&occurrence_id, hab_core::SnoozeVia::Button, now())
        .map_err(|e| e.to_string())
}

/// Snoozes until a chosen time, for the menu's other choices and "pick a time".
#[tauri::command]
fn snooze_until(
    app: tauri::State<App>,
    occurrence_id: String,
    until: Millis,
) -> Result<(), String> {
    fresh(&app)
        .snooze(&occurrence_id, until, hab_core::SnoozeVia::Button, now())
        .map_err(|e| e.to_string())
}

/// Cancels a snooze, for Undo after a swipe.
#[tauri::command]
fn unsnooze(app: tauri::State<App>, occurrence_id: String) -> Result<(), String> {
    fresh(&app)
        .unsnooze(&occurrence_id, now())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn snooze_picker(app: tauri::State<App>, occurrence_id: String) -> Option<hab_core::SnoozePicker> {
    fresh(&app).snooze_picker(&occurrence_id, now())
}

#[tauri::command]
fn skip(app: tauri::State<App>, occurrence_id: String, note: Option<String>) -> Result<(), String> {
    fresh(&app)
        .skip(&occurrence_id, note, now())
        .map_err(|e| e.to_string())
}

/// Done at a time the user says, which can be before the firing.
#[tauri::command]
fn complete_at(app: tauri::State<App>, occurrence_id: String, at: Millis) -> Result<(), String> {
    fresh(&app)
        .complete_at(&occurrence_id, at, now())
        .map_err(|e| e.to_string())
}

/// Skips an expected occurrence ahead of time.
#[tauri::command]
fn skip_ahead(
    app: tauri::State<App>,
    occurrence_id: String,
    note: Option<String>,
) -> Result<(), String> {
    let mut core = fresh(&app);
    core.skip_ahead(&occurrence_id, note, now())
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "android")]
    android::refresh();
    Ok(())
}

/// "Skip all…" on the folded row of older quiet reminders. Returns how many were skipped.
#[tauri::command]
fn skip_older_quiet(app: tauri::State<App>) -> Result<usize, String> {
    fresh(&app)
        .skip_older_quiet(now())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn undo(app: tauri::State<App>, occurrence_id: String) -> Result<(), String> {
    fresh(&app)
        .undo(&occurrence_id, now())
        .map_err(|e| e.to_string())
}

/// Changes a closed occurrence to completed or skipped, at a time.
#[tauri::command]
fn correct(
    app: tauri::State<App>,
    occurrence_id: String,
    to: hab_core::Outcome,
    at: Millis,
    note: Option<String>,
) -> Result<(), String> {
    fresh(&app)
        .correct(&occurrence_id, to, at, note, now())
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn recent_skip_notes(app: tauri::State<App>) -> Vec<String> {
    fresh(&app).recent_skip_notes(5)
}

/// Completes a reminder ahead of its next expected occurrence, which then never fires.
#[tauri::command]
fn complete_early(app: tauri::State<App>, reminder_id: String) -> Result<(), String> {
    let mut core = fresh(&app);
    core.complete_early(&reminder_id, now(), now())
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "android")]
    android::refresh();
    Ok(())
}

/// The built-in priorities with every setting, for Settings → Priorities (read-only).
#[tauri::command]
fn priorities() -> Vec<hab_core::PrioritySettings> {
    hab_core::all_settings()
}

#[tauri::command]
fn about(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// The permissions still missing on Android; the UI explains each before asking.
#[tauri::command]
fn missing_permissions() -> Vec<String> {
    #[cfg(target_os = "android")]
    return android::missing_permissions();
    #[cfg(not(target_os = "android"))]
    Vec::new()
}

#[tauri::command]
fn request_permissions() {
    #[cfg(target_os = "android")]
    android::request_permissions();
}

#[tauri::command]
fn complete(app: tauri::State<App>, occurrence_id: String) -> Result<(), String> {
    fresh(&app)
        .complete(&occurrence_id, now())
        .map_err(|e| e.to_string())
}

/// On Android, firing belongs to the exact alarm (Kotlin, through `android.rs`), which
/// works with the app closed. Here we only tell the UI when the stream changed.
#[cfg(target_os = "android")]
fn spawn_clock(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut last = Vec::new();
        loop {
            let ids: Vec<String> = {
                let state = handle.state::<App>();
                let core = fresh(&state);
                core.inbox(now())
                    .into_iter()
                    .map(|i| i.occurrence.id)
                    .collect()
            };
            if ids != last {
                last = ids;
                // something changed in the app: bring the notifications and alarm up to date
                android::refresh();
                let _ = handle.emit("changed", ());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Fires whatever is due, once now (which catches up on time that passed while the app
/// was closed) and then every second, whether or not the window is open. The core's
/// alert engine says what to show, and `linux.rs` shows it.
#[cfg(not(target_os = "android"))]
fn spawn_clock(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut engine = hab_core::AlertEngine::new(&device_name());
        #[cfg(target_os = "linux")]
        let mut shown = linux::Shown::default();
        loop {
            // asked before taking the lock: it's a D-Bus call
            #[cfg(target_os = "linux")]
            let dnd = linux::do_not_disturb();
            #[cfg(not(target_os = "linux"))]
            let dnd = false;
            let (poll, changed) = {
                let state = handle.state::<App>();
                let mut core = state.core.lock().unwrap();
                sync_zone(&mut core);
                let fired = core.fire_due(now()).unwrap_or_default();
                let poll = engine.poll(&mut core, now(), dnd).unwrap_or_default();
                let changed =
                    !fired.is_empty() || !poll.alerts.is_empty() || !poll.dismissed.is_empty();
                (poll, changed)
            };
            #[cfg(target_os = "linux")]
            {
                for id in &poll.dismissed {
                    linux::dismiss(&mut shown, id);
                }
                for alert in &poll.alerts {
                    linux::show(&handle, &mut shown, alert);
                }
            }
            #[cfg(not(target_os = "linux"))]
            for alert in &poll.alerts {
                let _ = handle
                    .notification()
                    .builder()
                    .title(&alert.title)
                    .body("Due now")
                    .show();
            }
            if changed {
                let _ = handle.emit("changed", ());
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

#[cfg(not(target_os = "android"))]
fn device_name() -> String {
    #[cfg(target_os = "linux")]
    return linux::device_name();
    #[cfg(not(target_os = "linux"))]
    "this device".into()
}

#[cfg(desktop)]
fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(desktop)]
fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_window(app),
            "quit" => app.exit(0),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            inbox,
            create_one_off,
            create_reminder,
            complete,
            skip,
            skip_ahead,
            complete_at,
            undo,
            skip_older_quiet,
            correct,
            recent_skip_notes,
            snooze,
            snooze_until,
            snooze_picker,
            unsnooze,
            priorities,
            about,
            complete_early,
            missing_permissions,
            request_permissions
        ])
        .setup(|app| {
            // Kotlin opens the same file by `filesDir`, so Android names it itself.
            #[cfg(target_os = "android")]
            let dir = std::path::PathBuf::from(android::files_dir().ok_or("no files dir")?);
            #[cfg(not(target_os = "android"))]
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("reminders.db");
            let core = Core::open(path.to_str().expect("utf-8 data path"), USER)?;
            #[cfg(target_os = "android")]
            android::refresh();
            app.manage(App {
                core: Mutex::new(core),
            });
            #[cfg(desktop)]
            setup_tray(app.handle())?;
            spawn_clock(app.handle().clone());
            Ok(())
        })
        // Closing the window leaves the app running in the tray; Quit in the tray exits.
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
            #[cfg(not(desktop))]
            let _ = (window, event);
        })
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
