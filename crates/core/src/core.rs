use std::path::Path;

use serde::Serialize;
use uuid::Uuid;

use crate::event::{Event, Outgoing, Payload, StoredEvent, FORMAT_VERSION, UPDATE_NOTICE};
use crate::state::{DueItem, State, UpcomingItem};
use crate::store::Store;
use crate::{Error, Result};

/// An occurrence that has just fired, for the platform to alert about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Fired {
    pub occurrence_id: String,
    pub reminder_id: String,
    pub title: String,
}

/// What the window shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    /// The Inbox's Due section.
    pub due: Vec<DueItem>,
    /// Reminders that haven't fired yet.
    pub upcoming: Vec<UpcomingItem>,
    /// Set while the list holds changes from a newer app, which this one keeps
    /// without applying.
    pub update_notice: Option<String>,
}

/// The core for one device: its storage, its user, and the personal list's state.
pub struct Core {
    store: Store,
    state: State,
    list_id: String,
    device_id: String,
    user_id: String,
    /// Events from a newer app are being kept unapplied.
    holding_newer: bool,
}

impl Core {
    pub fn open(path: &Path) -> Result<Self> {
        Self::from_store(Store::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::from_store(Store::open_in_memory()?)
    }

    /// Builds the current state by applying the personal list's stream.
    pub fn from_store(store: Store) -> Result<Self> {
        let new_id = || Uuid::new_v4().to_string();
        let list_id = store.meta_or_init("personal_list_id", new_id)?;
        let device_id = store.meta_or_init("device_id", new_id)?;
        let user_id = store.meta_or_init("user_id", new_id)?;
        let mut core = Core {
            store,
            state: State::default(),
            list_id,
            device_id,
            user_id,
            holding_newer: false,
        };
        core.rebuild()?;
        Ok(core)
    }

    /// Builds the state again from the stream: the server's numbered events in
    /// order, then this device's unsent ones on top.
    fn rebuild(&mut self) -> Result<()> {
        let mut state = State::default();
        for e in self.store.stream(&self.list_id)? {
            state.apply(&e);
        }
        self.state = state;
        self.holding_newer = !self.store.held(&self.list_id)?.is_empty();
        Ok(())
    }

    pub fn personal_list_id(&self) -> &str {
        &self.list_id
    }

    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Whether this device has joined a server.
    pub fn is_joined(&self) -> Result<bool> {
        self.store.joined()
    }

    /// This device has joined a server as `user_id`, as device `device_id`
    /// there. Everything it made while standalone is now that user's and
    /// that device's, still unsent: the sync layer uploads it, history
    /// included, and the server numbers it.
    pub fn join(&mut self, user_id: &str, device_id: &str) -> Result<()> {
        self.store.adopt(user_id, device_id)?;
        self.user_id = user_id.to_string();
        self.device_id = device_id.to_string();
        // The standalone numbers were this device's own; the server's replace them.
        self.store.forget_local_numbers(&self.list_id)?;
        self.rebuild()
    }

    /// Use the account's personal list, whose id the server gave this device,
    /// in place of the one it made for itself. Only for a device whose own
    /// list is still empty, such as one just added to an account.
    pub fn use_personal_list(&mut self, list_id: &str) -> Result<()> {
        if list_id == self.list_id {
            return Ok(());
        }
        if !self.store.stream(&self.list_id)?.is_empty() {
            return Err(Error::BadEvent("this device already has its own list"));
        }
        self.store.set_meta("personal_list_id", list_id)?;
        self.list_id = list_id.to_string();
        self.rebuild()
    }

    /// The events the server hasn't numbered, in the order they were made.
    pub fn unsent(&self) -> Result<Vec<Outgoing>> {
        self.store
            .unsent(&self.list_id)?
            .into_iter()
            .map(|row| {
                let payload = Payload {
                    author: row.author,
                    recorded_at: row.recorded_at,
                    event: serde_json::from_slice(&row.body)?,
                };
                Ok(Outgoing {
                    list_id: self.list_id.clone(),
                    event_id: row.event_id,
                    format: row.format,
                    recorded_at: payload.recorded_at,
                    payload: serde_json::to_vec(&payload)?,
                })
            })
            .collect()
    }

    /// The server numbered one of this device's events.
    pub fn mark_sent(&mut self, event_id: &str, seq: i64) -> Result<()> {
        self.store.set_seq(event_id, seq)?;
        self.rebuild()
    }

    /// An event the server numbered, decrypted and verified by the sync layer.
    /// One in a newer format is kept without being applied. Returns whether it
    /// was new to this device.
    pub fn receive(
        &mut self,
        list_id: &str,
        seq: i64,
        event_id: &str,
        device_id: &str,
        format: u32,
        payload: &[u8],
    ) -> Result<bool> {
        let p: Payload =
            serde_json::from_slice(payload).map_err(|_| Error::BadEvent("not a payload"))?;
        if format <= FORMAT_VERSION && serde_json::from_value::<Event>(p.event.clone()).is_err() {
            return Err(Error::BadEvent("not an event"));
        }
        let inserted = self.store.insert_numbered(
            list_id,
            seq,
            event_id,
            device_id,
            &p.author,
            p.recorded_at,
            format,
            &serde_json::to_vec(&p.event)?,
        )?;
        self.rebuild()?;
        Ok(inserted)
    }

    /// Ids of events kept without being applied, because a newer app made them.
    pub fn held_events(&self) -> Result<Vec<String>> {
        Ok(self
            .store
            .held(&self.list_id)?
            .into_iter()
            .map(|h| h.event_id)
            .collect())
    }

    /// The highest server number this device has downloaded up to.
    pub fn cursor(&self) -> Result<i64> {
        Ok(self
            .store
            .meta(&format!("cursor:{}", self.list_id))?
            .and_then(|v| v.parse().ok())
            .unwrap_or(0))
    }

    pub fn set_cursor(&self, seq: i64) -> Result<()> {
        self.store
            .set_meta(&format!("cursor:{}", self.list_id), &seq.to_string())
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn user_id(&self) -> &str {
        &self.user_id
    }

    fn record(&mut self, now: i64, event: Event) -> Result<StoredEvent> {
        let stored = self.store.append(
            &self.list_id,
            &self.device_id,
            &self.user_id,
            now,
            event,
            Uuid::new_v4().to_string(),
        )?;
        self.state.apply(&stored);
        Ok(stored)
    }

    /// Creates a one-off reminder in the personal list, to fire at `fire_at`.
    pub fn create_reminder(&mut self, title: &str, fire_at: i64, now: i64) -> Result<String> {
        let title = title.trim();
        if title.is_empty() {
            return Err(Error::EmptyTitle);
        }
        let reminder_id = Uuid::new_v4().to_string();
        self.record(
            now,
            Event::ReminderCreated {
                reminder_id: reminder_id.clone(),
                title: title.to_string(),
                fire_at,
            },
        )?;
        Ok(reminder_id)
    }

    /// Fires every reminder whose time has come, opening an occurrence for
    /// each. A reminder whose time passed while the app was closed fires late,
    /// on the first tick after start.
    pub fn tick(&mut self, now: i64) -> Result<Vec<Fired>> {
        let pending: Vec<(String, String, i64)> = self
            .state
            .pending_firings(now)
            .into_iter()
            .map(|r| (r.id.clone(), r.title.clone(), r.fire_at))
            .collect();
        let mut fired = Vec::new();
        for (reminder_id, title, scheduled_at) in pending {
            // The occurrence's identity is the reminder plus the scheduled
            // time, so firings on several devices merge into one.
            let occurrence_id = format!("{reminder_id}@{scheduled_at}");
            self.record(
                now,
                Event::OccurrenceOpened {
                    occurrence_id: occurrence_id.clone(),
                    reminder_id: reminder_id.clone(),
                    scheduled_at,
                    fired_at: now,
                },
            )?;
            fired.push(Fired {
                occurrence_id,
                reminder_id,
                title,
            });
        }
        Ok(fired)
    }

    /// Completes an open occurrence, recording who and when. The one-off
    /// reminder is then finished.
    pub fn complete(&mut self, occurrence_id: &str, now: i64) -> Result<()> {
        let open = self
            .state
            .occurrences
            .get(occurrence_id)
            .is_some_and(|o| o.completed.is_none());
        if !open {
            return Err(Error::NotOpen(occurrence_id.to_string()));
        }
        self.record(
            now,
            Event::OccurrenceCompleted {
                occurrence_id: occurrence_id.to_string(),
                completed_at: now,
            },
        )?;
        Ok(())
    }

    /// When the next unfired reminder is due, so the scheduler can sleep.
    pub fn next_fire_at(&self) -> Option<i64> {
        self.state.next_fire_at()
    }

    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            due: self.state.due(),
            upcoming: self.state.upcoming(),
            update_notice: self.holding_newer.then(|| UPDATE_NOTICE.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_000_000;

    fn core() -> Core {
        Core::open_in_memory().unwrap()
    }

    #[test]
    fn reminder_does_not_fire_before_its_time() {
        let mut c = core();
        c.create_reminder("Call the plumber", T0 + 60, T0).unwrap();
        assert!(c.tick(T0 + 59).unwrap().is_empty());
        assert!(c.snapshot().due.is_empty());
        assert_eq!(c.snapshot().upcoming.len(), 1);
        assert_eq!(c.next_fire_at(), Some(T0 + 60));
    }

    #[test]
    fn reminder_fires_at_its_time_and_opens_an_occurrence() {
        let mut c = core();
        c.create_reminder("Call the plumber", T0 + 60, T0).unwrap();
        let fired = c.tick(T0 + 60).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].title, "Call the plumber");
        let snap = c.snapshot();
        assert_eq!(snap.due.len(), 1);
        assert_eq!(snap.due[0].title, "Call the plumber");
        assert_eq!(snap.due[0].scheduled_at, T0 + 60);
        assert!(snap.upcoming.is_empty());
        assert_eq!(c.next_fire_at(), None);
    }

    #[test]
    fn firing_happens_once() {
        let mut c = core();
        c.create_reminder("Once", T0, T0).unwrap();
        assert_eq!(c.tick(T0).unwrap().len(), 1);
        assert!(c.tick(T0 + 1).unwrap().is_empty());
        assert_eq!(c.snapshot().due.len(), 1);
    }

    #[test]
    fn completing_records_who_and_when_and_finishes_the_one_off() {
        let mut c = core();
        let rid = c.create_reminder("Bins", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0 + 30).unwrap();

        assert!(c.snapshot().due.is_empty());
        let o = &c.state().occurrences[&fired[0].occurrence_id];
        assert_eq!(o.completed, Some((c.user_id().to_string(), T0 + 30)));
        assert!(c.state().is_finished(&rid));
        // Finished: it never fires again.
        assert!(c.tick(T0 + 1000).unwrap().is_empty());
        assert_eq!(c.next_fire_at(), None);
    }

    #[test]
    fn completing_twice_or_unknown_is_an_error() {
        let mut c = core();
        c.create_reminder("Bins", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0).unwrap();
        assert!(matches!(
            c.complete(&fired[0].occurrence_id, T0 + 1),
            Err(Error::NotOpen(_))
        ));
        assert!(matches!(c.complete("nope", T0), Err(Error::NotOpen(_))));
    }

    #[test]
    fn empty_title_is_rejected() {
        let mut c = core();
        assert!(matches!(
            c.create_reminder("   ", T0, T0),
            Err(Error::EmptyTitle)
        ));
    }

    #[test]
    fn same_time_reminders_each_fire() {
        let mut c = core();
        c.create_reminder("A", T0, T0).unwrap();
        c.create_reminder("B", T0, T0).unwrap();
        assert_eq!(c.tick(T0).unwrap().len(), 2);
        assert_eq!(c.snapshot().due.len(), 2);
    }

    #[test]
    fn restart_rebuilds_the_same_state_and_fires_what_was_missed() {
        let dir = std::env::temp_dir().join(format!("hab-core-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("hab.db");

        let (open_id, done_id);
        {
            let mut c = Core::open(&path).unwrap();
            c.create_reminder("Open one", T0 + 10, T0).unwrap();
            c.create_reminder("Done one", T0 + 10, T0).unwrap();
            c.create_reminder("Later", T0 + 5000, T0).unwrap();
            c.create_reminder("While closed", T0 + 100, T0).unwrap();
            let fired = c.tick(T0 + 10).unwrap();
            assert_eq!(fired.len(), 2);
            let done = fired.iter().find(|f| f.title == "Done one").unwrap();
            let open = fired.iter().find(|f| f.title == "Open one").unwrap();
            c.complete(&done.occurrence_id, T0 + 20).unwrap();
            done_id = done.occurrence_id.clone();
            open_id = open.occurrence_id.clone();
        }

        // Restart after "While closed" was due: the same state, rebuilt from
        // the stream, and the missed reminder fires on the first tick.
        let mut c = Core::open(&path).unwrap();
        assert_eq!(c.snapshot().due.len(), 1);
        assert_eq!(c.snapshot().due[0].occurrence_id, open_id);
        assert!(c.state().occurrences[&done_id].completed.is_some());
        assert_eq!(c.snapshot().upcoming.len(), 2);

        let fired = c.tick(T0 + 200).unwrap();
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].title, "While closed");
        assert_eq!(c.snapshot().due.len(), 2);
        assert_eq!(c.snapshot().upcoming[0].title, "Later");

        // Restarting again changes nothing and fires nothing.
        drop(c);
        let mut c = Core::open(&path).unwrap();
        assert!(c.tick(T0 + 200).unwrap().is_empty());
        assert_eq!(c.snapshot().due.len(), 2);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn every_change_is_an_event_numbered_in_the_stream() {
        let mut c = core();
        c.create_reminder("X", T0, T0).unwrap();
        let fired = c.tick(T0).unwrap();
        c.complete(&fired[0].occurrence_id, T0 + 1).unwrap();
        let events = c.store.stream(&c.list_id).unwrap();
        let seqs: Vec<Option<i64>> = events.iter().map(|e| e.seq).collect();
        assert_eq!(seqs, vec![Some(1), Some(2), Some(3)]);
        assert!(matches!(events[0].event, Event::ReminderCreated { .. }));
        assert!(matches!(events[1].event, Event::OccurrenceOpened { .. }));
        assert!(matches!(events[2].event, Event::OccurrenceCompleted { .. }));
        assert!(events.iter().all(|e| e.author == c.user_id));
    }

    #[test]
    fn duplicate_firings_from_another_device_merge() {
        // The occurrence id is the reminder plus the scheduled time, so a
        // second device's identical firing changes nothing.
        let mut c = core();
        let rid = c.create_reminder("Shared", T0, T0).unwrap();
        c.tick(T0).unwrap();
        let dup = StoredEvent {
            list_id: c.list_id.clone(),
            seq: Some(99),
            event_id: "other".into(),
            device_id: "other-device".into(),
            author: c.user_id.clone(),
            recorded_at: T0 + 5,
            event: Event::OccurrenceOpened {
                occurrence_id: format!("{rid}@{T0}"),
                reminder_id: rid,
                scheduled_at: T0,
                fired_at: T0 + 5,
            },
        };
        c.state.apply(&dup);
        assert_eq!(c.state().occurrences.len(), 1);
        assert_eq!(c.snapshot().due[0].fired_at, T0);
    }

    #[test]
    fn after_joining_new_changes_wait_for_the_server_to_number_them() {
        let mut c = core();
        let rid = c.create_reminder("Before joining", T0 + 50, T0).unwrap();
        assert!(!c.snapshot().upcoming[0].not_sent);
        c.join("u1", "7").unwrap();
        // The standalone history is now unsent, and is uploaded in order.
        assert!(c.snapshot().upcoming[0].not_sent);
        c.tick(T0 + 50).unwrap();
        let out = c.unsent().unwrap();
        assert_eq!(out.len(), 2);
        let first: Payload = serde_json::from_slice(&out[0].payload).unwrap();
        assert_eq!((first.author.as_str(), first.recorded_at), ("u1", T0));
        assert_eq!(first.event["reminder_id"], rid);
        // The server's numbers replace the local ones, wherever they fall.
        c.mark_sent(&out[0].event_id, 5).unwrap();
        assert!(c.snapshot().due[0].not_sent);
        c.mark_sent(&out[1].event_id, 6).unwrap();
        assert!(!c.snapshot().due[0].not_sent);
        assert!(c.unsent().unwrap().is_empty());
    }

    #[test]
    fn events_from_a_newer_format_are_kept_and_applied_after_an_update() {
        let mut c = core();
        c.join("u1", "7").unwrap();
        let payload = |event: serde_json::Value| {
            serde_json::to_vec(&Payload {
                author: "u1".into(),
                recorded_at: T0,
                event,
            })
            .unwrap()
        };
        let list = c.personal_list_id().to_string();
        let future = payload(serde_json::json!({"type": "hologram", "n": 1}));
        assert!(c
            .receive(&list, 1, "f1", "8", FORMAT_VERSION + 1, &future)
            .unwrap());
        assert_eq!(c.snapshot().update_notice.as_deref(), Some(UPDATE_NOTICE));
        assert!(c.snapshot().upcoming.is_empty());
        // An event in a known format that doesn't read is refused, not kept.
        let junk = payload(serde_json::json!({"type": "hologram"}));
        assert!(c
            .receive(&list, 2, "bad", "8", FORMAT_VERSION, &junk)
            .is_err());
        // Receiving the same event again changes nothing.
        assert!(!c
            .receive(&list, 1, "f1", "8", FORMAT_VERSION + 1, &future)
            .unwrap());
        assert_eq!(c.held_events().unwrap(), vec!["f1".to_string()]);
    }

    #[test]
    fn a_new_device_takes_the_accounts_list_only_while_its_own_is_empty() {
        let mut c = core();
        c.use_personal_list("account-list").unwrap();
        assert_eq!(c.personal_list_id(), "account-list");
        c.create_reminder("X", T0, T0).unwrap();
        assert!(c.use_personal_list("other").is_err());
        assert_eq!(c.personal_list_id(), "account-list");
    }
}
