//! Linux notifications with buttons, over D-Bus (`org.freedesktop.Notifications`).
//!
//! Tauri's notification plugin can't carry action buttons, so alerts go out
//! through `zbus` directly. `zbus` is already in the build (the single-instance
//! plugin and the Secret Service client use it), it gives one connection that
//! can both call `Notify` and listen for `ActionInvoked` and
//! `NotificationClosed`, and it can read the server's `Inhibited` property
//! (Do Not Disturb). `notify-rust`, which the plugin used, only reports an
//! action by blocking a thread per notification and has none of the rest.
//!
//! The split:
//! - [`Notifier`] is the one-method-per-D-Bus-call seam. [`DbusNotifier`] is the
//!   real one; the tests use a fake.
//! - [`Delivery`] carries out the [`Command`]s that `hab_core::Alerter` decides
//!   on, remembers which notification id belongs to which occurrence (so a
//!   repeat replaces the one on screen), and turns signals into
//!   [`UserAction`]s.
//! - [`listen`] runs a thread that turns the server's signals into
//!   [`Signal`]s.
//!
//! What each part of the freedesktop.org spec is used for:
//! - actions: `default` (a click on the notification), `done` and `skip`;
//! - hints: `urgency` (low for silent, normal otherwise), `desktop-entry`,
//!   `sound-name` from the sound theme (or `suppress-sound`), no `resident`
//!   (so the server closes it after an action) and no `transient` (so a gentle
//!   one settles into the server's list);
//! - `expire_timeout`, so a gentle notification times out;
//! - `Inhibited`, which Plasma implements. GNOME doesn't expose it, so there
//!   its "Do Not Disturb" switch (`show-banners` off) is read from gsettings.

use std::collections::HashMap;
use std::sync::Mutex;

use hab_core::{Command, Notification, Urgency, ACTION_DONE, ACTION_OPEN, ACTION_SKIP};

/// The name the notification shows for the app.
const APP_NAME: &str = "Reminders";
/// The desktop file the notification belongs to, which is also the Tauri
/// identifier.
const DESKTOP_ENTRY: &str = "io.github.csnook.hab-bot";

/// What the app can ask of a notification server.
pub trait Notifier: Send {
    /// Shows a notification, replacing `replaces` if given. Returns its id.
    fn show(&mut self, n: &Notification, replaces: Option<u32>) -> Result<u32, String>;
    fn close(&mut self, id: u32);
    /// Whether the server is inhibited: Do Not Disturb.
    fn inhibited(&mut self) -> bool;
}

/// What the server tells us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    ActionInvoked {
        id: u32,
        key: String,
    },
    Closed {
        id: u32,
    },
    /// The server's properties changed, which may be Do Not Disturb.
    PropertiesChanged,
}

/// What the user did with a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    Done(String),
    Skip(String),
    /// Clicked: open the occurrence.
    Open(String),
}

#[derive(Default)]
struct Ids {
    by_occurrence: HashMap<String, u32>,
    by_id: HashMap<u32, String>,
}

impl Ids {
    fn forget(&mut self, occurrence_id: &str) -> Option<u32> {
        let id = self.by_occurrence.remove(occurrence_id)?;
        self.by_id.remove(&id);
        Some(id)
    }
}

/// Carries out alert commands and understands what comes back.
pub struct Delivery {
    notifier: Mutex<Box<dyn Notifier>>,
    ids: Mutex<Ids>,
}

impl Delivery {
    pub fn new(notifier: Box<dyn Notifier>) -> Self {
        Delivery {
            notifier: Mutex::new(notifier),
            ids: Mutex::new(Ids::default()),
        }
    }

    pub fn inhibited(&self) -> bool {
        self.notifier.lock().unwrap().inhibited()
    }

