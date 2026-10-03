//! The Tauri shell: commands for the UI, a clock thread that fires reminders, and the tray.

use hab_core::{Core, InboxItem, Millis};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;

/// The personal user's name in the skeleton, until accounts exist.
const USER: &str = "me";

struct App {
    core: Mutex<Core>,
}

fn now() -> Millis {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as Millis).unwrap_or(0)
}

#[tauri::command]
fn inbox(app: tauri::State<App>) -> Vec<InboxItem> {
    app.core.lock().unwrap().inbox()
}

#[tauri::command]
fn create_one_off(app: tauri::State<App>, title: String, due_at: Millis) -> Result<(), String> {
    let mut core = app.core.lock().unwrap();
    core.create_one_off(&title, due_at, now()).map(|_| ()).map_err(|e| e.to_string())
}

#[tauri::command]
fn complete(app: tauri::State<App>, occurrence_id: String) -> Result<(), String> {
    app.core.lock().unwrap().complete(&occurrence_id, now()).map_err(|e| e.to_string())
}

/// Fires whatever is due, once now (which catches up on time that passed while the app
/// was closed) and then on every tick, whether or not the window is open.
fn spawn_clock(handle: AppHandle) {
    std::thread::spawn(move || loop {
        let opened = {
            let state = handle.state::<App>();
            let mut core = state.core.lock().unwrap();
            core.fire_due(now()).unwrap_or_default()
        };
        for occurrence in &opened {
            let _ = handle.notification().builder().title(&occurrence.title).body("Due now").show();
        }
        if !opened.is_empty() {
            let _ = handle.emit("changed", ());
        }
        std::thread::sleep(Duration::from_secs(1));
    });
}

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
        .invoke_handler(tauri::generate_handler![inbox, create_one_off, complete])
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join("reminders.db");
            let core = Core::open(path.to_str().expect("utf-8 data path"), USER)?;
            app.manage(App { core: Mutex::new(core) });
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
