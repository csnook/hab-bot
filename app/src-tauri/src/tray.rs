//! The tray icon, as our own StatusNotifierItem over D-Bus.
//!
//! Why not Tauri's tray: on Linux Tauri's tray (`tray-icon` through
//! libappindicator) never reports a click, so "left-click opens the window"
//! can't be built on it (the issue says as much). Tauri exposes no way to
//! switch it to the `ksni` backend that does. So the tray is ours, on `zbus`,
//! which is already in the build for notifications:
//!
//! - `org.kde.StatusNotifierItem` at `/StatusNotifierItem`: the icon (as
//!   pixels, see [`crate::badge`]), the tooltip, and `Activate`, which Plasma
//!   calls on a left click because `ItemIsMenu` is false. `ContextMenu`
//!   (right click) is answered by the host itself from the menu below.
//! - `com.canonical.dbusmenu` at `/MenuBar`: the menu, rebuilt from
//!   [`crate::tray_model`] whenever it changes. Submenus carry each open
//!   occurrence's buttons.
//! - Registration with `org.kde.StatusNotifierWatcher`. Plasma has it
//!   natively; GNOME has it through the AppIndicator extension. At login the
//!   watcher may not exist yet (the research notes `ksni` giving up in that
//!   race), so a thread checks every few seconds and (re-)registers whenever
//!   the watcher has a new owner, which also covers the shell restarting.
//! - `ProvideXdgActivationToken`: Plasma calls it before `Activate` and
//!   before the menu opens. The token is handed on with the action so that
//!   raising the window on Wayland is allowed to take focus. No library does
//!   this for us.
//!
//! The pure parts (what the menu is, the numbering of its items) are tested
//! directly; the D-Bus part is tested against a private bus with a fake
//! watcher and a client that plays the host. It has not been seen on a real
//! Plasma or GNOME panel.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use zbus::zvariant::{OwnedValue, Structure, Type, Value};

use crate::badge;
use crate::tray_model::{Badge, Entry, TrayAction};

pub const ITEM_PATH: &str = "/StatusNotifierItem";
pub const MENU_PATH: &str = "/MenuBar";
const WATCHER: &str = "org.kde.StatusNotifierWatcher";
const WATCHER_PATH: &str = "/StatusNotifierWatcher";
/// How often the registration is checked.
const CHECK_EVERY: Duration = Duration::from_secs(3);

/// What a chosen entry or a click does, with the xdg-activation token the
/// panel gave just before, if any.
pub type OnAction = Arc<dyn Fn(TrayAction, Option<String>) + Send + Sync>;

// ---- The menu as dbusmenu numbers it ----

/// One node of the menu tree. Id 0 is the root.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub id: i32,
    pub props: Vec<(&'static str, Prop)>,
    pub children: Vec<Node>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Prop {
    Text(String),
    Flag(bool),
}

/// A label as dbusmenu reads it: `_` marks a mnemonic, so a real one is
/// doubled.
pub fn escape_label(label: &str) -> String {
    label.replace('_', "__")
}

/// Numbers the entries in order, depth first from 1, and returns the tree
/// with, for each id, the action it carries (`actions[id]`; the root and
/// anything not clickable have none).
pub fn build_tree(entries: &[Entry]) -> (Node, Vec<Option<TrayAction>>) {
    fn go(entry: &Entry, next: &mut i32, actions: &mut Vec<Option<TrayAction>>) -> Node {
        let id = *next;
        *next += 1;
        actions.push(None);
        match entry {
            Entry::Separator => Node {
                id,
                props: vec![("type", Prop::Text("separator".into()))],
                children: vec![],
            },
            Entry::Heading(label) => Node {
                id,
                props: vec![
                    ("label", Prop::Text(escape_label(label))),
                    ("enabled", Prop::Flag(false)),
                ],
                children: vec![],
            },
            Entry::Item { label, action } => {
                actions[id as usize] = Some(action.clone());
                Node {
                    id,
                    props: vec![("label", Prop::Text(escape_label(label)))],
                    children: vec![],
                }
            }
            Entry::Submenu { label, children } => {
                let children = children.iter().map(|c| go(c, next, actions)).collect();
                Node {
                    id,
                    props: vec![
                        ("label", Prop::Text(escape_label(label))),
                        ("children-display", Prop::Text("submenu".into())),
                    ],
                    children,
                }
            }
        }
    }
    let mut actions = vec![None];
    let mut next = 1;
    let children = entries
        .iter()
        .map(|e| go(e, &mut next, &mut actions))
        .collect();
    (
        Node {
            id: 0,
            props: vec![("children-display", Prop::Text("submenu".into()))],
            children,
        },
        actions,
    )
}

impl Node {
    fn find(&self, id: i32) -> Option<&Node> {
        if self.id == id {
            return Some(self);
        }
        self.children.iter().find_map(|c| c.find(id))
    }

    fn walk<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for c in &self.children {
            c.walk(out);
        }
    }
}