    pub fn apply(&self, commands: &[Command]) {
        for c in commands {
            match c {
                Command::Show(n) => {
                    let replaces = self
                        .ids
                        .lock()
                        .unwrap()
                        .by_occurrence
                        .get(&n.occurrence_id)
                        .copied();
                    let shown = self.notifier.lock().unwrap().show(n, replaces);
                    match shown {
                        Ok(id) => {
                            let mut ids = self.ids.lock().unwrap();
                            if let Some(old) = ids.by_occurrence.insert(n.occurrence_id.clone(), id)
                            {
                                ids.by_id.remove(&old);
                            }
                            ids.by_id.insert(id, n.occurrence_id.clone());
                        }
                        Err(e) => eprintln!("notification failed: {e}"),
                    }
                }
                Command::Close { occurrence_id } => {
                    let id = self.ids.lock().unwrap().forget(occurrence_id);
                    if let Some(id) = id {
                        self.notifier.lock().unwrap().close(id);
                    }
                }
            }
        }
    }

    /// What a signal means for the app, if it is about one of our
    /// notifications. A closed notification is forgotten: dismissing it is
    /// not acting on the occurrence, so the next repeat shows a new one.
    pub fn on_signal(&self, signal: &Signal) -> Option<UserAction> {
        let mut ids = self.ids.lock().unwrap();
        match signal {
            Signal::ActionInvoked { id, key } => {
                let occurrence = ids.by_id.get(id)?.clone();
                match key.as_str() {
                    ACTION_DONE => Some(UserAction::Done(occurrence)),
                    ACTION_SKIP => Some(UserAction::Skip(occurrence)),
                    ACTION_OPEN => Some(UserAction::Open(occurrence)),
                    _ => None,
                }
            }
            Signal::Closed { id } => {
                if let Some(occurrence) = ids.by_id.remove(id) {
                    ids.by_occurrence.remove(&occurrence);
                }
                None
            }
            Signal::PropertiesChanged => None,
        }
    }
}

