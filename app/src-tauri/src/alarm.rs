//! The alarm window, and raising it.
//!
//! An alarm is a critical notification, a looping sound and this window (spec:
//! Alerts → The alarm screens). [`crate::notify::Delivery`] starts and ends all
//! three; this module is the window half:
//! - [`AlarmWindows`] is the seam: open (or raise) the window for an
//!   occurrence, close it. [`TauriWindows`] is the real one, a small
//!   always-on-top window per ringing occurrence showing the UI's alarm view
//!   (`index.html?alarm=<occurrence id>`); the tests use a fake.
//! - [`Activation`] gets an xdg-activation token for raising it on Wayland.
//!
//! Wayland doesn't let an app take focus for itself. A compositor raises a
//! window only if the request carries an xdg-activation token that it handed
//! out for a recent user action (or its own policy allows it). None of the
//! libraries we use passes one on: GTK asks for tokens only for the apps it
//! launches. So, in order of preference, the window is raised with:
//! 1. the token the notification server sent with a click on the alarm's
//!    notification (the `ActivationToken` signal), which a compositor
//!    treats as user input;
//! 2. a token we ask the compositor for ourselves ([`crate::wayland`]), which
//!    Plasma honours only if its focus-stealing prevention allows it, and
//!    otherwise shows as the window demanding attention in the task manager.
//!
//! The token is given to GTK as the window's startup id before presenting it,
//! which GTK's Wayland backend turns into `xdg_activation_v1.activate`. On X11
//! there is no token and presenting the window is enough.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// What the app asks of the alarm window.
pub trait AlarmWindows: Send + Sync {
    /// Shows the occurrence's alarm window, or raises it if it is already up,
    /// using the xdg-activation `token` where there is one.
    fn open(&self, occurrence_id: &str, token: Option<&str>);
    /// Closes it, if it is up.
    fn close(&self, occurrence_id: &str);
}

/// Where an xdg-activation token for raising our own window comes from.
pub trait Activation: Send + Sync {
    fn token(&self) -> Option<String>;
}

/// Asks the Wayland compositor for a token; none on X11 or if it refuses.
pub struct WaylandActivation {
    pub app_id: &'static str,
}

impl Activation for WaylandActivation {
    fn token(&self) -> Option<String> {
        std::env::var_os("WAYLAND_DISPLAY")?;
        match crate::wayland::request_token(self.app_id) {
            Ok(t) => Some(t),
            Err(e) => {
                eprintln!("no xdg-activation token: {e}");
                None
            }
        }
    }
}

/// Which window belongs to which occurrence. Window labels can't hold an
/// occurrence id (`@`), so each gets a numbered one.
#[derive(Default)]
pub struct Registry {
    inner: Mutex<RegistryInner>,
}

#[derive(Default)]
struct RegistryInner {
    next: u32,
    by_occurrence: HashMap<String, String>,
}

pub const LABEL_PREFIX: &str = "alarm-";

impl Registry {
    pub fn label_for(&self, occurrence_id: &str) -> String {
        let mut r = self.inner.lock().unwrap();
        if let Some(l) = r.by_occurrence.get(occurrence_id) {
            return l.clone();
        }
        r.next += 1;
        let label = format!("{LABEL_PREFIX}{}", r.next);
        r.by_occurrence
            .insert(occurrence_id.to_string(), label.clone());
        label
    }

    pub fn label_of(&self, occurrence_id: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .by_occurrence
            .get(occurrence_id)
            .cloned()
    }

    pub fn occurrence_of(&self, label: &str) -> Option<String> {
        self.inner
            .lock()
            .unwrap()
            .by_occurrence
            .iter()
            .find(|(_, l)| l.as_str() == label)
            .map(|(o, _)| o.clone())
    }

    pub fn forget(&self, occurrence_id: &str) {
        self.inner
            .lock()
            .unwrap()
            .by_occurrence
            .remove(occurrence_id);
    }
}

/// Percent-encodes an occurrence id for a URL query.
pub fn encode_query(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The page an alarm window loads.
pub fn page_for(occurrence_id: &str) -> String {
    format!("index.html?alarm={}", encode_query(occurrence_id))
}

/// The real alarm windows.
pub struct TauriWindows {
    app: AppHandle,
    registry: std::sync::Arc<Registry>,
}

impl TauriWindows {
    pub fn new(app: AppHandle, registry: std::sync::Arc<Registry>) -> Self {
        TauriWindows { app, registry }
    }
}

impl AlarmWindows for TauriWindows {
    fn open(&self, occurrence_id: &str, token: Option<&str>) {
        let label = self.registry.label_for(occurrence_id);
        let window = match self.app.get_webview_window(&label) {
            Some(w) => w,
            None => {
                let built = WebviewWindowBuilder::new(
                    &self.app,
                    &label,
                    WebviewUrl::App(page_for(occurrence_id).into()),
                )
                .title("Ringing")
                .inner_size(420.0, 380.0)
                .always_on_top(true)
                .center()
                .build();
                match built {
                    Ok(w) => w,
                    Err(e) => {
                        eprintln!("cannot open the alarm window: {e}");
                        self.registry.forget(occurrence_id);
                        return;
                    }
                }
            }
        };
        raise(&window, token);
    }

    fn close(&self, occurrence_id: &str) {
        if let Some(label) = self.registry.label_of(occurrence_id) {
            if let Some(w) = self.app.get_webview_window(&label) {
                let _ = w.destroy();
            }
        }
        self.registry.forget(occurrence_id);
    }
}

/// Brings the window to the front, giving GTK the activation token first.
fn raise(window: &WebviewWindow, token: Option<&str>) {
    use gtk::prelude::GtkWindowExt;
    let _ = window.show();
    let _ = window.unminimize();
    let token = token.map(str::to_string);
    let target = window.clone();
    let _ = window.run_on_main_thread(move || {
        if let Ok(gtk_window) = target.gtk_window() {
            if let Some(t) = &token {
                gtk_window.set_startup_id(t);
            }
            gtk_window.present();
        }
    });
    let _ = window.set_focus();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_occurrence_gets_one_stable_label() {
        let r = Registry::default();
        let a = r.label_for("rem@1790000000");
        let b = r.label_for("other@1790000000");
        assert_ne!(a, b);
        assert_eq!(r.label_for("rem@1790000000"), a);
        assert!(a.starts_with(LABEL_PREFIX));
        assert_eq!(r.occurrence_of(&a).as_deref(), Some("rem@1790000000"));
        r.forget("rem@1790000000");
        assert_eq!(r.label_of("rem@1790000000"), None);
        assert_eq!(r.occurrence_of(&a), None);
    }

    #[test]
    fn the_window_page_carries_the_occurrence_id() {
        assert_eq!(page_for("0f/1@17 9"), "index.html?alarm=0f%2F1%4017%209");
    }
}
