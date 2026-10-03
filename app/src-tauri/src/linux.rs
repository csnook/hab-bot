//! Linux alerts: our own `org.freedesktop.Notifications` notifications, since Tauri's have
//! no buttons, and the alarm: a critical notification, the app's own alarm window, and a
//! looping sound. Plasma and GNOME both implement the actions, urgency and sound hints
//! used here. What to show and when is decided by the core's `AlertEngine`.

use crate::App;
use hab_core::{Alert, AlertKind, AlertStyle, SnoozeVia};
use notify_rust::{Hint, Notification, Timeout, Urgency};
use std::collections::HashMap;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";
const ALARM_WINDOW: &str = "alarm";

/// The notifications on screen, by occurrence, and whether something is still listening for
/// their buttons.
#[derive(Default)]
struct Shown {
    by_occurrence: HashMap<String, (u32, Arc<AtomicBool>)>,
    /// The alarm that is ringing: its occurrence and the flag that stops its sound.
    ringing: Option<(String, Arc<AtomicBool>)>,
}

static SHOWN: LazyLock<Mutex<Shown>> = LazyLock::new(Mutex::default);

/// This device's name, recorded in the history with each alert.
pub fn device_name() -> String {
    hostname::get()
        .ok()
        .and_then(|h| h.into_string().ok())
        .unwrap_or_else(|| "this device".into())
}

/// Whether the notification server reports itself inhibited (Do Not Disturb). Plasma
/// exposes this as the `Inhibited` property. A server that doesn't is never inhibited.
pub fn do_not_disturb() -> bool {
    let Ok(conn) = zbus::blocking::Connection::session() else {
        return false;
    };
    conn.call_method(
        Some(BUS_NAME),
        OBJECT,
        Some("org.freedesktop.DBus.Properties"),
        "Get",
        &(BUS_NAME, "Inhibited"),
    )
    .ok()
    .and_then(|reply| {
        reply
            .body()
            .deserialize::<zbus::zvariant::OwnedValue>()
            .ok()
    })
    .and_then(|v| bool::try_from(v).ok())
    .unwrap_or(false)
}

fn close_notification(id: u32) {
    if let Ok(conn) = zbus::blocking::Connection::session() {
        let _ = conn.call_method(
            Some(BUS_NAME),
            OBJECT,
            Some(BUS_NAME),
            "CloseNotification",
            &(id,),
        );
    }
}

/// Takes down everything shown for an occurrence that closed: its notification, and its
/// alarm window and sound if it was ringing.
pub fn dismiss(app: &AppHandle, occurrence_id: &str) {
    let notification = SHOWN.lock().unwrap().by_occurrence.remove(occurrence_id);
    if let Some((id, _)) = notification {
        close_notification(id);
    }
    silence(app, occurrence_id);
}

/// Silences an alarm: stops the sound and closes the window. Closing either the window or
/// the notification silences both, so this is called from each.
fn silence(app: &AppHandle, occurrence_id: &str) {
    let ringing = {
        let mut shown = SHOWN.lock().unwrap();
        match &shown.ringing {
            Some((id, _)) if id == occurrence_id => shown.ringing.take(),
            _ => None,
        }
    };
    if let Some((_, stop)) = ringing {
        stop.store(true, Ordering::SeqCst);
        if let Some(window) = app.get_webview_window(ALARM_WINDOW) {
            let _ = window.close();
        }
    }
}

/// Called when the alarm window is closed by the user: silence the sound and take down the
/// notification.
pub fn alarm_window_closed(app: &AppHandle) {
    let id = SHOWN
        .lock()
        .unwrap()
        .ringing
        .as_ref()
        .map(|(id, _)| id.clone());
    if let Some(id) = id {
        silence(app, &id);
        let notification = SHOWN.lock().unwrap().by_occurrence.remove(&id);
        if let Some((n, _)) = notification {
            close_notification(n);
        }
    }
}

/// Plays the alarm sound from the sound theme in a loop until stopped. There is no
/// maintained Rust crate for sound-theme playback, so this uses libcanberra's
/// `canberra-gtk-play`, falling back to PulseAudio's `paplay` on the freedesktop theme.
fn play_loop(stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            let played = Command::new("canberra-gtk-play")
                .args(["-i", "alarm-clock-elapsed", "-d", "Reminders alarm"])
                .status()
                .or_else(|_| {
                    Command::new("paplay")
                        .arg("/usr/share/sounds/freedesktop/stereo/alarm-clock-elapsed.oga")
                        .status()
                })
                .is_ok_and(|s| s.success());
            if !played {
                // no player: don't spin
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        }
    });
}