/// The hints, actions and timeout `Notify` is called with.
pub fn hints_of(n: &Notification) -> Vec<(&'static str, Hint)> {
    let urgency = match n.urgency {
        Urgency::Low => 0,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    };
    let mut hints = vec![
        ("urgency", Hint::Byte(urgency)),
        ("desktop-entry", Hint::Str(DESKTOP_ENTRY)),
    ];
    match n.sound {
        Some(name) => hints.push(("sound-name", Hint::Str(name))),
        None => hints.push(("suppress-sound", Hint::Bool(true))),
    }
    hints
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hint {
    Byte(u8),
    Str(&'static str),
    Bool(bool),
}

/// The flat list `Notify` wants: key, label, key, label...
pub fn actions_of(n: &Notification) -> Vec<&'static str> {
    let mut v = vec![ACTION_OPEN, "Open"];
    for (key, label) in &n.actions {
        v.push(key);
        v.push(label);
    }
    v
}

/// Reads "true" or "false" as gsettings prints a boolean.
pub fn parse_gsettings_bool(out: &str) -> Option<bool> {
    match out.trim() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";

/// The real notification server, over the session bus.
pub struct DbusNotifier {
    /// A bus address to use instead of the session bus (tests).
    address: Option<String>,
    conn: Option<zbus::blocking::Connection>,
}

impl DbusNotifier {
    pub fn session() -> Self {
        DbusNotifier {
            address: None,
            conn: None,
        }
    }

    #[cfg(test)]
    pub fn at(address: &str) -> Self {
        DbusNotifier {
            address: Some(address.to_string()),
            conn: None,
        }
    }

    fn conn(&mut self) -> Result<&zbus::blocking::Connection, String> {
        if self.conn.is_none() {
            self.conn = Some(connect(self.address.as_deref())?);
        }
        Ok(self.conn.as_ref().expect("just connected"))
    }
}

fn connect(address: Option<&str>) -> Result<zbus::blocking::Connection, String> {
    match address {
        Some(a) => zbus::blocking::connection::Builder::address(a)
            .and_then(|b| b.build())
            .map_err(|e| e.to_string()),
        None => zbus::blocking::Connection::session().map_err(|e| e.to_string()),
    }
}

impl Notifier for DbusNotifier {
    fn show(&mut self, n: &Notification, replaces: Option<u32>) -> Result<u32, String> {
        use zbus::zvariant::Value;
        let hints: HashMap<&str, Value<'_>> = hints_of(n)
            .into_iter()
            .map(|(k, h)| {
                let v = match h {
                    Hint::Byte(b) => Value::from(b),
                    Hint::Str(s) => Value::from(s),
                    Hint::Bool(b) => Value::from(b),
                };
                (k, v)
            })
            .collect();
        let body = (
            APP_NAME,
            replaces.unwrap_or(0),
            "",
            n.title.as_str(),
            n.body.as_str(),
            actions_of(n),
            hints,
            n.timeout_ms,
        );
        let result = self
            .conn()?
            .call_method(Some(DEST), PATH, Some(DEST), "Notify", &body)
            .and_then(|m| m.body().deserialize::<u32>());
        if result.is_err() {
            // The server may have restarted: connect afresh next time.
            self.conn = None;
        }
        result.map_err(|e| e.to_string())
    }

    fn close(&mut self, id: u32) {
        if let Ok(conn) = self.conn() {
            let _ = conn.call_method(Some(DEST), PATH, Some(DEST), "CloseNotification", &(id,));
        }
    }

    fn inhibited(&mut self) -> bool {
        let property = self.conn().ok().and_then(|conn| {
            conn.call_method(
                Some(DEST),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(DEST, "Inhibited"),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<zbus::zvariant::OwnedValue>().ok())
            .and_then(|v| bool::try_from(v).ok())
        });
        match property {
            Some(b) => b,
            None => gnome_do_not_disturb(),
        }
    }
}

/// GNOME doesn't implement `Inhibited`; its Do Not Disturb switch is the
/// `show-banners` setting.
fn gnome_do_not_disturb() -> bool {
    let gnome = std::env::var("XDG_CURRENT_DESKTOP").is_ok_and(|d| d.contains("GNOME"));
    if !gnome {
        return false;
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.notifications", "show-banners"])
        .output()
        .ok()
        .and_then(|o| parse_gsettings_bool(&String::from_utf8_lossy(&o.stdout)))
        .is_some_and(|banners| !banners)
}

/// Starts a thread that reports the notification server's signals until the
/// bus goes away. `address` is for tests; `None` is the session bus.
pub fn listen(
    address: Option<String>,
    mut on_signal: impl FnMut(Signal) + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<()>> {
    std::thread::Builder::new()
        .name("notification-signals".into())
        .spawn(move || {
            if let Err(e) = listen_loop(address.as_deref(), &mut on_signal) {
                eprintln!("not listening to notification signals: {e}");
            }
        })
}

fn listen_loop(address: Option<&str>, on_signal: &mut dyn FnMut(Signal)) -> Result<(), String> {
    let conn = connect(address)?;
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .path(PATH)
        .map_err(|e| e.to_string())?
        .build();
    let messages = zbus::blocking::MessageIterator::for_match_rule(rule, &conn, None)
        .map_err(|e| e.to_string())?;
    for message in messages {
        let Ok(message) = message else { continue };
        let header = message.header();
        let member = header.member().map(|m| m.as_str().to_string());
        let signal = match member.as_deref() {
            Some("ActionInvoked") => message
                .body()
                .deserialize::<(u32, String)>()
                .ok()
                .map(|(id, key)| Signal::ActionInvoked { id, key }),
            Some("NotificationClosed") => message
                .body()
                .deserialize::<(u32, u32)>()
                .ok()
                .map(|(id, _reason)| Signal::Closed { id }),
            Some("PropertiesChanged") => Some(Signal::PropertiesChanged),
            _ => None,
        };
        if let Some(signal) = signal {
            on_signal(signal);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hab_core::AlertStyle;

    #[derive(Default)]
    struct Fake {
        shown: Vec<(u32, String, Option<u32>)>,
        closed: Vec<u32>,
        next: u32,
    }

    #[derive(Clone, Default)]
    struct SharedFake(std::sync::Arc<Mutex<Fake>>);

    impl Notifier for SharedFake {
        fn show(&mut self, n: &Notification, replaces: Option<u32>) -> Result<u32, String> {
            let mut f = self.0.lock().unwrap();
            f.next += 1;
            let id = f.next;
            f.shown.push((id, n.occurrence_id.clone(), replaces));
            Ok(id)
        }
        fn close(&mut self, id: u32) {
            self.0.lock().unwrap().closed.push(id);
        }
        fn inhibited(&mut self) -> bool {
            false
        }
    }

    fn note(occ: &str, style: AlertStyle) -> Notification {
        Notification {
            occurrence_id: occ.into(),
            title: "Water".into(),
            body: "Due".into(),
            style,
            urgency: Urgency::Normal,
            sound: Some("message-new-instant"),
            timeout_ms: 10_000,
            actions: vec![(ACTION_DONE, "Done"), (ACTION_SKIP, "Skip")],
        }
    }

    fn delivery() -> (Delivery, SharedFake) {
        let fake = SharedFake::default();
        (Delivery::new(Box::new(fake.clone())), fake)
    }

    fn action(id: u32, key: &str) -> Signal {
        Signal::ActionInvoked {
            id,
            key: key.into(),
        }
    }

    #[test]
    fn buttons_and_clicks_map_back_to_the_occurrence() {
        let (d, _) = delivery();
        d.apply(&[Command::Show(note("o1", AlertStyle::Gentle))]);
        assert_eq!(
            d.on_signal(&action(1, "done")),
            Some(UserAction::Done("o1".into()))
        );
        assert_eq!(
            d.on_signal(&action(1, "skip")),
            Some(UserAction::Skip("o1".into()))
        );
        assert_eq!(
            d.on_signal(&action(1, "default")),
            Some(UserAction::Open("o1".into()))
        );
        assert_eq!(d.on_signal(&action(1, "other")), None);
        // A notification that isn't ours is ignored.
        assert_eq!(d.on_signal(&action(99, "done")), None);
    }

    #[test]
    fn a_repeat_replaces_the_notification_on_screen() {
        let (d, fake) = delivery();
        d.apply(&[Command::Show(note("o1", AlertStyle::Insistent))]);
        d.apply(&[Command::Show(note("o1", AlertStyle::Insistent))]);
        let shown = fake.0.lock().unwrap().shown.clone();
        assert_eq!(shown[0].2, None);
        assert_eq!(shown[1].2, Some(1));
        // Only the latest id is ours now.
        assert_eq!(d.on_signal(&action(1, "done")), None);
        assert_eq!(
            d.on_signal(&action(2, "done")),
            Some(UserAction::Done("o1".into()))
        );
    }

    #[test]
    fn a_dismissed_notification_is_forgotten_so_the_next_alert_is_new() {
        let (d, fake) = delivery();
        d.apply(&[Command::Show(note("o1", AlertStyle::Insistent))]);
        assert_eq!(d.on_signal(&Signal::Closed { id: 1 }), None);
        d.apply(&[Command::Show(note("o1", AlertStyle::Insistent))]);
        assert_eq!(fake.0.lock().unwrap().shown[1].2, None);
    }

    #[test]
    fn closing_takes_the_notification_down() {
        let (d, fake) = delivery();
        d.apply(&[Command::Show(note("o1", AlertStyle::Gentle))]);
        d.apply(&[Command::Close {
            occurrence_id: "o1".into(),
        }]);
        assert_eq!(fake.0.lock().unwrap().closed, [1]);
        d.apply(&[Command::Close {
            occurrence_id: "o1".into(),
        }]);
        assert_eq!(fake.0.lock().unwrap().closed, [1], "nothing left to close");
    }

    #[test]
    fn hints_follow_the_style() {
        let silent = Notification {
            urgency: Urgency::Low,
            sound: None,
            ..note("o", AlertStyle::Silent)
        };
        let h = hints_of(&silent);
        assert!(h.contains(&("urgency", Hint::Byte(0))));
        assert!(h.contains(&("suppress-sound", Hint::Bool(true))));
        assert!(!h.iter().any(|(k, _)| *k == "sound-name"));
        let gentle = hints_of(&note("o", AlertStyle::Gentle));
        assert!(gentle.contains(&("urgency", Hint::Byte(1))));
        assert!(gentle.contains(&("sound-name", Hint::Str("message-new-instant"))));
        assert!(gentle.contains(&("desktop-entry", Hint::Str("io.github.csnook.hab-bot"))));
        assert!(!gentle
            .iter()
            .any(|(k, _)| *k == "resident" || *k == "transient"));
    }

    #[test]
    fn actions_are_a_click_then_done_and_skip() {
        assert_eq!(
            actions_of(&note("o", AlertStyle::Gentle)),
            ["default", "Open", "done", "Done", "skip", "Skip"]
        );
    }

    #[test]
    fn gsettings_booleans_are_read() {
        assert_eq!(parse_gsettings_bool("true\n"), Some(true));
        assert_eq!(parse_gsettings_bool("false\n"), Some(false));
        assert_eq!(parse_gsettings_bool("oops"), None);
    }

    /// A real round trip over a private session bus: a fake notification
    /// server records the `Notify` calls the app makes, answers the
    /// `Inhibited` property, and sends the `ActionInvoked` signals a click
    /// would. Skipped when `dbus-daemon` isn't installed.
    mod over_dbus {
        use super::*;
        use std::io::{BufRead, BufReader};
        use std::process::{Child, Stdio};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::mpsc;
        use std::sync::Arc;
        use std::time::Duration;
        use zbus::zvariant::OwnedValue;

        #[derive(Debug, Clone)]
        struct Call {
            app_name: String,
            replaces: u32,
            summary: String,
            actions: Vec<String>,
            urgency: Option<u8>,
            sound_name: Option<String>,
            suppress_sound: Option<bool>,
            desktop_entry: Option<String>,
            has_resident_or_transient: bool,
            timeout: i32,
        }

        struct Server {
            calls: Arc<Mutex<Vec<Call>>>,
            inhibited: Arc<AtomicBool>,
            closed: Arc<Mutex<Vec<u32>>>,
        }

        #[zbus::interface(name = "org.freedesktop.Notifications")]
        impl Server {
            #[allow(clippy::too_many_arguments)]
            fn notify(
                &self,
                app_name: String,
                replaces_id: u32,
                _icon: String,
                summary: String,
                _body: String,
                actions: Vec<String>,
                hints: HashMap<String, OwnedValue>,
                timeout: i32,
            ) -> u32 {
                let mut calls = self.calls.lock().unwrap();
                let call = Call {
                    app_name,
                    replaces: replaces_id,
                    summary,
                    actions,
                    urgency: hints.get("urgency").and_then(|v| u8::try_from(v).ok()),
                    sound_name: hints
                        .get("sound-name")
                        .and_then(|v| String::try_from(v.try_clone().ok()?).ok()),
                    suppress_sound: hints
                        .get("suppress-sound")
                        .and_then(|v| bool::try_from(v).ok()),
                    desktop_entry: hints
                        .get("desktop-entry")
                        .and_then(|v| String::try_from(v.try_clone().ok()?).ok()),
                    has_resident_or_transient: hints.contains_key("resident")
                        || hints.contains_key("transient"),
                    timeout,
                };
                calls.push(call);
                calls.len() as u32 + 100
            }

            fn close_notification(&self, id: u32) {
                self.closed.lock().unwrap().push(id);
            }

            fn get_capabilities(&self) -> Vec<String> {
                vec!["actions".into(), "sound".into()]
            }

            #[zbus(property)]
            fn inhibited(&self) -> bool {
                self.inhibited.load(Ordering::SeqCst)
            }
        }

        struct Daemon(Child);
        impl Drop for Daemon {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }

        fn start_bus() -> Option<(Daemon, String)> {
            let mut child = std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(Stdio::piped())
                .spawn()
                .ok()?;
            let mut line = String::new();
            BufReader::new(child.stdout.take()?)
                .read_line(&mut line)
                .ok()?;
            Some((Daemon(child), line.trim().to_string()))
        }

        #[test]
        fn notifications_buttons_and_do_not_disturb_work_over_the_bus() {
            let Some((_bus, address)) = start_bus() else {
                eprintln!("skipped: no dbus-daemon");
                return;
            };
            let calls = Arc::new(Mutex::new(Vec::new()));
            let closed = Arc::new(Mutex::new(Vec::new()));
            let inhibited = Arc::new(AtomicBool::new(false));
            let server = zbus::blocking::connection::Builder::address(address.as_str())
                .unwrap()
                .name("org.freedesktop.Notifications")
                .unwrap()
                .serve_at(
                    PATH,
                    Server {
                        calls: calls.clone(),
                        inhibited: inhibited.clone(),
                        closed: closed.clone(),
                    },
                )
                .unwrap()
                .build()
                .unwrap();

            let delivery = Arc::new(Delivery::new(Box::new(DbusNotifier::at(&address))));
            let (tx, rx) = mpsc::channel();
            let d = delivery.clone();
            listen(Some(address.clone()), move |signal| {
                if let Some(action) = d.on_signal(&signal) {
                    let _ = tx.send(action);
                }
            })
            .unwrap();
            std::thread::sleep(Duration::from_millis(500));

            // A gentle alert from a real core, through the alerter.
            let mut core = hab_core::Core::open_in_memory().unwrap();
            let t0 = 1_790_000_000;
            let id = core
                .create_reminder("Water the plants", t0, t0 - 60)
                .unwrap();
            core.edit_reminder(
                &id,
                hab_core::EditReminder {
                    priority: Some(hab_core::Priority::Low),
                    ..Default::default()
                },
                t0 - 60,
            )
            .unwrap();
            core.tick(t0).unwrap();
            let occurrence = format!("{id}@{t0}");
            let mut alerter = hab_core::Alerter::new();
            let pass = alerter.pass(&mut core, t0, delivery.inhibited()).unwrap();
            delivery.apply(&pass.commands);

            let call = calls.lock().unwrap()[0].clone();
            assert_eq!(call.app_name, "Reminders");
            assert_eq!(call.summary, "Water the plants");
            assert_eq!(call.replaces, 0);
            assert_eq!(
                call.actions,
                ["default", "Open", "done", "Done", "skip", "Skip"]
            );
            assert_eq!(call.urgency, Some(1));
            assert_eq!(call.sound_name.as_deref(), Some("message-new-instant"));
            assert_eq!(call.suppress_sound, None);
            assert_eq!(
                call.desktop_entry.as_deref(),
                Some("io.github.csnook.hab-bot")
            );
            assert!(!call.has_resident_or_transient);
            assert_eq!(call.timeout, 10_000);

            // Clicking Done and the notification itself come back as actions.
            let emit = |key: &str| {
                server
                    .emit_signal(None::<&str>, PATH, DEST, "ActionInvoked", &(101u32, key))
                    .unwrap()
            };
            emit("done");
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                UserAction::Done(occurrence.clone())
            );
            emit("default");
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                UserAction::Open(occurrence.clone())
            );

            // Do Not Disturb is the server's Inhibited property.
            assert!(!delivery.inhibited());
            inhibited.store(true, Ordering::SeqCst);
            assert!(delivery.inhibited());

            // A silent alert is low urgency with the sound suppressed, and
            // closing takes the notification down at the server.
            let silent = Notification {
                urgency: Urgency::Low,
                sound: None,
                ..note("o2", hab_core::AlertStyle::Silent)
            };
            delivery.apply(&[Command::Show(silent)]);
            let call = calls.lock().unwrap()[1].clone();
            assert_eq!(call.urgency, Some(0));
            assert_eq!(call.sound_name, None);
            assert_eq!(call.suppress_sound, Some(true));
            delivery.apply(&[Command::Close {
                occurrence_id: "o2".into(),
            }]);
            assert_eq!(*closed.lock().unwrap(), [102]);
        }
    }
}