fn props_map(props: &[(&'static str, Prop)], wanted: &[String]) -> HashMap<String, OwnedValue> {
    props
        .iter()
        .filter(|(k, _)| wanted.is_empty() || wanted.iter().any(|w| w == k))
        .map(|(k, v)| {
            let value = match v {
                Prop::Text(s) => Value::from(s.as_str()),
                Prop::Flag(b) => Value::from(*b),
            };
            (
                k.to_string(),
                OwnedValue::try_from(value).expect("plain values"),
            )
        })
        .collect()
}

/// One icon as `IconPixmap` carries it: width, height, ARGB32 bytes.
type Pixmap = (i32, i32, Vec<u8>);
/// `ToolTip`: icon name, icon pixels, title, text.
type ToolTip = (String, Vec<Pixmap>, String, String);
/// A `GetLayout` reply's node, as a client reads it back (tests).
#[cfg(test)]
type RawNode = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

/// `(ia{sv}av)`.
#[derive(Debug, Type, serde::Serialize)]
pub struct Layout(i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

fn layout(node: &Node, depth: i32, wanted: &[String]) -> Layout {
    // Depth -1 is everything; 0 is this node alone.
    let children = if depth == 0 {
        vec![]
    } else {
        node.children
            .iter()
            .map(|c| {
                let l = layout(c, depth - 1, wanted);
                let s = Structure::from((
                    l.0,
                    l.1.into_iter()
                        .map(|(k, v)| (k, Value::from(v)))
                        .collect::<HashMap<String, Value>>(),
                    l.2.into_iter().map(Value::from).collect::<Vec<Value>>(),
                ));
                OwnedValue::try_from(Value::Structure(s)).expect("no file descriptors")
            })
            .collect()
    };
    Layout(node.id, props_map(&node.props, wanted), children)
}

// ---- The shared state the interfaces read ----

struct MenuState {
    revision: u32,
    root: Node,
    actions: Vec<Option<TrayAction>>,
}

struct ItemState {
    badge: Badge,
    tooltip: String,
    /// The latest token from the panel, used once.
    token: Option<String>,
}

struct Shared {
    menu: Mutex<MenuState>,
    item: Mutex<ItemState>,
    on_action: OnAction,
}

impl Shared {
    fn act(&self, action: TrayAction) {
        let token = self.item.lock().unwrap().token.take();
        (self.on_action)(action, token);
    }
}

struct Item(Arc<Shared>);

#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn category(&self) -> &str {
        "ApplicationStatus"
    }
    #[zbus(property)]
    fn id(&self) -> &str {
        "io.github.csnook.hab-bot"
    }
    #[zbus(property)]
    fn title(&self) -> &str {
        "Reminders"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "Active"
    }
    #[zbus(property)]
    fn window_id(&self) -> u32 {
        0
    }
    #[zbus(property)]
    fn icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        badge::pixmaps(self.0.item.lock().unwrap().badge)
    }
    #[zbus(property)]
    fn overlay_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn overlay_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }
    #[zbus(property)]
    fn attention_icon_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn attention_icon_pixmap(&self) -> Vec<(i32, i32, Vec<u8>)> {
        vec![]
    }
    #[zbus(property)]
    fn attention_movie_name(&self) -> &str {
        ""
    }
    #[zbus(property)]
    fn tool_tip(&self) -> ToolTip {
        (
            String::new(),
            vec![],
            "Reminders".into(),
            self.0.item.lock().unwrap().tooltip.clone(),
        )
    }
    /// False, so that the host calls `Activate` on a left click rather than
    /// opening the menu.
    #[zbus(property)]
    fn item_is_menu(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn menu(&self) -> zbus::zvariant::ObjectPath<'static> {
        zbus::zvariant::ObjectPath::from_static_str_unchecked(MENU_PATH)
    }

    fn activate(&self, _x: i32, _y: i32) {
        self.0.act(TrayAction::OpenWindow);
    }
    /// Middle click.
    fn secondary_activate(&self, _x: i32, _y: i32) {
        self.0.act(TrayAction::OpenWindow);
    }
    /// The host shows the menu itself; there is nothing to do.
    fn context_menu(&self, _x: i32, _y: i32) {}
    fn scroll(&self, _delta: i32, _orientation: &str) {}
    fn provide_xdg_activation_token(&self, token: String) {
        self.0.item.lock().unwrap().token = Some(token).filter(|t| !t.is_empty());
    }
}

