//! Checking with the server before an alert (spec: Alerts → Alerting on
//! several devices).
//!
//! [`Checks`] is the seam between the platform-independent alerter
//! (`hab_core::ServerCheck`) and the sync loop. The alerter asks for a check
//! and later asks whether it came back; the sync loop does the check, a catch-up
//! with the server over HTTPS, and says it is done. The app makes one
//! `Checks`, hands it to the [`Syncer`](crate::Syncer) and to the scheduler,
//! and gives it a waker so the scheduler looks again as soon as a check comes
//! back or the link changes.

use hab_core::ServerCheck;
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Mutex;
use tokio::sync::Notify;

/// What the device knows about the server right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    /// No server, or it can't be reached: alerts don't wait.
    Unreachable,
    /// Connecting for the first time since the app started. A late firing at
    /// start-up should still check first, so this counts as reachable. It
    /// ends within the connection timeout.
    Connecting,
    /// A live connection to the server.
    Connected,
}

impl Link {
    fn code(self) -> u8 {
        match self {
            Link::Unreachable => 0,
            Link::Connecting => 1,
            Link::Connected => 2,
        }
    }
    fn of(code: u8) -> Link {
        match code {
            1 => Link::Connecting,
            2 => Link::Connected,
            _ => Link::Unreachable,
        }
    }
}

/// The requests for checks and which have come back.
pub struct Checks {
    link: AtomicU8,
    /// The newest ticket handed out.
    requested: AtomicU64,
    /// Every ticket up to this one has been satisfied by a check that began
    /// after it was requested.
    completed: AtomicU64,
    /// Wakes the sync loop when there is a request.
    asked: Notify,
    waker: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
}

impl Default for Checks {
    fn default() -> Self {
        Self::new()
    }
}

impl Checks {
    pub fn new() -> Self {
        Checks {
            link: AtomicU8::new(Link::Unreachable.code()),
            requested: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            asked: Notify::new(),
            waker: Mutex::new(None),
        }
    }

    /// For a device that has a server to sync with and is about to connect:
    /// alerts that come before the first connection attempt has finished
    /// still check first ([`Link::Connecting`]).
    pub fn connecting() -> Self {
        let c = Self::new();
        c.link.store(Link::Connecting.code(), Ordering::SeqCst);
        c
    }

    /// `wake` runs whenever a check comes back or the link changes, so the
    /// scheduler can release alerts that were waiting.
    pub fn on_progress(&self, wake: impl Fn() + Send + Sync + 'static) {
        *self.waker.lock().unwrap() = Some(Box::new(wake));
    }

    fn wake(&self) {
        if let Some(w) = self.waker.lock().unwrap().as_ref() {
            w();
        }
    }

    pub fn link(&self) -> Link {
        Link::of(self.link.load(Ordering::SeqCst))
    }

    /// The sync loop's report on the connection. Checks that were waiting on
    /// a link that went away are released by the wake.
    pub fn set_link(&self, link: Link) {
        if self.link.swap(link.code(), Ordering::SeqCst) != link.code() {
            self.wake();
        }
    }

    /// Resolves when a check has been asked for.
    pub(crate) async fn asked(&self) {
        self.asked.notified().await;
    }

    /// The newest ticket asked for so far. The sync loop reads this before it
    /// starts a check, then passes it to [`complete`](Self::complete): a
    /// ticket handed out after that read isn't covered by this check.
    pub(crate) fn newest(&self) -> u64 {
        self.requested.load(Ordering::SeqCst)
    }

    /// A check that began when `newest` was the latest ticket has finished.
    pub(crate) fn complete(&self, newest: u64) {
        self.completed.fetch_max(newest, Ordering::SeqCst);
        self.wake();
    }
}

impl ServerCheck for Checks {
    fn reachable(&self) -> bool {
        self.link() != Link::Unreachable
    }

    fn request(&self) -> u64 {
        let ticket = self.requested.fetch_add(1, Ordering::SeqCst) + 1;
        self.asked.notify_one();
        ticket
    }

    fn done(&self, ticket: u64) -> bool {
        self.completed.load(Ordering::SeqCst) >= ticket
    }
}
