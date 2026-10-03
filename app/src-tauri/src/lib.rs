//! The Linux desktop app: a Tauri 2 shell around `hab-core`.
//!
//! The scheduler runs in a Rust thread, not in the window, so reminders fire
//! with the window closed. Closing the window hides it; Quit in the tray exits.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hab_core::{Core, Snapshot};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// How long the scheduler sleeps at most, so it notices clock changes and
/// resumes from suspend promptly.
const MAX_SLEEP: Duration = Duration::from_secs(30);
const STATE_CHANGED: &str = "state-changed";

struct App {
    core: Arc<Mutex<Core>>,
    wake: Sender<()>,
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

#[tauri::command]
fn create_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    title: String,
    fire_at: i64,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .create_reminder(&title, fire_at, now())
        .map_err(|e| e.to_string())?;
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

#[tauri::command]
fn complete_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .complete(&occurrence_id, now())
        .map_err(|e| e.to_string())?;
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn db_path(app: &AppHandle) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if let Ok(p) = std::env::var("HAB_BOT_DB") {
        return Ok(PathBuf::from(p));
    }
    let dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("hab-bot.db"))
}

/// Fires what is due, shows a plain notification for each, and sleeps until
/// the next reminder. The first pass fires reminders whose time passed while
/// the app was closed.
fn run_scheduler(app: AppHandle, core: Arc<Mutex<Core>>, woken: mpsc::Receiver<()>) {
    loop {
        let (fired, next) = {
            let mut core = core.lock().unwrap();
            let fired = core.tick(now()).unwrap_or_else(|e| {
                eprintln!("tick failed: {e}");
                Vec::new()
            });
            (fired, core.next_fire_at())
        };
        for f in &fired {
            if let Err(e) = app
                .notification()
                .builder()
                .title(&f.title)
                .body("Reminder")
                .show()
            {
                eprintln!("notification failed: {e}");
            }
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .invoke_handler(tauri::generate_handler![
            snapshot,
            create_reminder,
            complete_occurrence
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let core = Arc::new(Mutex::new(Core::open(&db_path(&handle)?)?));
            let (wake, woken) = mpsc::channel();
            app.manage(App {
                core: core.clone(),
                wake,
            });

            let open = MenuItem::with_id(app, "open", "Open Reminders", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;
            let mut tray = TrayIconBuilder::new()
                .tooltip("Reminders")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            std::thread::spawn(move || run_scheduler(handle, core, woken));
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window leaves the app running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running the app");
}