/// Starts the alarm for an occurrence: the sound and the alarm window. Raising the window is
/// best-effort: on Wayland the compositor only allows it with an xdg-activation token, which
/// none of the libraries here pass on, so it may open without focus.
fn ring(app: &AppHandle, occurrence_id: &str) {
    {
        let mut shown = SHOWN.lock().unwrap();
        if shown
            .ringing
            .as_ref()
            .is_some_and(|(id, _)| id == occurrence_id)
        {
            return;
        }
        let stop = Arc::new(AtomicBool::new(false));
        play_loop(stop.clone());
        shown.ringing = Some((occurrence_id.to_string(), stop));
    }
    let url = WebviewUrl::App(format!("index.html?alarm={occurrence_id}").into());
    if let Some(window) = app.get_webview_window(ALARM_WINDOW) {
        let _ = window.close();
    }
    if let Ok(window) = WebviewWindowBuilder::new(app, ALARM_WINDOW, url)
        .title("Reminders: alarm")
        .inner_size(420.0, 420.0)
        .always_on_top(true)
        .center()
        .build()
    {
        let _ = window.set_focus();
    }
}

/// Shows or updates the notification for an alert.
///
/// - Silent: low urgency, no sound.
/// - Gentle: normal urgency, a sound from the theme, and it times out.
/// - Insistent: gentle, repeated by the engine every interval.
/// - Alarm: critical, never times out, with Done · Snooze · Acknowledge, alongside the alarm
///   window and its looping sound. Maximum gets through Do Not Disturb this way.
pub fn show(app: &AppHandle, alert: &Alert) {
    // An occurrence that was ringing and is now quieter (acknowledged, snoozed) stops ringing.
    if alert.style != AlertStyle::Alarm {
        silence(app, &alert.occurrence_id);
    }
    let (summary, body) = match (alert.kind, alert.expires_at) {
        (AlertKind::LastChance, Some(at)) => {
            let at = chrono::DateTime::from_timestamp_millis(at)
                .unwrap_or_default()
                .with_timezone(&chrono::Local);
            (
                format!("Last chance: {}", alert.title),
                format!("Expires at {}", at.format("%H:%M")),
            )
        }
        _ => (
            alert.title.clone(),
            if alert.overdue { "Overdue" } else { "Due now" }.to_string(),
        ),
    };
    let mut n = Notification::new();
    n.appname("Reminders")
        .summary(&summary)
        .body(&body)
        .action("done", "Done")
        .action("snooze", "Snooze");
    match alert.style {
        AlertStyle::Alarm => n.action("acknowledge", "Acknowledge"),
        _ => n.action("skip", "Skip"),
    };
    n.action("default", "Open");
    match alert.style {
        AlertStyle::Silent => {
            n.urgency(Urgency::Low).hint(Hint::SuppressSound(true));
        }
        AlertStyle::Gentle | AlertStyle::Insistent => {
            n.urgency(Urgency::Normal)
                .hint(Hint::SoundName("message-new-instant".into()))
                .timeout(Timeout::Default);
        }
        AlertStyle::Alarm => {
            // the app plays the looping sound itself
            n.urgency(Urgency::Critical)
                .hint(Hint::SuppressSound(true))
                .hint(Hint::Resident(true))
                .timeout(Timeout::Never);
        }
    }
    // Replace the notification already showing for this occurrence.
    let existing = SHOWN
        .lock()
        .unwrap()
        .by_occurrence
        .get(&alert.occurrence_id)
        .map(|(id, alive)| (*id, alive.clone()));
    if let Some((id, _)) = &existing {
        n.id(*id);
    }
    let Ok(handle) = n.show() else { return };
    let id = handle.id();
    let listening = existing
        .map(|(_, alive)| alive)
        .filter(|alive| alive.load(Ordering::SeqCst));
    if let Some(alive) = listening {
        // the earlier handle's thread still listens for this id
        SHOWN
            .lock()
            .unwrap()
            .by_occurrence
            .insert(alert.occurrence_id.clone(), (id, alive));
    } else {
        let alive = Arc::new(AtomicBool::new(true));
        SHOWN
            .lock()
            .unwrap()
            .by_occurrence
            .insert(alert.occurrence_id.clone(), (id, alive.clone()));
        listen(app, handle, alert.occurrence_id.clone(), alive);
    }
    if alert.style == AlertStyle::Alarm {
        ring(app, &alert.occurrence_id);
    }
}

/// Buttons work with the window closed: the thread acts through the core directly.
fn listen(
    app: &AppHandle,
    handle: notify_rust::NotificationHandle,
    occurrence_id: String,
    alive: Arc<AtomicBool>,
) {
    let app = app.clone();
    std::thread::spawn(move || {
        handle.wait_for_action(|action| {
            let state = app.state::<App>();
            match action {
                "done" => {
                    let _ = crate::fresh(&state).complete(&occurrence_id, crate::now());
                }
                "snooze" => {
                    let _ = crate::fresh(&state).snooze_default(
                        &occurrence_id,
                        SnoozeVia::Button,
                        crate::now(),
                    );
                }
                "skip" => {
                    let _ = crate::fresh(&state).skip(&occurrence_id, None, crate::now());
                }
                "acknowledge" => {
                    let _ = crate::fresh(&state).acknowledge(&occurrence_id, crate::now());
                }
                "default" => {
                    crate::show_window(&app);
                    let _ = app.emit("open", &occurrence_id);
                }
                _ => {}
            }
            // any button, or closing the notification, silences the alarm window and sound
            if action != "default" {
                silence(&app, &occurrence_id);
            }
            let _ = app.emit("changed", ());
        });
        alive.store(false, Ordering::SeqCst);
    });
}
