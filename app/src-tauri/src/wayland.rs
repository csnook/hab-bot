//! Asking a Wayland compositor for an xdg-activation token (protocol
//! `xdg_activation_v1`), which no library we use does for an app's own windows.
//!
//! The token request is made on a connection of our own, with the app id of
//! our desktop file. It carries no surface or input serial, because the alarm
//! rings without anyone having touched the app: so the compositor decides by
//! its own policy whether a window activated with it may take focus (KWin
//! does with its focus-stealing prevention set low; by default it marks the
//! window as wanting attention instead). A token from a click on the alarm's
//! notification is better and is preferred (see [`crate::alarm`]).
//!
//! This has not been run against a real compositor: there is none in the
//! environment it was written in.

use std::time::{Duration, Instant};

use wayland_client::globals::{registry_queue_init, GlobalListContents};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

#[derive(Default)]
struct State {
    token: Option<String>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &XdgActivationV1,
        _: <XdgActivationV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<XdgActivationTokenV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &XdgActivationTokenV1,
        event: xdg_activation_token_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let xdg_activation_token_v1::Event::Done { token } = event {
            state.token = Some(token);
        }
    }
}

/// How long to wait for the compositor's `done`.
const WAIT: Duration = Duration::from_secs(2);

/// Requests a token for `app_id`.
pub fn request_token(app_id: &str) -> Result<String, String> {
    let conn = Connection::connect_to_env().map_err(|e| format!("no Wayland connection: {e}"))?;
    let (globals, mut queue) =
        registry_queue_init::<State>(&conn).map_err(|e| format!("Wayland registry: {e}"))?;
    let qh = queue.handle();
    let activation: XdgActivationV1 = globals
        .bind(&qh, 1..=1, ())
        .map_err(|_| "the compositor has no xdg_activation_v1".to_string())?;
    let request = activation.get_activation_token(&qh, ());
    request.set_app_id(app_id.to_string());
    request.commit();
    let mut state = State::default();
    let deadline = Instant::now() + WAIT;
    while state.token.is_none() && Instant::now() < deadline {
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("Wayland: {e}"))?;
        if state.token.is_none() {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    request.destroy();
    activation.destroy();
    state
        .token
        .ok_or_else(|| "the compositor gave no token".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_a_compositor_there_is_no_token_and_no_hang() {
        // The test environment has no WAYLAND_DISPLAY.
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            return;
        }
        let started = Instant::now();
        assert!(request_token("io.github.csnook.hab-bot").is_err());
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
