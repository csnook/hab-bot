//! Syncing the personal list through the server (ADR 0005).
//!
//! Every event is encrypted with the list's key and signed by this device
//! before it leaves, and the server numbers it without reading it. Live events
//! travel on one WebSocket per device while the server can be reached, and
//! bulk uploads and downloads go over HTTPS. Changes made offline stay "not
//! sent yet" in the core and are uploaded when the connection returns.

use crate::keystore::{KeyId, KeyKind, KeyStore, KeyStoreError};
use crate::profile::Profile;
use crate::tls::{self, parse_address, Pinned, TlsError};
use futures_util::{SinkExt, StreamExt};
use hab_core::Core;
use hab_proto::wire::{
    AppendBatch, AppendResults, ClientMessage, DeviceList, DeviceRecord, Envelope, EventPage,
    FetchEvents, ListRef, ListRefs, Numbered, NumberedEnvelope, RegisterList, ServerMessage,
};
use hab_proto::{seal_list_key, verify_device, DeviceKeys, KeyError, Keys, ListKey};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;
use tokio::sync::{watch, Notify};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;

/// Events per upload over HTTPS.
const UPLOAD_CHUNK: usize = 50;
const PAGE: u32 = 100;
const PING_EVERY: Duration = Duration::from_secs(25);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Server(#[from] TlsError),
    #[error(transparent)]
    Keys(#[from] KeyStoreError),
    #[error("this device's keys are missing or damaged")]
    MissingKeys,
    #[error(transparent)]
    Key(#[from] KeyError),
    #[error(transparent)]
    Core(#[from] hab_core::Error),
    #[error("the server refused an event: {0}")]
    Rejected(String),
    #[error("the connection failed: {0}")]
    Connection(String),
}

/// How syncing is going, for Settings.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncStatus {
    /// A WebSocket to the server is open.
    pub connected: bool,
    /// Why the last attempt failed, until one works.
    pub last_error: Option<String>,
}

pub struct Syncer {
    server: Pinned,
    device_id: i64,
    device: DeviceKeys,
    keys: Keys,
    core: Arc<Mutex<Core>>,
    /// Wakes the loop when the core has a new change to send.
    wake: Arc<Notify>,
    on_change: Box<dyn Fn() + Send + Sync>,
    /// The account's devices that its identity key vouches for.
    directory: Mutex<HashMap<i64, DeviceRecord>>,
    status: Mutex<SyncStatus>,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_rustls::client::TlsStream<tokio::net::TcpStream>>;

impl Syncer {
    /// Load this device's keys and prepare to sync `core` with the server in
    /// `profile`. `wake` is notified by the app after each change it makes.
    /// `on_change` runs after the server's events change what the core shows.
    pub async fn new(
        profile: &Profile,
        store: &KeyStore,
        core: Arc<Mutex<Core>>,
        wake: Arc<Notify>,
        on_change: impl Fn() + Send + Sync + 'static,
    ) -> Result<Syncer, SyncError> {
        let id = |kind| KeyId {
            server_fingerprint: profile.server_fingerprint.clone(),
            username: profile.username.clone(),
            kind,
        };
        let account = store
            .get(&id(KeyKind::Account))
            .await?
            .ok_or(SyncError::MissingKeys)?;
        let device = store
            .get(&id(KeyKind::Device))
            .await?
            .ok_or(SyncError::MissingKeys)?;
        Ok(Syncer {
            server: Pinned::new(&profile.server_address, &profile.server_fingerprint),
            device_id: profile.device_id,
            device: DeviceKeys::from_bytes(&device)?,
            keys: Keys::from_bytes(&account)?,
            core,
            wake,
            on_change: Box::new(on_change),
            directory: Mutex::new(HashMap::new()),
            status: Mutex::new(SyncStatus::default()),
        })
    }

    pub fn status(&self) -> SyncStatus {
        self.status.lock().unwrap().clone()
    }

    fn set_status(&self, connected: bool, error: Option<String>) {
        *self.status.lock().unwrap() = SyncStatus {
            connected,
            last_error: error,
        };
    }

    fn core(&self) -> MutexGuard<'_, Core> {
        self.core.lock().unwrap()
    }

    /// The key of a list. The personal list's is made from the personal key, so
    /// every device of the account can have it.
    fn list_key(&self, list_id: &str) -> ListKey {
        ListKey::personal(&self.keys.personal, list_id)
    }

    async fn post<B: serde::Serialize, R: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> Result<R, SyncError> {
        Ok(self
            .server
            .signed_post(path, body, self.device_id, &self.device)
            .await?)
    }

    /// The account's devices, keeping only those the identity key signed.
    async fn refresh_directory(&self) -> Result<HashMap<i64, DeviceRecord>, SyncError> {
        let list: DeviceList = self.post("/api/v1/devices", &()).await?;
        let identity = self.keys.identity_public();
        let verified: HashMap<i64, DeviceRecord> = list
            .devices
            .into_iter()
            .filter(|d| verify_device(&identity, &d.record).is_ok())
            .map(|d| (d.id, d.record))
            .collect();
        *self.directory.lock().unwrap() = verified.clone();
        Ok(verified)
    }

    /// The ids of the account's lists on the server. A device added to an
    /// account takes the first as its personal list
    /// ([`Core::use_personal_list`]) before it downloads.
    pub async fn account_lists(&self) -> Result<Vec<String>, SyncError> {
        let lists: ListRefs = self.post("/api/v1/lists", &()).await?;
        Ok(lists.lists.into_iter().map(|l| l.list_id).collect())
    }

    /// Make the personal list the account's on the server and store its key
    /// sealed to each of the account's devices.
    pub async fn register_personal_list(&self) -> Result<(), SyncError> {
        let list_id = self.core().personal_list_id().to_string();
        let key = self.list_key(&list_id);
        let devices = self.refresh_directory().await?;
        let mut sealed = Vec::new();
        for (id, record) in &devices {
            sealed.push(seal_list_key(
                &key,
                &list_id,
                1,
                &self.device,
                self.device_id,
                *id,
                &record.sealing_public,
            )?);
        }
        let _: ListRef = self
            .post(
                "/api/v1/lists/register",
                &RegisterList {
                    list_id,
                    keys: sealed,
                },
            )
            .await?;
        Ok(())
    }

    fn seal_event(&self, o: &hab_core::Outgoing) -> Envelope {
        Envelope::seal(
            &self.list_key(&o.list_id),
            &self.device,
            self.device_id,
            &o.list_id,
            &o.event_id,
            o.format,
            o.recorded_at,
            &o.payload,
        )
    }

    fn numbered(&self, n: &Numbered) -> Result<(), SyncError> {
        self.core().mark_sent(&n.event_id, n.seq)?;
        (self.on_change)();
        Ok(())
    }

    /// Upload everything the server hasn't numbered, in order, over HTTPS.
    /// Joining does this with the standalone history. Returns how many events
    /// the server numbered.
    pub async fn upload_unsent(&self) -> Result<usize, SyncError> {
        let mut sent = 0;
        loop {
            let chunk: Vec<_> = self
                .core()
                .unsent()?
                .into_iter()
                .take(UPLOAD_CHUNK)
                .collect();
            if chunk.is_empty() {
                return Ok(sent);
            }
            let envelopes = chunk.iter().map(|o| self.seal_event(o)).collect();
            let results: AppendResults = self
                .post("/api/v1/sync/append", &AppendBatch { envelopes })
                .await?;
            for n in &results.numbered {
                self.numbered(n)?;
                sent += 1;
            }
            if let Some(r) = results.rejected.first() {
                return Err(SyncError::Rejected(r.error.clone()));
            }
        }
    }

    /// Register the personal list and upload the standalone history. Run once
    /// when joining; the sync loop repeats it harmlessly on every connection.
    pub async fn upload_standalone(&self) -> Result<usize, SyncError> {
        self.register_personal_list().await?;
        self.upload_unsent().await
    }

    /// Download what the server has numbered since this device last looked,
    /// over HTTPS. Returns how many events were new to this device.
    pub async fn download(&self) -> Result<usize, SyncError> {
        let list_id = self.core().personal_list_id().to_string();
        let mut new = 0;
        loop {
            let after = self.core().cursor()?;
            let page: EventPage = self
                .post(
                    "/api/v1/sync/events",
                    &FetchEvents {
                        list_id: list_id.clone(),
                        after,
                        limit: PAGE,
                    },
                )
                .await?;
            self.ensure_known(&page.events).await?;
            for n in &page.events {
                if self.ingest(n) {
                    new += 1;
                }
            }
            if let Some(last) = page.events.last() {
                self.core().set_cursor(last.seq)?;
            }
            if !page.events.is_empty() {
                (self.on_change)();
            }
            if !page.more || page.events.is_empty() {
                return Ok(new);
            }
        }
    }

    /// Refresh the device directory if an event names a device it lacks.
    async fn ensure_known(&self, events: &[NumberedEnvelope]) -> Result<(), SyncError> {
        let unknown = {
            let dir = self.directory.lock().unwrap();
            events
                .iter()
                .any(|n| !dir.contains_key(&n.envelope.device_id))
        };
        if unknown {
            self.refresh_directory().await?;
        }
        Ok(())
    }

    /// Verify, decrypt and hand one numbered event to the core. Anything that
    /// doesn't check out is dropped. Returns whether it was new to this device.
    fn ingest(&self, n: &NumberedEnvelope) -> bool {
        let e = &n.envelope;
        let signer = self
            .directory
            .lock()
            .unwrap()
            .get(&e.device_id)
            .map(|r| r.signing_public.clone());
        let Some(signer) = signer else {
            tracing_skip("an event from a device the identity key didn't sign");
            return false;
        };
        if e.verify(&signer).is_err() {
            tracing_skip("an event with a bad signature");
            return false;
        }
        let Ok(payload) = e.open(&self.list_key(&e.list_id)) else {
            tracing_skip("an event that could not be opened");
            return false;
        };
        match self.core().receive(
            &e.list_id,
            n.seq,
            &e.event_id,
            &e.device_id.to_string(),
            e.format,
            &payload,
        ) {
            Ok(new) => new,
            Err(_) => {
                tracing_skip("an event that could not be read");
                false
            }
        }
    }

    /// Open the WebSocket, signed as this device.
    async fn connect_ws(&self) -> Result<Socket, SyncError> {
        const PATH: &str = "/api/v1/sync/ws";
        let (host, port) = parse_address(&self.server.address)?;
        let host = if host.contains(':') {
            format!("[{host}]")
        } else {
            host
        };
        let mut request = format!("wss://{host}:{port}{PATH}")
            .into_client_request()
            .map_err(|e| SyncError::Connection(e.to_string()))?;
        for (name, value) in tls::signed_headers(&self.device, self.device_id, "GET", PATH, b"") {
            request.headers_mut().insert(
                name,
                value
                    .parse()
                    .map_err(|_| SyncError::Connection("bad header".into()))?,
            );
        }
        let (tls, _) = tls::connect(&self.server.address, Some(&self.server.fingerprint)).await?;
        let (socket, _) = tokio_tungstenite::client_async(request, tls)
            .await
            .map_err(|e| SyncError::Connection(e.to_string()))?;
        Ok(socket)
    }

    /// Send every unsent event not already in flight.
    async fn flush(
        &self,
        socket: &mut Socket,
        in_flight: &mut HashSet<String>,
    ) -> Result<(), SyncError> {
        let unsent = self.core().unsent()?;
        for o in unsent {
            if !in_flight.insert(o.event_id.clone()) {
                continue;
            }
            let msg = ClientMessage::Append {
                envelope: self.seal_event(&o),
            };
            let text = serde_json::to_string(&msg).expect("messages serialize");
            socket
                .send(Message::text(text))
                .await
                .map_err(|e| SyncError::Connection(e.to_string()))?;
        }
        Ok(())
    }

    /// One connection: connect, catch up, send what is waiting, then carry
    /// live events both ways until the connection ends or `stop` is set.
    async fn session(&self, stop: &mut watch::Receiver<bool>) -> Result<(), SyncError> {
        self.register_personal_list().await?;
        let mut socket = self.connect_ws().await?;
        // Connected before downloading, so an event can't fall between the two.
        self.download().await?;
        self.set_status(true, None);
        let mut in_flight = HashSet::new();
        self.flush(&mut socket, &mut in_flight).await?;
        let mut ping = tokio::time::interval(PING_EVERY);
        ping.tick().await;
        loop {
            tokio::select! {
                _ = stop.changed() => {
                    let _ = socket.close(None).await;
                    return Ok(());
                }
                _ = self.wake.notified() => self.flush(&mut socket, &mut in_flight).await?,
                _ = ping.tick() => socket
                    .send(Message::Ping(Vec::new().into()))
                    .await
                    .map_err(|e| SyncError::Connection(e.to_string()))?,
                message = socket.next() => {
                    let message = match message {
                        Some(Ok(m)) => m,
                        Some(Err(e)) => return Err(SyncError::Connection(e.to_string())),
                        None => return Err(SyncError::Connection("closed".into())),
                    };
                    let Message::Text(text) = message else {
                        if matches!(message, Message::Close(_)) {
                            return Err(SyncError::Connection("closed".into()));
                        }
                        continue;
                    };
                    let Ok(msg) = serde_json::from_str::<ServerMessage>(&text) else { continue };
                    self.handle(msg, &mut in_flight).await?;
                }
            }
        }
    }

    async fn handle(
        &self,
        msg: ServerMessage,
        in_flight: &mut HashSet<String>,
    ) -> Result<(), SyncError> {
        match msg {
            ServerMessage::Numbered(n) => {
                in_flight.remove(&n.event_id);
                self.numbered(&n)?;
            }
            ServerMessage::Rejected(r) => {
                // Left unsent and not resent on this connection.
                self.set_status(
                    true,
                    Some(format!("the server refused an event: {}", r.error)),
                );
            }
            ServerMessage::Resync => {
                self.download().await?;
            }
            ServerMessage::Event {
                seq,
                received_at,
                envelope,
            } => {
                let cursor = self.core().cursor()?;
                if seq <= cursor {
                    return Ok(());
                }
                if seq == cursor + 1 {
                    let n = NumberedEnvelope {
                        seq,
                        received_at,
                        envelope,
                    };
                    self.ensure_known(std::slice::from_ref(&n)).await?;
                    self.ingest(&n);
                    self.core().set_cursor(seq)?;
                    (self.on_change)();
                } else {
                    // Missed some: the bulk download fills the gap in order.
                    self.download().await?;
                }
            }
        }
        Ok(())
    }

    /// Keep this device in sync until `stop` is set: stay connected while the
    /// server can be reached, and try again with a growing wait when it can't.
    pub async fn run(&self, mut stop: watch::Receiver<bool>) {
        let mut wait = Duration::from_secs(1);
        loop {
            if *stop.borrow() {
                return;
            }
            let started = tokio::time::Instant::now();
            let result = self.session(&mut stop).await;
            if *stop.borrow() {
                return;
            }
            if let Err(e) = result {
                self.set_status(false, Some(e.to_string()));
            }
            if started.elapsed() > Duration::from_secs(30) {
                wait = Duration::from_secs(1);
            }
            tokio::select! {
                _ = tokio::time::sleep(wait) => {}
                _ = stop.changed() => return,
            }
            wait = (wait * 2).min(MAX_BACKOFF);
        }
    }
}

/// An event that fails a check is dropped, and the others carry on.
fn tracing_skip(what: &str) {
    eprintln!("sync: dropped {what}");
}
