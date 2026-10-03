//! The Tauri shell: commands for the UI, a clock thread that fires reminders, and the tray.

#[cfg(target_os = "android")]
mod android;

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
#[cfg(not(target_os = "android"))]
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
    android::schedule_alarm(core.next_due(now()));
    Ok(())
}

/// Completes a reminder ahead of its next expected occurrence, which then never fires.
#[tauri::command]
fn complete_early(app: tauri::State<App>, reminder_id: String) -> Result<(), String> {
    let mut core = fresh(&app);
    core.complete_early(&reminder_id, now(), now())
        .map_err(|e| e.to_string())?;
    #[cfg(target_os = "android")]
    android::schedule_alarm(core.next_due(now()));
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
                let _ = handle.emit("changed", ());
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

/// Fires whatever is due, once now (which catches up on time that passed while the app
/// was closed) and then on every tick, whether or not the window is open.
#[cfg(not(target_os = "android"))]
fn spawn_clock(handle: AppHandle) {
    std::thread::spawn(move || loop {
        let opened = {
            let state = handle.state::<App>();
            let mut core = state.core.lock().unwrap();
            sync_zone(&mut core);
            core.fire_due(now()).unwrap_or_default()
        };
        for occurrence in &opened {
            let _ = handle
                .notification()
                .builder()
                .title(&occurrence.title)
                .body("Due now")
                .show();
        }
        if !opened.is_empty() {
            let _ = handle.emit("changed", ());
        }
        std::thread::sleep(Duration::from_secs(1));
    });
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
            android::schedule_alarm(core.next_due(now()));
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
