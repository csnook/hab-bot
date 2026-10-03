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
//!   [`UserAction`]s. It is also where an alarm happens: the same `Show` that
//!   raises the critical notification starts the looping [`Sound`] and opens
//!   the alarm window, and anything that ends the alarm (the occurrence
//!   closing, an action, closing the notification, closing the window) ends
//!   all three together.
//! - [`listen`] runs a thread that turns the server's signals into
//!   [`Signal`]s.
//!
//! What each part of the freedesktop.org spec is used for:
//! - actions: `default` (a click on the notification), `done`, `snooze` and
//!   `skip` (or, on an alarm, `acknowledge`);
//! - hints: `urgency` (low for silent, normal for gentle and insistent,
//!   critical for an alarm), `desktop-entry`, `sound-name` from the sound
//!   theme (or `suppress-sound`, which an alarm always sets because the app
//!   plays its own looping sound), no `resident` (so the server closes it
//!   after an action) and no `transient` (so a gentle one settles into the
//!   server's list);
//! - `expire_timeout`: a gentle notification times out, an alarm never does;
//! - the `ActivationToken` signal, which gives the xdg-activation token of
//!   the click on a notification, so that opening the alarm window from it is
//!   allowed to take focus on Wayland;
//! - `Inhibited`, which Plasma implements. GNOME doesn't expose it, so there
//!   its "Do Not Disturb" switch (`show-banners` off) is read from gsettings.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

use hab_core::{
    AlertStyle, Command, Notification, Urgency, ACTION_ACKNOWLEDGE, ACTION_DONE, ACTION_OPEN,
    ACTION_SKIP, ACTION_SNOOZE,
};

use crate::alarm::{Activation, AlarmWindows};
use crate::sound::Sound;

/// The name the notification shows for the app.
const APP_NAME: &str = "Reminders";
/// The desktop file the notification belongs to, which is also the Tauri
/// identifier.
pub const DESKTOP_ENTRY: &str = "io.github.csnook.hab-bot";

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
    /// The xdg-activation token of the click that is about to invoke an
    /// action on the notification.
    ActivationToken {
        id: u32,
        token: String,
    },
    /// The server's properties changed, which may be Do Not Disturb.
    PropertiesChanged,
}

/// What the user did with a notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserAction {
    Done(String),
    Skip(String),
    /// One tap on Snooze: for the priority's snooze length.
    Snooze(String),
    /// Silence the current alert without closing the occurrence.
    Acknowledge(String),
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

/// Which occurrences have an alarm up, and which of those are ringing.
#[derive(Default)]
struct Alarms {
    /// Alarms with a notification and a window up.
    up: HashSet<String>,
    /// Alarms whose sound is on. One loop serves them all.
    ringing: HashSet<String>,
}

/// Carries out alert commands and understands what comes back.
pub struct Delivery {
    notifier: Mutex<Box<dyn Notifier>>,
    ids: Mutex<Ids>,
    alarms: Mutex<Alarms>,
    sound: Mutex<Box<dyn Sound>>,
    windows: Box<dyn AlarmWindows>,
    activation: Box<dyn Activation>,
    /// The token of the last click on one of our notifications, until the
    /// action it led to has used it.
    click_token: Mutex<Option<String>>,
}

impl Delivery {
    pub fn new(
        notifier: Box<dyn Notifier>,
        sound: Box<dyn Sound>,
        windows: Box<dyn AlarmWindows>,
        activation: Box<dyn Activation>,
    ) -> Self {
        Delivery {
            notifier: Mutex::new(notifier),
            ids: Mutex::new(Ids::default()),
            alarms: Mutex::new(Alarms::default()),
            sound: Mutex::new(sound),
            windows,
            activation,
            click_token: Mutex::new(None),
        }
    }

    pub fn inhibited(&self) -> bool {
        self.notifier.lock().unwrap().inhibited()
    }

    pub fn apply(&self, commands: &[Command]) {
        for c in commands {
            match c {
                Command::Show(n) => {
                    self.show(n);
                    if n.style == AlertStyle::Alarm {
                        self.ring(&n.occurrence_id);
                    } else {
                        // An alarm that is now quieter (Do Not Disturb came
                        // on) is over: its sound and window go.
                        self.end_alarm(&n.occurrence_id);
                    }
                }
                Command::Close { occurrence_id } => {
                    let id = self.ids.lock().unwrap().forget(occurrence_id);
                    if let Some(id) = id {
                        self.notifier.lock().unwrap().close(id);
                    }
                    self.end_alarm(occurrence_id);
                }
                Command::StopRinging { occurrence_id } => {
                    self.alarms.lock().unwrap().ringing.remove(occurrence_id);
                    self.update_sound();
                }
            }
        }
    }