struct DbusMenu(Arc<Shared>);

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl DbusMenu {
    #[zbus(property)]
    fn version(&self) -> u32 {
        3
    }
    #[zbus(property)]
    fn text_direction(&self) -> &str {
        "ltr"
    }
    #[zbus(property)]
    fn status(&self) -> &str {
        "normal"
    }
    #[zbus(property)]
    fn icon_theme_path(&self) -> Vec<String> {
        vec![]
    }

    fn get_layout(
        &self,
        parent_id: i32,
        recursion_depth: i32,
        property_names: Vec<String>,
    ) -> zbus::fdo::Result<(u32, Layout)> {
        let m = self.0.menu.lock().unwrap();
        let node = m
            .root
            .find(parent_id)
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no menu item {parent_id}")))?;
        Ok((m.revision, layout(node, recursion_depth, &property_names)))
    }

    fn get_group_properties(
        &self,
        ids: Vec<i32>,
        property_names: Vec<String>,
    ) -> Vec<(i32, HashMap<String, OwnedValue>)> {
        let m = self.0.menu.lock().unwrap();
        let mut all = vec![];
        m.root.walk(&mut all);
        all.into_iter()
            .filter(|n| ids.is_empty() || ids.contains(&n.id))
            .map(|n| (n.id, props_map(&n.props, &property_names)))
            .collect()
    }

    fn get_property(&self, id: i32, name: &str) -> zbus::fdo::Result<OwnedValue> {
        let m = self.0.menu.lock().unwrap();
        m.root
            .find(id)
            .and_then(|n| props_map(&n.props, &[name.to_string()]).remove(name))
            .ok_or_else(|| zbus::fdo::Error::InvalidArgs(format!("no property {name} on {id}")))
    }

    fn event(&self, id: i32, event_id: &str, _data: Value<'_>, _timestamp: u32) {
        if event_id != "clicked" {
            return;
        }
        let action = self
            .0
            .menu
            .lock()
            .unwrap()
            .actions
            .get(id as usize)
            .cloned()
            .flatten();
        if let Some(action) = action {
            self.0.act(action);
        }
    }

    fn event_group(&self, events: Vec<(i32, String, Value<'_>, u32)>) -> Vec<i32> {
        for (id, event_id, data, timestamp) in events {
            self.event(id, &event_id, data, timestamp);
        }
        vec![]
    }

    /// The menu is pushed when it changes, so it is always current.
    fn about_to_show(&self, _id: i32) -> bool {
        false
    }

    fn about_to_show_group(&self, _ids: Vec<i32>) -> (Vec<i32>, Vec<i32>) {
        (vec![], vec![])
    }
}

// ---- The tray ----

/// The running tray. Dropping it takes the icon down.
pub struct Tray {
    conn: zbus::blocking::Connection,
    shared: Arc<Shared>,
    last: Mutex<Option<(Badge, String, Vec<Entry>)>>,
    stop: Arc<AtomicBool>,
}

impl Tray {
    /// Puts the tray on the session bus (or `address`), serving its objects
    /// at once and registering with the watcher as soon as there is one.
    pub fn start(address: Option<&str>, on_action: OnAction) -> Result<Tray, String> {
        Self::start_checking(address, on_action, CHECK_EVERY)
    }

    pub fn start_checking(
        address: Option<&str>,
        on_action: OnAction,
        check_every: Duration,
    ) -> Result<Tray, String> {
        let badge = Badge {
            count: 0,
            tone: crate::tray_model::Tone::Blue,
        };
        let (root, actions) = build_tree(&[]);
        let shared = Arc::new(Shared {
            menu: Mutex::new(MenuState {
                revision: 1,
                root,
                actions,
            }),
            item: Mutex::new(ItemState {
                badge,
                tooltip: String::new(),
                token: None,
            }),
            on_action,
        });
        let service = format!("org.kde.StatusNotifierItem-{}-1", std::process::id());
        let builder = match address {
            Some(a) => zbus::blocking::connection::Builder::address(a),
            None => zbus::blocking::connection::Builder::session(),
        }
        .map_err(|e| e.to_string())?;
        let conn = builder
            .name(service.as_str())
            .map_err(|e| e.to_string())?
            .serve_at(ITEM_PATH, Item(shared.clone()))
            .map_err(|e| e.to_string())?
            .serve_at(MENU_PATH, DbusMenu(shared.clone()))
            .map_err(|e| e.to_string())?
            .build()
            .map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        {
            let (conn, stop) = (conn.clone(), stop.clone());
            std::thread::Builder::new()
                .name("tray-register".into())
                .spawn(move || keep_registered(&conn, &service, &stop, check_every))
                .map_err(|e| e.to_string())?;
        }
        Ok(Tray {
            conn,
            shared,
            last: Mutex::new(None),
            stop,
        })
    }

    /// Shows a new badge, tooltip and menu. Nothing is sent when they are
    /// what is already shown.
    pub fn update(&self, badge: Badge, tooltip: String, menu: Vec<Entry>) {
        let mut last = self.last.lock().unwrap();
        let (icon_changed, tip_changed, menu_changed) = match &*last {
            None => (true, true, true),
            Some((b, t, m)) => (*b != badge, *t != tooltip, *m != menu),
        };
        if !(icon_changed || tip_changed || menu_changed) {
            return;
        }
        {
            let mut item = self.shared.item.lock().unwrap();
            item.badge = badge;
            item.tooltip = tooltip.clone();
        }
        let revision = menu_changed.then(|| {
            let (root, actions) = build_tree(&menu);
            let mut m = self.shared.menu.lock().unwrap();
            m.revision += 1;
            m.root = root;
            m.actions = actions;
            m.revision
        });
        *last = Some((badge, tooltip, menu));
        drop(last);
        let signal = |path: &str, iface: &str, name: &str| {
            let _ = self.conn.emit_signal(None::<&str>, path, iface, name, &());
        };
        if icon_changed {
            signal(ITEM_PATH, "org.kde.StatusNotifierItem", "NewIcon");
        }
        if tip_changed {
            signal(ITEM_PATH, "org.kde.StatusNotifierItem", "NewToolTip");
        }
        if let Some(revision) = revision {
            let _ = self.conn.emit_signal(
                None::<&str>,
                MENU_PATH,
                "com.canonical.dbusmenu",
                "LayoutUpdated",
                &(revision, 0i32),
            );
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// Registers with the watcher, and again whenever the watcher's owner
/// changes, until `stop`.
fn keep_registered(
    conn: &zbus::blocking::Connection,
    service: &str,
    stop: &AtomicBool,
    every: Duration,
) {
    let mut registered_with: Option<String> = None;
    while !stop.load(Ordering::SeqCst) {
        let owner = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "GetNameOwner",
                &(WATCHER,),
            )
            .ok()
            .and_then(|m| m.body().deserialize::<String>().ok());
        if owner != registered_with {
            registered_with = match owner {
                Some(owner) => {
                    let result = conn.call_method(
                        Some(WATCHER),
                        WATCHER_PATH,
                        Some(WATCHER),
                        "RegisterStatusNotifierItem",
                        &(service,),
                    );
                    match result {
                        Ok(_) => Some(owner),
                        Err(e) => {
                            eprintln!("tray: the watcher refused the icon: {e}");
                            None
                        }
                    }
                }
                None => None,
            };
        }
        std::thread::sleep(every);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tray_model::Tone;
    use std::io::{BufRead, BufReader};
    use std::process::{Child, Stdio};
    use std::sync::mpsc;

    fn entries() -> Vec<Entry> {
        vec![
            Entry::Item {
                label: "Open Reminders".into(),
                action: TrayAction::OpenWindow,
            },
            Entry::Separator,
            Entry::Submenu {
                label: "Take_pills".into(),
                children: vec![
                    Entry::Item {
                        label: "Done".into(),
                        action: TrayAction::Done("o1".into()),
                    },
                    Entry::Item {
                        label: "Snooze".into(),
                        action: TrayAction::Snooze("o1".into()),
                    },
                ],
            },
            Entry::Heading("Waiting".into()),
            Entry::Item {
                label: "Quit".into(),
                action: TrayAction::Quit,
            },
        ]
    }

    #[test]
    fn ids_follow_the_order_and_only_items_carry_actions() {
        let (root, actions) = build_tree(&entries());
        assert_eq!(root.id, 0);
        let mut all = vec![];
        root.walk(&mut all);
        let ids: Vec<i32> = all.iter().map(|n| n.id).collect();
        assert_eq!(ids, (0..=7).collect::<Vec<_>>());
        assert_eq!(actions.len(), 8);
        assert_eq!(actions[1], Some(TrayAction::OpenWindow));
        assert_eq!(actions[2], None, "separator");
        assert_eq!(actions[3], None, "submenu row");
        assert_eq!(actions[4], Some(TrayAction::Done("o1".into())));
        assert_eq!(actions[5], Some(TrayAction::Snooze("o1".into())));
        assert_eq!(actions[6], None, "heading");
        assert_eq!(actions[7], Some(TrayAction::Quit));
    }

    #[test]
    fn underscores_in_titles_are_not_mnemonics() {
        assert_eq!(escape_label("a_b"), "a__b");
        let (root, _) = build_tree(&entries());
        let row = &root.children[2];
        assert!(row
            .props
            .contains(&("label", Prop::Text("Take__pills".into()))));
        assert!(row
            .props
            .contains(&("children-display", Prop::Text("submenu".into()))));
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

    /// A stand-in for the desktop's StatusNotifierWatcher.
    struct FakeWatcher(mpsc::Sender<String>);

    #[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
    impl FakeWatcher {
        fn register_status_notifier_item(&self, service: String) {
            let _ = self.0.send(service);
        }
    }

    fn watcher(address: &str) -> (zbus::blocking::Connection, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel();
        let conn = zbus::blocking::connection::Builder::address(address)
            .unwrap()
            .name(WATCHER)
            .unwrap()
            .serve_at(WATCHER_PATH, FakeWatcher(tx))
            .unwrap()
            .build()
            .unwrap();
        (conn, rx)
    }

    fn client(address: &str) -> zbus::blocking::Connection {
        zbus::blocking::connection::Builder::address(address)
            .unwrap()
            .build()
            .unwrap()
    }

    fn call<B: serde::Serialize + zbus::zvariant::DynamicType>(
        c: &zbus::blocking::Connection,
        service: &str,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<zbus::Message> {
        c.call_method(Some(service), path, Some(iface), method, body)
    }

    fn property<T: TryFrom<OwnedValue>>(
        c: &zbus::blocking::Connection,
        service: &str,
        path: &str,
        iface: &str,
        name: &str,
    ) -> T
    where
        T::Error: std::fmt::Debug,
    {
        let m = call(
            c,
            service,
            path,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(iface, name),
        )
        .unwrap();
        let v: OwnedValue = m.body().deserialize().unwrap();
        T::try_from(v).unwrap()
    }

    #[test]
    fn a_left_click_opens_the_window_and_the_menu_works_over_the_bus() {
        let Some((_bus, address)) = start_bus() else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let (_watcher, registered) = watcher(&address);
        let (tx, actions) = mpsc::channel();
        let tray = Tray::start_checking(
            Some(&address),
            Arc::new(move |a, token| {
                let _ = tx.send((a, token));
            }),
            Duration::from_millis(50),
        )
        .unwrap();

        // It registers its own bus name with the watcher.
        let service = registered.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            service.starts_with("org.kde.StatusNotifierItem-"),
            "{service}"
        );

        let host = client(&address);
        let item = "org.kde.StatusNotifierItem";
        // A left click needs ItemIsMenu false.
        assert!(!property::<bool>(
            &host,
            &service,
            ITEM_PATH,
            item,
            "ItemIsMenu"
        ));
        assert_eq!(
            property::<String>(&host, &service, ITEM_PATH, item, "Id"),
            "io.github.csnook.hab-bot"
        );

        // The badge and menu are what update() was given.
        let badge = Badge {
            count: 3,
            tone: Tone::Red,
        };
        tray.update(badge, "3 due".into(), entries());
        let m = call(
            &host,
            &service,
            ITEM_PATH,
            "org.freedesktop.DBus.Properties",
            "Get",
            &(item, "IconPixmap"),
        )
        .unwrap();
        let v: OwnedValue = m.body().deserialize().unwrap();
        let pixmaps: Vec<(i32, i32, Vec<u8>)> = v.try_into().unwrap();
        assert_eq!(pixmaps, badge::pixmaps(badge));
        let tip: ToolTip = property(&host, &service, ITEM_PATH, item, "ToolTip");
        assert_eq!((tip.2.as_str(), tip.3.as_str()), ("Reminders", "3 due"));

        // Left click: Plasma hands a token first, then calls Activate.
        call(
            &host,
            &service,
            ITEM_PATH,
            item,
            "ProvideXdgActivationToken",
            &("tok-1",),
        )
        .unwrap();
        call(
            &host,
            &service,
            ITEM_PATH,
            item,
            "Activate",
            &(10i32, 20i32),
        )
        .unwrap();
        assert_eq!(
            actions.recv_timeout(Duration::from_secs(5)).unwrap(),
            (TrayAction::OpenWindow, Some("tok-1".to_string()))
        );
        // The token is good for one use.
        call(&host, &service, ITEM_PATH, item, "Activate", &(0i32, 0i32)).unwrap();
        assert_eq!(
            actions.recv_timeout(Duration::from_secs(5)).unwrap(),
            (TrayAction::OpenWindow, None)
        );

        // The menu, as the host reads it.
        let menu = "com.canonical.dbusmenu";
        let m = call(
            &host,
            &service,
            MENU_PATH,
            menu,
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .unwrap();
        let (revision, root): (u32, RawNode) = m.body().deserialize().unwrap();
        assert!(revision >= 2);
        assert_eq!(root.0, 0);
        assert_eq!(root.2.len(), 5);
        let first: Structure = Structure::try_from(root.2[0].try_clone().unwrap()).unwrap();
        let fields = first.fields();
        assert_eq!(i32::try_from(&fields[0]).unwrap(), 1);
        let Value::Dict(props) = &fields[1] else {
            panic!("{fields:?}")
        };
        let label: Option<String> = props.get(&"label").unwrap();
        assert_eq!(label.as_deref(), Some("Open Reminders"));
        let row = Structure::try_from(root.2[2].try_clone().unwrap()).unwrap();
        let Value::Array(kids) = &row.fields()[2] else {
            panic!("{row:?}")
        };
        assert_eq!(kids.len(), 2, "Done and Snooze under the occurrence");

        // Choosing Done under the occurrence (id 4) acts on it.
        call(
            &host,
            &service,
            MENU_PATH,
            menu,
            "Event",
            &(4i32, "clicked", Value::from(0i32), 0u32),
        )
        .unwrap();
        assert_eq!(
            actions.recv_timeout(Duration::from_secs(5)).unwrap().0,
            TrayAction::Done("o1".into())
        );
        // Opening a submenu, or a separator, does nothing.
        call(
            &host,
            &service,
            MENU_PATH,
            menu,
            "Event",
            &(3i32, "opened", Value::from(0i32), 0u32),
        )
        .unwrap();
        call(
            &host,
            &service,
            MENU_PATH,
            menu,
            "Event",
            &(2i32, "clicked", Value::from(0i32), 0u32),
        )
        .unwrap();
        assert!(actions.recv_timeout(Duration::from_millis(300)).is_err());

        // Updating with the same thing sends nothing new; a change bumps the
        // revision.
        let before = revision;
        tray.update(badge, "3 due".into(), entries());
        let m = call(
            &host,
            &service,
            MENU_PATH,
            menu,
            "GetLayout",
            &(0i32, 0i32, Vec::<String>::new()),
        )
        .unwrap();
        let (same, _): (u32, RawNode) = m.body().deserialize().unwrap();
        assert_eq!(same, before);
        tray.update(badge, "3 due".into(), vec![]);
        let m = call(
            &host,
            &service,
            MENU_PATH,
            menu,
            "GetLayout",
            &(0i32, -1i32, Vec::<String>::new()),
        )
        .unwrap();
        let (after, root): (u32, RawNode) = m.body().deserialize().unwrap();
        assert!(after > before);
        assert!(root.2.is_empty());
    }

    #[test]
    fn it_registers_when_the_watcher_arrives_late_and_again_when_it_restarts() {
        let Some((_bus, address)) = start_bus() else {
            eprintln!("skipped: no dbus-daemon");
            return;
        };
        let (tx, _actions) = mpsc::channel();
        let _tray = Tray::start_checking(
            Some(&address),
            Arc::new(move |a, t| {
                let _ = tx.send((a, t));
            }),
            Duration::from_millis(50),
        )
        .unwrap();
        // Login: the desktop's watcher isn't up yet.
        std::thread::sleep(Duration::from_millis(300));
        let (first, registered) = watcher(&address);
        let service = registered.recv_timeout(Duration::from_secs(5)).unwrap();
        // The shell restarts: a new watcher, a new registration.
        drop(first);
        let (_second, registered) = watcher(&address);
        assert_eq!(
            registered.recv_timeout(Duration::from_secs(5)).unwrap(),
            service
        );
    }
}
