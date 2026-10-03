//! Linux alerts: our own `org.freedesktop.Notifications` notifications, since Tauri's have
//! no buttons. Plasma and GNOME both implement the actions, urgency and sound hints used
//! here. What to show and when is decided by the core's `AlertEngine`.

use crate::App;
use hab_core::{Alert, AlertKind, AlertStyle, SnoozeVia};
use notify_rust::{Hint, Notification, Timeout, Urgency};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

const BUS_NAME: &str = "org.freedesktop.Notifications";
const OBJECT: &str = "/org/freedesktop/Notifications";

/// The notifications on screen, by occurrence, and whether something is still listening for
/// their buttons.
#[derive(Default)]
pub struct Shown {
    by_occurrence: HashMap<String, (u32, Arc<AtomicBool>)>,
}

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

fn close(id: u32) {
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

/// Takes down the notification of an occurrence that closed.
pub fn dismiss(shown: &mut Shown, occurrence_id: &str) {
    if let Some((id, _)) = shown.by_occurrence.remove(occurrence_id) {
        close(id);
    }
}

/// Shows or updates the notification for an alert.
///
/// - Silent: low urgency, no sound.
/// - Gentle: normal urgency, a sound from the theme, and it times out.
/// - Insistent: gentle, repeated by the engine every interval.
/// - Alarm: critical, never times out. The alarm window and looping sound come later.
pub fn show(app: &AppHandle, shown: &mut Shown, alert: &Alert) {
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
        .action("snooze", "Snooze")
        .action("skip", "Skip")
        .action("default", "Open");
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
            n.urgency(Urgency::Critical)
                .hint(Hint::SoundName("alarm-clock-elapsed".into()))
                .hint(Hint::Resident(true))
                .timeout(Timeout::Never);
        }
    }
    // Replace the notification already showing for this occurrence.
    if let Some((id, _)) = shown.by_occurrence.get(&alert.occurrence_id) {
        n.id(*id);
    }
    let Ok(handle) = n.show() else { return };
    let id = handle.id();
    let listening = shown
        .by_occurrence
        .get(&alert.occurrence_id)
        .map(|(_, alive)| alive.clone())
        .filter(|alive| alive.load(Ordering::SeqCst));
    if let Some(alive) = listening {
        // the earlier handle's thread still listens for this id
        shown
            .by_occurrence
            .insert(alert.occurrence_id.clone(), (id, alive));
        return;
    }
    let alive = Arc::new(AtomicBool::new(true));
    shown
        .by_occurrence
        .insert(alert.occurrence_id.clone(), (id, alive.clone()));
    let (app, occurrence_id) = (app.clone(), alert.occurrence_id.clone());
    // Buttons work with the window closed: the thread acts through the core directly.
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
                "default" => {
                    crate::show_window(&app);
                    let _ = app.emit("open", &occurrence_id);
                }
                _ => {}
            }
            let _ = app.emit("changed", ());
        });
        alive.store(false, Ordering::SeqCst);
    });
}