    fn show(&self, n: &Notification) {
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
                if let Some(old) = ids.by_occurrence.insert(n.occurrence_id.clone(), id) {
                    ids.by_id.remove(&old);
                }
                ids.by_id.insert(id, n.occurrence_id.clone());
            }
            Err(e) => eprintln!("notification failed: {e}"),
        }
    }

    /// An alarm alerts: the sound loops, and the window comes up (and is
    /// raised again at each repeat).
    fn ring(&self, occurrence_id: &str) {
        {
            let mut alarms = self.alarms.lock().unwrap();
            alarms.up.insert(occurrence_id.to_string());
            alarms.ringing.insert(occurrence_id.to_string());
        }
        self.update_sound();
        let token = self.activation.token();
        self.windows.open(occurrence_id, token.as_deref());
    }

    /// Turns the sound on while any alarm rings, and off when none does.
    fn update_sound(&self) {
        let ringing = !self.alarms.lock().unwrap().ringing.is_empty();
        let mut sound = self.sound.lock().unwrap();
        if ringing {
            sound.start();
        } else {
            sound.stop();
        }
    }

    /// Ends an alarm's sound and window (not its notification).
    fn end_alarm(&self, occurrence_id: &str) {
        let was_up = {
            let mut alarms = self.alarms.lock().unwrap();
            alarms.ringing.remove(occurrence_id);
            alarms.up.remove(occurrence_id)
        };
        self.update_sound();
        if was_up {
            self.windows.close(occurrence_id);
        }
    }

    /// Closing the alarm window or its notification silences both, and the
    /// sound. It isn't acting on the occurrence: the alerter rings again at
    /// the next repeat. `window_closed` is true when the window is already
    /// going away.
    pub fn silence(&self, occurrence_id: &str, window_closed: bool) {
        let id = self.ids.lock().unwrap().forget(occurrence_id);
        if let Some(id) = id {
            self.notifier.lock().unwrap().close(id);
        }
        if window_closed {
            let mut alarms = self.alarms.lock().unwrap();
            alarms.up.remove(occurrence_id);
            alarms.ringing.remove(occurrence_id);
            drop(alarms);
            self.update_sound();
        } else {
            self.end_alarm(occurrence_id);
        }
    }

    /// Whether an alarm for the occurrence is up.
    pub fn is_alarm(&self, occurrence_id: &str) -> bool {
        self.alarms.lock().unwrap().up.contains(occurrence_id)
    }

    /// Raises the occurrence's alarm window, if it has one, with the token of
    /// the click that asked (or a fresh one). Returns whether there was one.
    pub fn raise_alarm(&self, occurrence_id: &str) -> bool {
        if !self.is_alarm(occurrence_id) {
            return false;
        }
        let token = self.take_click_token().or_else(|| self.activation.token());
        self.windows.open(occurrence_id, token.as_deref());
        true
    }

    /// The xdg-activation token of the last click on a notification, once.
    pub fn take_click_token(&self) -> Option<String> {
        self.click_token.lock().unwrap().take()
    }

    /// What a signal means for the app, if it is about one of our
    /// notifications. A closed notification is forgotten: dismissing it is
    /// not acting on the occurrence, so the next repeat shows a new one. If
    /// it was an alarm's, the sound stops and the window closes with it.
    pub fn on_signal(&self, signal: &Signal) -> Option<UserAction> {
        match signal {
            Signal::ActionInvoked { id, key } => {
                let occurrence = self.ids.lock().unwrap().by_id.get(id)?.clone();
                match key.as_str() {
                    ACTION_DONE => Some(UserAction::Done(occurrence)),
                    ACTION_SKIP => Some(UserAction::Skip(occurrence)),
                    ACTION_SNOOZE => Some(UserAction::Snooze(occurrence)),
                    ACTION_ACKNOWLEDGE => Some(UserAction::Acknowledge(occurrence)),
                    ACTION_OPEN => Some(UserAction::Open(occurrence)),
                    _ => None,
                }
            }
            Signal::Closed { id } => {
                let occurrence = {
                    let mut ids = self.ids.lock().unwrap();
                    let occurrence = ids.by_id.remove(id)?;
                    ids.by_occurrence.remove(&occurrence);
                    occurrence
                };
                if self.is_alarm(&occurrence) {
                    self.end_alarm(&occurrence);
                }
                None
            }
            Signal::ActivationToken { id, token } => {
                if self.ids.lock().unwrap().by_id.contains_key(id) {
                    *self.click_token.lock().unwrap() = Some(token.clone());
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
            Some("ActivationToken") => message
                .body()
                .deserialize::<(u32, String)>()
                .ok()
                .map(|(id, token)| Signal::ActivationToken { id, token }),
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
    use std::sync::Arc;

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
        if style == AlertStyle::Alarm {
            return alarm(occ);
        }
        Notification {
            occurrence_id: occ.into(),
            title: "Water".into(),
            body: "Due".into(),
            style,
            urgency: Urgency::Normal,
            sound: Some("message-new-instant"),
            timeout_ms: 10_000,
            actions: vec![
                (ACTION_DONE, "Done"),
                (ACTION_SNOOZE, "Snooze"),
                (ACTION_SKIP, "Skip"),
            ],
        }
    }

    fn alarm(occ: &str) -> Notification {
        Notification {
            occurrence_id: occ.into(),
            title: "Meds".into(),
            body: "Overdue".into(),
            style: AlertStyle::Alarm,
            urgency: Urgency::Critical,
            sound: None,
            timeout_ms: 0,
            actions: vec![
                (ACTION_DONE, "Done"),
                (ACTION_SNOOZE, "Snooze"),
                (ACTION_ACKNOWLEDGE, "Acknowledge"),
            ],
        }
    }

    /// What the fake sound and window were asked, in order, plus tokens.
    #[derive(Default)]
    struct Log {
        events: Vec<String>,
        playing: bool,
    }

    #[derive(Clone, Default)]
    struct Platform(Arc<Mutex<Log>>);

    impl Platform {
        fn events(&self) -> Vec<String> {
            self.0.lock().unwrap().events.clone()
        }
        fn playing(&self) -> bool {
            self.0.lock().unwrap().playing
        }
    }

    struct FakeSound(Platform);
    impl Sound for FakeSound {
        fn start(&mut self) {
            let mut l = (self.0).0.lock().unwrap();
            if !l.playing {
                l.events.push("sound start".into());
            }
            l.playing = true;
        }
        fn stop(&mut self) {
            let mut l = (self.0).0.lock().unwrap();
            if l.playing {
                l.events.push("sound stop".into());
            }
            l.playing = false;
        }
    }

    struct FakeWindows(Platform);
    impl AlarmWindows for FakeWindows {
        fn open(&self, occurrence_id: &str, token: Option<&str>) {
            (self.0)
                .0
                .lock()
                .unwrap()
                .events
                .push(format!("window open {occurrence_id} token={token:?}"));
        }
        fn close(&self, occurrence_id: &str) {
            (self.0)
                .0
                .lock()
                .unwrap()
                .events
                .push(format!("window close {occurrence_id}"));
        }
    }

    struct FakeActivation(Option<&'static str>);
    impl Activation for FakeActivation {
        fn token(&self) -> Option<String> {
            self.0.map(str::to_string)
        }
    }

    fn delivery() -> (Delivery, SharedFake) {
        let (d, fake, _) = delivery_with_platform(Some("fresh-token"));
        (d, fake)
    }

    fn delivery_with_platform(token: Option<&'static str>) -> (Delivery, SharedFake, Platform) {
        let fake = SharedFake::default();
        let platform = Platform::default();
        let d = Delivery::new(
            Box::new(fake.clone()),
            Box::new(FakeSound(platform.clone())),
            Box::new(FakeWindows(platform.clone())),
            Box::new(FakeActivation(token)),
        );
        (d, fake, platform)
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
            d.on_signal(&action(1, "snooze")),
            Some(UserAction::Snooze("o1".into()))
        );
        assert_eq!(
            d.on_signal(&action(1, "acknowledge")),
            Some(UserAction::Acknowledge("o1".into()))
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
    fn actions_are_a_click_then_done_snooze_and_the_third_button() {
        assert_eq!(
            actions_of(&note("o", AlertStyle::Gentle)),
            ["default", "Open", "done", "Done", "snooze", "Snooze", "skip", "Skip"]
        );
        assert_eq!(
            actions_of(&alarm("o")),
            [
                "default",
                "Open",
                "done",
                "Done",
                "snooze",
                "Snooze",
                "acknowledge",
                "Acknowledge"
            ]
        );
    }

    #[test]
    fn an_alarm_is_critical_with_the_sound_suppressed_for_the_apps_own() {
        let h = hints_of(&alarm("o"));
        assert!(h.contains(&("urgency", Hint::Byte(2))));
        assert!(h.contains(&("suppress-sound", Hint::Bool(true))));
        assert!(!h.iter().any(|(k, _)| *k == "sound-name"));
    }

    #[test]
    fn an_alarm_starts_the_sound_and_opens_the_window_with_the_notification() {
        let (d, fake, platform) = delivery_with_platform(Some("fresh-token"));
        d.apply(&[Command::Show(alarm("o1"))]);
        assert_eq!(fake.0.lock().unwrap().shown.len(), 1);
        assert_eq!(
            platform.events(),
            ["sound start", "window open o1 token=Some(\"fresh-token\")"]
        );
        assert!(d.is_alarm("o1"));
        // A repeat replaces the notification, raises the window again and
        // doesn't start a second sound.
        d.apply(&[Command::Show(alarm("o1"))]);
        assert_eq!(fake.0.lock().unwrap().shown[1].2, Some(1));
        assert_eq!(platform.events().len(), 3);
        assert!(platform.events()[2].starts_with("window open o1"));
        // A gentle alert opens no window and plays no sound of its own.
        d.apply(&[Command::Show(note("o2", AlertStyle::Gentle))]);
        assert_eq!(platform.events().len(), 3);
    }

    #[test]
    fn the_occurrence_closing_ends_the_notification_sound_and_window() {
        let (d2, fake2, p2) = delivery_with_platform(None);
        d2.apply(&[Command::Show(alarm("o1"))]);
        d2.apply(&[Command::Close {
            occurrence_id: "o1".into(),
        }]);
        assert_eq!(fake2.0.lock().unwrap().closed, [1]);
        assert!(!p2.playing());
        assert_eq!(p2.events().last().unwrap(), "window close o1");
        assert!(!d2.is_alarm("o1"));
    }

    #[test]
    fn stop_ringing_ends_only_the_sound() {
        let (d, fake, platform) = delivery_with_platform(None);
        d.apply(&[Command::Show(alarm("o1"))]);
        d.apply(&[Command::StopRinging {
            occurrence_id: "o1".into(),
        }]);
        assert!(!platform.playing());
        assert!(fake.0.lock().unwrap().closed.is_empty());
        assert!(d.is_alarm("o1"), "the window and notification stay up");
        // The next repeat rings again.
        d.apply(&[Command::Show(alarm("o1"))]);
        assert!(platform.playing());
    }

    #[test]
    fn one_sound_serves_every_ringing_alarm_until_the_last_stops() {
        let (d, _, platform) = delivery_with_platform(None);
        d.apply(&[Command::Show(alarm("a")), Command::Show(alarm("b"))]);
        assert_eq!(
            platform
                .events()
                .iter()
                .filter(|e| *e == "sound start")
                .count(),
            1
        );
        d.apply(&[Command::Close {
            occurrence_id: "a".into(),
        }]);
        assert!(platform.playing());
        d.apply(&[Command::Close {
            occurrence_id: "b".into(),
        }]);
        assert!(!platform.playing());
    }

    #[test]
    fn closing_the_notification_silences_the_sound_and_closes_the_window() {
        let (d, _, platform) = delivery_with_platform(None);
        d.apply(&[Command::Show(alarm("o1"))]);
        assert_eq!(d.on_signal(&Signal::Closed { id: 1 }), None);
        assert!(!platform.playing());
        assert_eq!(platform.events().last().unwrap(), "window close o1");
        assert!(!d.is_alarm("o1"));
        // Not an action on the occurrence: the next repeat rings anew.
        d.apply(&[Command::Show(alarm("o1"))]);
        assert!(platform.playing());
    }

    #[test]
    fn closing_the_window_silences_the_sound_and_closes_the_notification() {
        let (d, fake, platform) = delivery_with_platform(None);
        d.apply(&[Command::Show(alarm("o1"))]);
        d.silence("o1", true);
        assert!(!platform.playing());
        assert_eq!(fake.0.lock().unwrap().closed, [1]);
        // The window is already going: it isn't asked to close again.
        assert!(!platform
            .events()
            .iter()
            .any(|e| e.starts_with("window close")));
        assert!(!d.is_alarm("o1"));
        assert_eq!(
            d.on_signal(&action(1, "done")),
            None,
            "notification forgotten"
        );
    }

    #[test]
    fn an_alarm_downgraded_to_silent_ends_the_sound_and_window() {
        let (d, _, platform) = delivery_with_platform(None);
        d.apply(&[Command::Show(alarm("o1"))]);
        let silent = Notification {
            style: AlertStyle::Silent,
            urgency: Urgency::Low,
            sound: None,
            timeout_ms: -1,
            ..note("o1", AlertStyle::Silent)
        };
        d.apply(&[Command::Show(silent)]);
        assert!(!platform.playing());
        assert_eq!(platform.events().last().unwrap(), "window close o1");
    }

    #[test]
    fn clicking_an_alarm_raises_its_window_with_the_clicks_token() {
        let (d, _, platform) = delivery_with_platform(Some("fresh-token"));
        d.apply(&[Command::Show(alarm("o1"))]);
        // The server sends the click's token before the action.
        d.on_signal(&Signal::ActivationToken {
            id: 1,
            token: "click-token".into(),
        });
        assert!(d.raise_alarm("o1"));
        assert_eq!(
            platform.events().last().unwrap(),
            "window open o1 token=Some(\"click-token\")"
        );
        // The token is used once; the next raise falls back to a fresh one.
        assert!(d.raise_alarm("o1"));
        assert_eq!(
            platform.events().last().unwrap(),
            "window open o1 token=Some(\"fresh-token\")"
        );
        // Not an alarm, or not ours: nothing to raise, and no stray token.
        assert!(!d.raise_alarm("other"));
        d.on_signal(&Signal::ActivationToken {
            id: 99,
            token: "x".into(),
        });
        assert_eq!(d.take_click_token(), None);
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

            let platform = Platform::default();
            let delivery = Arc::new(Delivery::new(
                Box::new(DbusNotifier::at(&address)),
                Box::new(FakeSound(platform.clone())),
                Box::new(FakeWindows(platform.clone())),
                Box::new(FakeActivation(None)),
            ));
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
                ["default", "Open", "done", "Done", "snooze", "Snooze", "skip", "Skip"]
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

            // Maximum breaks Do Not Disturb (still inhibited): a critical
            // notification that never expires, with the app's own sound.
            let mut core = hab_core::Core::open_in_memory().unwrap();
            let id = core.create_reminder("Meds", t0, t0 - 60).unwrap();
            core.edit_reminder(
                &id,
                hab_core::EditReminder {
                    priority: Some(hab_core::Priority::Maximum),
                    ..Default::default()
                },
                t0 - 60,
            )
            .unwrap();
            core.tick(t0).unwrap();
            let occurrence = format!("{id}@{t0}");
            let mut alerter = hab_core::Alerter::new();
            assert!(delivery.inhibited());
            let pass = alerter.pass(&mut core, t0, delivery.inhibited()).unwrap();
            delivery.apply(&pass.commands);
            let call = calls.lock().unwrap()[2].clone();
            assert_eq!(call.summary, "Meds");
            assert_eq!(call.urgency, Some(2), "critical");
            assert_eq!(call.timeout, 0, "never expires");
            assert_eq!(call.suppress_sound, Some(true));
            assert_eq!(call.sound_name, None);
            assert_eq!(
                call.actions,
                [
                    "default",
                    "Open",
                    "done",
                    "Done",
                    "snooze",
                    "Snooze",
                    "acknowledge",
                    "Acknowledge"
                ]
            );
            assert!(platform.playing(), "the app plays its own sound");
            assert!(delivery.is_alarm(&occurrence));

            // Acknowledge comes back as an action...
            server
                .emit_signal(
                    None::<&str>,
                    PATH,
                    DEST,
                    "ActionInvoked",
                    &(103u32, "acknowledge"),
                )
                .unwrap();
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(5)).unwrap(),
                UserAction::Acknowledge(occurrence.clone())
            );
            // ...and the server closing the notification (the user swiped or
            // closed it) silences the sound and closes the window.
            server
                .emit_signal(
                    None::<&str>,
                    PATH,
                    DEST,
                    "NotificationClosed",
                    &(103u32, 2u32),
                )
                .unwrap();
            let end = std::time::Instant::now() + Duration::from_secs(5);
            while platform.playing() && std::time::Instant::now() < end {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert!(!platform.playing());
            assert!(!delivery.is_alarm(&occurrence));
        }
    }
}
