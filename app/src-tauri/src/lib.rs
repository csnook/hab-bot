//! The Linux desktop app: a Tauri 2 shell around `hab-core`.
//!
//! The scheduler runs in a Rust thread, not in the window, so reminders fire
//! with the window closed. Closing the window hides it; Quit in the tray exits.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hab_client::{
    check_password, join, suggest_passphrase, tls, JoinRequest, KeyStore, PasswordCheck, Pinned,
    Profile, Setup, SetupFile, Syncer,
};
use hab_core::{Core, Snapshot};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{watch, Notify};

/// How long the scheduler sleeps at most, so it notices clock changes and
/// resumes from suspend promptly.
const MAX_SLEEP: Duration = Duration::from_secs(30);
const STATE_CHANGED: &str = "state-changed";

struct App {
    core: Arc<Mutex<Core>>,
    wake: Sender<()>,
    /// Tells the sync loop there is a change to send.
    sync_wake: Arc<Notify>,
    /// Stops the sync loop; set when the app quits.
    sync_stop: watch::Sender<bool>,
    /// Where `setup.json` and the key file live.
    data_dir: PathBuf,
    /// How this device is set up; None until the first-start choice is made.
    setup: Mutex<Option<Setup>>,
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
    app.sync_wake.notify_one();
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
    app.sync_wake.notify_one();
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

/// Sync this device's personal list with its server until the app quits.
/// What the device made while standalone is uploaded first, history included;
/// the loop then carries on from the same place after every restart, and
/// uploads anything made offline when the server can be reached again.
async fn start_sync(
    handle: AppHandle,
    profile: Profile,
    data_dir: PathBuf,
    core: Arc<Mutex<Core>>,
    wake: Arc<Notify>,
    stop: watch::Receiver<bool>,
    upload_first: bool,
) -> Result<(), String> {
    {
        let mut core = core.lock().unwrap();
        if !core.is_joined().map_err(|e| e.to_string())? {
            core.join(
                &format!("u{}", profile.account_id),
                &profile.device_id.to_string(),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    let store = KeyStore::open(&data_dir).await;
    let notify = handle.clone();
    let syncer = Syncer::new(&profile, &store, core, wake, move || {
        let _ = notify.emit(STATE_CHANGED, ());
    })
    .await
    .map_err(|e| e.to_string())?;
    let _ = handle.emit(STATE_CHANGED, ());
    if upload_first {
        // If this fails the loop below does it again when it connects.
        if let Err(e) = syncer.upload_standalone().await {
            eprintln!("uploading the standalone history failed: {e}");
        }
        let _ = handle.emit(STATE_CHANGED, ());
    }
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
        app.core.clone(),
        app.sync_wake.clone(),
        app.sync_stop.subscribe(),
        true,
    )
    .await?;
    Ok(joined.profile)
}

/// For Settings → Account and This device: where the keys are kept.
#[tauri::command]
async fn key_store_name(app: tauri::State<'_, App>) -> Result<String, String> {
    Ok(KeyStore::open(&app.data_dir).await.backend().to_string())
}

/// Fires what is due, shows a plain notification for each, and sleeps until
/// the next reminder. The first pass fires reminders whose time passed while
/// the app was closed.
fn run_scheduler(
    app: AppHandle,
    core: Arc<Mutex<Core>>,
    sync_wake: Arc<Notify>,
    woken: mpsc::Receiver<()>,
) {
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
            sync_wake.notify_one();
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
            complete_occurrence,
            setup_state,
            choose_standalone,
            probe_server,
            password_check,
            passphrase_suggestion,
            join_server,
            key_store_name
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let db = db_path(&handle)?;
            let core = Arc::new(Mutex::new(Core::open(&db)?));
            let data_dir = data_dir_of(&db);
            let setup = SetupFile::in_dir(&data_dir).load()?;
            let (wake, woken) = mpsc::channel();
            let sync_wake = Arc::new(Notify::new());
            let (sync_stop, stop) = watch::channel(false);
            if let Some(Setup::Joined(profile)) = &setup {
                let (handle, profile, data_dir) =
                    (handle.clone(), profile.clone(), data_dir.clone());
                let (core, sync_wake) = (core.clone(), sync_wake.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(e) =
                        start_sync(handle, profile, data_dir, core, sync_wake, stop, false).await
                    {
                        eprintln!("sync did not start: {e}");
                    }
                });
            }
            app.manage(App {
                core: core.clone(),
                wake,
                sync_wake: sync_wake.clone(),
                sync_stop,
                data_dir,
                setup: Mutex::new(setup),
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

            std::thread::spawn(move || run_scheduler(handle, core, sync_wake, woken));
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
