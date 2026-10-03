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
use hab_core::{Core, Payload};
use hab_proto::wire::SealedKeys;
use hab_proto::wire::{
    AppendBatch, AppendResults, ClientMessage, DeviceList, DeviceRecord, Envelope, EventPage,
    FetchEvents, ListRef, ListRefs, Numbered, NumberedEnvelope, RegisterList, RemoveDevice,
    Rotation, ServerMessage,
};
use hab_proto::{open_list_key, seal_list_key, verify_device, DeviceKeys, KeyError, Keys, ListKey};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
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
    #[error(
        "the account has no personal list on the server yet; open the app on another device first"
    )]
    NoPersonalList,
    #[error("no list key sealed to this device by one of the account's devices")]
    NoSealedKey,
    #[error("waiting for another of your devices to share the list's new key with this one")]
    WaitingForKey,
    #[error("a device can only be removed from another device")]
    OwnDevice,
    #[error("there is no such device on this account")]
    UnknownDevice,
    #[error("this account has a list whose key this device cannot rotate")]
    CannotRotate,
}

/// One of the account's devices, as Settings → Account lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeviceInfo {
    /// The server's id for the device.
    pub id: i64,
    /// What the device calls itself, from the user's encrypted settings. Not
    /// known until its first event has arrived here.
    pub name: Option<String>,
    pub portable: bool,
    /// When the server last heard from the device, in Unix seconds.
    pub last_synced: Option<i64>,
    /// This is the device asking.
    pub this_device: bool,
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
    /// Devices that were removed. What they signed still verifies; nothing is
    /// ever sealed to them.
    retired: Mutex<HashMap<i64, DeviceRecord>>,
    /// The personal list's keys after the first rotation, by version. Version
    /// 1 is made from the personal key; later ones are random and arrive
    /// sealed to this device by another of the account's devices.
    rotated: Mutex<BTreeMap<u32, ListKey>>,
    status: Mutex<SyncStatus>,
    /// A device signed in since the personal list's key was last sealed to
    /// the account's devices.
    reseal: AtomicBool,
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
            retired: Mutex::new(HashMap::new()),
            rotated: Mutex::new(BTreeMap::new()),
            status: Mutex::new(SyncStatus::default()),
            reseal: AtomicBool::new(false),
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

    /// The personal list's keys, newest first. Version 1 is made from the
    /// personal key, so every device of the account has it; a removed device
    /// has it too, which is why removing one adds a random version that is
    /// only ever sealed to the devices that remain (ADR 0008).
    fn keyring(&self, list_id: &str) -> Vec<(u32, ListKey)> {
        let mut ring: Vec<(u32, ListKey)> = self
            .rotated
            .lock()
            .unwrap()
            .iter()
            .rev()
            .map(|(v, k)| (*v, k.clone()))
            .collect();
        ring.push((1, ListKey::personal(&self.keys.personal, list_id)));
        ring
    }

    /// The key new events are encrypted with: the newest.
    fn list_key(&self, list_id: &str) -> ListKey {
        self.keyring(list_id).swap_remove(0).1
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

    /// The account's devices, keeping only those the identity key signed,
    /// with when the server last heard from each.
    async fn fetch_devices(&self) -> Result<Vec<hab_proto::wire::DeviceEntry>, SyncError> {
        let list: DeviceList = self.post("/api/v1/devices", &()).await?;
        let identity = self.keys.identity_public();
        let vouched =
            |d: &hab_proto::wire::DeviceEntry| verify_device(&identity, &d.record).is_ok();
        let devices: Vec<_> = list.devices.into_iter().filter(vouched).collect();
        *self.directory.lock().unwrap() =
            devices.iter().map(|d| (d.id, d.record.clone())).collect();
        *self.retired.lock().unwrap() = list
            .retired
            .into_iter()
            .filter(vouched)
            .map(|d| (d.id, d.record))
            .collect();
        Ok(devices)
    }

    async fn refresh_directory(&self) -> Result<HashMap<i64, DeviceRecord>, SyncError> {
        self.fetch_devices().await?;
        Ok(self.directory.lock().unwrap().clone())
    }

    /// The account's devices for Settings → Account, oldest first.
    pub async fn devices(&self) -> Result<Vec<DeviceInfo>, SyncError> {
        let devices = self.fetch_devices().await?;
        let core = self.core();
        Ok(devices
            .into_iter()
            .map(|d| DeviceInfo {
                id: d.id,
                name: core.device_name(&d.id.to_string()).map(str::to_string),
                portable: d.record.portable,
                last_synced: d.last_synced,
                this_device: d.id == self.device_id,
            })
            .collect())
    }

    /// Fetch the copies of the personal list's key sealed to this device and
    /// keep every version sealed by a device the identity key vouches for.
    /// Returns whether a version new to this device arrived.
    async fn load_keys(&self) -> Result<bool, SyncError> {
        let list_id = self.core().personal_list_id().to_string();
        let directory = self.refresh_directory().await?;
        let sealed: SealedKeys = match self
            .post(
                "/api/v1/lists/keys",
                &ListRef {
                    list_id: list_id.clone(),
                },
            )
            .await
        {
            Ok(sealed) => sealed,
            // The list isn't on the server yet: there is nothing to load.
            Err(SyncError::Server(TlsError::Status { status: 404, .. })) => return Ok(false),
            Err(e) => return Err(e),
        };
        let mut changed = false;
        for k in sealed.keys.iter().filter(|k| k.key_version > 1) {
            let Some(sender) = directory.get(&k.sealed_by) else {
                continue;
            };
            if self.rotated.lock().unwrap().contains_key(&k.key_version) {
                continue;
            }
            if let Ok(key) = open_list_key(k, &list_id, &self.device, &sender.sealing_public) {
                self.rotated.lock().unwrap().insert(k.key_version, key);
                changed = true;
            }
        }
        Ok(changed)
    }

    /// Remove another of the account's devices and rotate the personal list's
    /// key, in one step on the server. The new key is sealed, together with
    /// every older one, to each device that remains, so they can still read
    /// the whole history. The removed device can't fetch or send anything
    /// after this, and can't open anything made with the new key.
    pub async fn remove_device(&self, device_id: i64) -> Result<(), SyncError> {
        if device_id == self.device_id {
            return Err(SyncError::OwnDevice);
        }
        let mut attempt = 0;
        let (new_version, new_key) = loop {
            attempt += 1;
            // Start from what the server has now: another device may have
            // rotated, or signed in, since this device last looked.
            self.load_keys().await?;
            let directory = self.directory.lock().unwrap().clone();
            if !directory.contains_key(&device_id) {
                return Err(SyncError::UnknownDevice);
            }
            let list_id = self.core().personal_list_id().to_string();
            if self.account_lists().await? != [list_id.clone()] {
                return Err(SyncError::CannotRotate);
            }
            let mut ring = self.keyring(&list_id);
            let new_version = ring[0].0 + 1;
            let new_key = ListKey::random();
            ring.insert(0, (new_version, new_key.clone()));
            let mut keys = Vec::new();
            for (id, record) in directory.iter().filter(|(id, _)| **id != device_id) {
                for (version, key) in &ring {
                    keys.push(seal_list_key(
                        key,
                        &list_id,
                        *version,
                        &self.device,
                        self.device_id,
                        *id,
                        &record.sealing_public,
                    )?);
                }
            }
            let sent: Result<serde_json::Value, SyncError> = self
                .post(
                    "/api/v1/devices/remove",
                    &RemoveDevice {
                        device_id,
                        lists: vec![Rotation { list_id, keys }],
                    },
                )
                .await;
            match sent {
                Ok(_) => break (new_version, new_key),
                Err(SyncError::Server(TlsError::Status { status: 409, .. })) if attempt < 3 => {
                    continue
                }
                Err(SyncError::Server(TlsError::Status { status: 404, .. })) => {
                    return Err(SyncError::UnknownDevice)
                }
                Err(e) => return Err(e),
            }
        };
        self.rotated.lock().unwrap().insert(new_version, new_key);
        self.refresh_directory().await?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        self.core()
            .record_device_removed(&device_id.to_string(), now)?;
        self.wake.notify_one();
        (self.on_change)();
        Ok(())
    }

    /// The ids of the account's lists on the server. A device added to an
    /// account takes the first as its personal list
    /// ([`Core::use_personal_list`]) before it downloads.
    pub async fn account_lists(&self) -> Result<Vec<String>, SyncError> {
        let lists: ListRefs = self.post("/api/v1/lists", &()).await?;
        Ok(lists.lists.into_iter().map(|l| l.list_id).collect())
    }

    /// A device that has just signed in to an existing account takes the
    /// account's personal list in place of the one it made for itself
    /// ([`Core::use_personal_list`]), before it downloads anything.
    pub async fn adopt_account_list(&self) -> Result<(), SyncError> {
        let lists = self.account_lists().await?;
        let first = lists.first().ok_or(SyncError::NoPersonalList)?;
        self.core().use_personal_list(first)?;
        Ok(())
    }

    /// The personal list's key as sealed to this device by one of the
    /// account's other (or its own) devices, opened here. The device that
    /// sealed it has to be one the identity key vouches for.
    pub async fn sealed_list_key(&self) -> Result<ListKey, SyncError> {
        let list_id = self.core().personal_list_id().to_string();
        let directory = self.refresh_directory().await?;
        let sealed: SealedKeys = self
            .post(
                "/api/v1/lists/keys",
                &ListRef {
                    list_id: list_id.clone(),
                },
            )
            .await?;
        let newest = sealed
            .keys
            .iter()
            .filter(|k| directory.contains_key(&k.sealed_by))
            .max_by_key(|k| k.key_version)
            .ok_or(SyncError::NoSealedKey)?;
        let sender = &directory[&newest.sealed_by];
        Ok(open_list_key(
            newest,
            &list_id,
            &self.device,
            &sender.sealing_public,
        )?)
    }

    /// Make the personal list the account's on the server and store its key
    /// sealed to each of the account's devices.
    pub async fn register_personal_list(&self) -> Result<(), SyncError> {
        let list_id = self.core().personal_list_id().to_string();
        self.reseal.store(false, Ordering::SeqCst);
        // Also refreshes the directory, and finds any key a rotation added.
        self.load_keys().await?;
        let devices = self.directory.lock().unwrap().clone();
        let ring = self.keyring(&list_id);
        let mut sealed = Vec::new();
        for (id, record) in &devices {
            for (version, key) in &ring {
                sealed.push(seal_list_key(
                    key,
                    &list_id,
                    *version,
                    &self.device,
                    self.device_id,
                    *id,
                    &record.sealing_public,
                )?);
            }
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
            let mut reloaded = false;
            for n in &page.events {
                if self.ingest_reloading(n, &mut reloaded).await? {
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
            let retired = self.retired.lock().unwrap();
            events.iter().any(|n| {
                let id = n.envelope.device_id;
                !dir.contains_key(&id) && !retired.contains_key(&id)
            })
        };
        if unknown {
            self.refresh_directory().await?;
        }
        Ok(())
    }

    /// [`Self::ingest`], and if no key this device has opens the event, once
    /// per batch fetch the keys again first: the list's key may have been
    /// rotated since this device looked.
    async fn ingest_reloading(
        &self,
        n: &NumberedEnvelope,
        reloaded: &mut bool,
    ) -> Result<bool, SyncError> {
        let mut result = self.ingest(n);
        if result == Ingest::NoKey && !*reloaded {
            *reloaded = true;
            if self.load_keys().await? {
                result = self.ingest(n);
            }
        }
        match result {
            Ingest::New => Ok(true),
            Ingest::Known | Ingest::Dropped => Ok(false),
            // The event stays unread and the cursor stays put: it is read once
            // another device has sealed the key to this one.
            Ingest::NoKey => Err(SyncError::WaitingForKey),
        }
    }

    /// Verify, decrypt and hand one numbered event to the core. Anything that
    /// doesn't check out is dropped.
    fn ingest(&self, n: &NumberedEnvelope) -> Ingest {
        let e = &n.envelope;
        // A removed device's earlier events still count; they were signed
        // while it was one of the account's.
        let signer = {
            let directory = self.directory.lock().unwrap();
            let retired = self.retired.lock().unwrap();
            directory
                .get(&e.device_id)
                .or_else(|| retired.get(&e.device_id))
                .map(|r| r.signing_public.clone())
        };
        let Some(signer) = signer else {
            tracing_skip("an event from a device the identity key didn't sign");
            return Ingest::Dropped;
        };
        if e.verify(&signer).is_err() {
            tracing_skip("an event with a bad signature");
            return Ingest::Dropped;
        }
        // Whichever version of the list's key it was made with, newest first.
        let Some(payload) = self
            .keyring(&e.list_id)
            .iter()
            .find_map(|(_, key)| e.open(key).ok())
        else {
            tracing_skip("an event that could not be opened");
            return Ingest::NoKey;
        };
        let signed_in = e.format <= hab_core::FORMAT_VERSION
            && e.device_id != self.device_id
            && serde_json::from_slice::<Payload>(&payload)
                .map(|p| p.event.get("type").and_then(|t| t.as_str()) == Some("device_signed_in"))
                .unwrap_or(false);
        match self.core().receive(
            &e.list_id,
            n.seq,
            &e.event_id,
            &e.device_id.to_string(),
            e.format,
            &payload,
        ) {
            Ok(new) => {
                if new && signed_in {
                    // The new device needs the list key sealed to it.
                    self.reseal.store(true, Ordering::SeqCst);
                }
                if new {
                    Ingest::New
                } else {
                    Ingest::Known
                }
            }
            Err(_) => {
                tracing_skip("an event that could not be read");
                Ingest::Dropped
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
        match self.download().await {
            Ok(_) => {}
            Err(SyncError::WaitingForKey) => {
                // A device that signed in after a key was rotated has to tell
                // the others it is there before they seal the key to it.
                self.upload_unsent().await?;
                return Err(SyncError::WaitingForKey);
            }
            Err(e) => return Err(e),
        }
        self.reseal_if_needed().await?;
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
                    self.reseal_if_needed().await?;
                }
            }
        }
    }

    /// After another device signs in, seal the list key to it as well.
    async fn reseal_if_needed(&self) -> Result<(), SyncError> {
        if self.reseal.load(Ordering::SeqCst) {
            self.register_personal_list().await?;
        }
        Ok(())
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
            ServerMessage::KeysChanged => {
                // A device was removed: the directory and the key changed.
                self.load_keys().await?;
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
                    self.ingest_reloading(&n, &mut false).await?;
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

/// What became of one numbered event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ingest {
    /// Applied; the device hadn't seen it.
    New,
    /// The device already had it.
    Known,
    /// Failed a check, and is left out.
    Dropped,
    /// No key this device has opens it.
    NoKey,
}

/// An event that fails a check is dropped, and the others carry on.
fn tracing_skip(what: &str) {
    eprintln!("sync: dropped {what}");
}
