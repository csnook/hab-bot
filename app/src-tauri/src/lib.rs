//! The Linux desktop app: a Tauri 2 shell around `hab-core`.
//!
//! The scheduler runs in a Rust thread, not in the window, so reminders fire
//! with the window closed. Closing the window hides it; Quit in the tray exits.

use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hab_client::{
    check_password, join, parse_target, sign_in, suggest_passphrase, tls, ApprovalLink, DeviceInfo,
    JoinRequest, KeyStore, NewDevice, PasswordCheck, PendingDevice, Pinned, Profile, Setup,
    SetupFile, SignInCode, SignInRequest, SignInTarget, Syncer,
};
use hab_core::{Alerter, Core, Snapshot};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WindowEvent};
use tokio::sync::{watch, Notify};

/// How long the scheduler sleeps at most, so it notices clock changes and
/// resumes from suspend promptly.
const MAX_SLEEP: Duration = Duration::from_secs(30);
const STATE_CHANGED: &str = "state-changed";
/// Sent to the window when a notification is clicked; the payload is the
/// occurrence's id.
const OPEN_OCCURRENCE: &str = "open-occurrence";

mod notify;
use notify::{Delivery, UserAction};

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
) -> Result<(), String> {
    {
        let mut core = app.core.lock().unwrap();
        let id = core
            .create_reminder(&title, fire_at, now())
            .map_err(|e| e.to_string())?;
        set_priority(&mut core, &id, priority)?;
    }
    app.sync_wake.notify_one();
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

/// A reminder that repeats: a common pattern from a date, at a time of day,
/// pinned to a time zone or, with none, floating.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn create_recurring_reminder(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    title: String,
    pattern: hab_core::Pattern,
    date: String,
    time: String,
    zone: Option<String>,
    priority: Option<hab_core::Priority>,
) -> Result<(), String> {
    let schedule =
        hab_core::Schedule::from_pattern(&pattern, &date, &time).map_err(|e| e.to_string())?;
    {
        let mut core = app.core.lock().unwrap();
        let _ = core.use_system_zone();
        let id = core
            .create_recurring_reminder(&title, vec![schedule], zone.as_deref(), now())
            .map_err(|e| e.to_string())?;
        set_priority(&mut core, &id, priority)?;
    }
    app.sync_wake.notify_one();
    let _ = app.wake.send(());
    let _ = handle.emit(STATE_CHANGED, ());
    Ok(())
}

/// Overdue, Due, Later today and Earlier today.
#[tauri::command]
fn inbox(app: tauri::State<'_, App>) -> hab_core::Inbox {
    let core = app.core.lock().unwrap();
    let _ = core.use_system_zone();
    core.inbox(now())
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

#[tauri::command]
fn skip_occurrence(
    app: tauri::State<'_, App>,
    handle: AppHandle,
    occurrence_id: String,
) -> Result<(), String> {
    app.core
        .lock()
        .unwrap()
        .skip(&occurrence_id, None, now())
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
    let syncer = Arc::new(
        Syncer::new(&profile, &store, core.clone(), wake, move || {
            let _ = notify.emit(STATE_CHANGED, ());
        })
        .await
        .map_err(|e| e.to_string())?,
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
) {
    // When an open occurrence next goes overdue: the window then has to move
    // it from Due to Overdue.
    let mut overdue_at: Option<i64> = None;
    let mut alerter = Alerter::new();
    loop {
        // Asked of the notification server before taking the core's lock.
        let inhibited = delivery.inhibited();
        let (fired, next, next_overdue, pass) = {
            let mut core = core.lock().unwrap();
            // The user may have travelled: floating reminders follow.
            let _ = core.use_system_zone();
            let fired = core.tick(now()).unwrap_or_else(|e| {
                eprintln!("tick failed: {e}");
                Vec::new()
            });
            let pass = alerter
                .pass(&mut core, now(), inhibited)
                .unwrap_or_else(|e| {
                    eprintln!("alerting failed: {e}");
                    Default::default()
                });
            (
                fired,
                core.next_fire_at(),
                core.next_overdue_at(now()),
                pass,
            )
        };
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
/// the window closed: Done and Skip act on the core directly.
fn act_on_notification(
    action: UserAction,
    app: &AppHandle,
    core: &Mutex<Core>,
    sync_wake: &Notify,
    wake: &Sender<()>,
) {
    let result = match &action {
        UserAction::Done(id) => core.lock().unwrap().complete(id, now()),
        UserAction::Skip(id) => core.lock().unwrap().skip(id, None, now()),
        UserAction::Open(id) => {
            show_window(app);
            let _ = app.emit(OPEN_OCCURRENCE, id);
            return;
        }
    };
    if let Err(e) = result {
        // Most likely closed elsewhere a moment ago.
        eprintln!("notification action failed: {e}");
    }
    sync_wake.notify_one();
    // The scheduler takes the notification down.
    let _ = wake.send(());
    let _ = app.emit(STATE_CHANGED, ());
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_window(app);
        }))
        .invoke_handler(tauri::generate_handler![
            snapshot,
            create_reminder,
            create_recurring_reminder,
            priorities,
            app_version,
            inbox,
            complete_occurrence,
            skip_occurrence,
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
            key_store_name
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let db = db_path(&handle)?;
            let core = Arc::new(Mutex::new(Core::open(&db)?));
            let data_dir = data_dir_of(&db);
            let setup = SetupFile::in_dir(&data_dir).load()?;
            let (wake, woken) = mpsc::channel();
            let wake_scheduler = wake.clone();
            let sync_wake = Arc::new(Notify::new());
            let (sync_stop, stop) = watch::channel(false);
            let syncer: SyncerSlot = Arc::new(Mutex::new(None));
            if let Some(Setup::Joined(profile)) = &setup {
                let (handle, profile, data_dir) =
                    (handle.clone(), profile.clone(), data_dir.clone());
                let (core, sync_wake, slot) = (core.clone(), sync_wake.clone(), syncer.clone());
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = start_sync(
                        handle,
                        profile,
                        data_dir,
                        SyncShared {
                            core,
                            wake: sync_wake,
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
                sync_wake: sync_wake.clone(),
                sync_stop,
                data_dir,
                setup: Mutex::new(setup),
                syncer,
                waiting: tokio::sync::Mutex::new(None),
                pending: Mutex::new(None),
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

            let delivery = Arc::new(Delivery::new(Box::new(notify::DbusNotifier::session())));
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
                        act_on_notification(action, &handle, &core, &sync_wake, &wake);
                    }
                })?;
            }
            std::thread::spawn(move || run_scheduler(handle, core, sync_wake, woken, delivery));
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
