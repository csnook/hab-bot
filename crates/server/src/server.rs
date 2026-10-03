//! The HTTPS listener and the routes behind it.

use crate::backup::{self, BackupError};
use crate::cert::{self, CertError};
use crate::config::{Config, DEFAULT_NAME};
use crate::db::{Db, DbError, LoginAccount, NewAccount, NewDevice, SyncError};
use crate::peers::Peers;
use crate::setup::SetupCode;
use crate::trusted::{self, TrustedError};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{routing::get, routing::post, Json, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use hab_proto::auth::{
    verify_request, HEADER_DEVICE, HEADER_SIGNATURE, HEADER_TIME, MAX_REQUEST_AGE_SECS,
};
use hab_proto::opaque_ke::{
    self, CredentialFinalization, CredentialRequest, RegistrationRequest, RegistrationUpload,
    ServerLogin, ServerLoginParameters, ServerRegistration, ServerSetup,
};
use hab_proto::wire::{
    valid_display_name, valid_id, valid_username, Algs, AppendBatch, AppendResults, ClientMessage,
    DeviceList, Envelope, ErrorBody, EventPage, FetchEvents, JoinFinish, JoinStart, JoinStarted,
    Joined, Kdf, KeyBundle, ListRef, ListRefs, LoginDevice, LoginFinish, LoginFinished, LoginStart,
    LoginStarted, Numbered, RegisterList, Rejected, SealedKeys, ServerMessage,
    MAX_CLOCK_AHEAD_SECS, MAX_EVENT_BYTES,
};
use hab_proto::{verify_device, Suite, ARGON_LANES, ARGON_MEMORY_KIB, ARGON_PASSES};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use serde::Serialize;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;
use tokio_rustls::TlsAcceptor;

const NAME_KEY: &str = "server_name";
const OPAQUE_KEY: &str = "opaque_setup";

/// The server's long-term OPAQUE keys, made once and kept in the database.
fn load_or_create_opaque(db: &Db) -> Result<ServerSetup<Suite>, DbError> {
    if let Some(text) = db.get(OPAQUE_KEY)? {
        if let Some(setup) = STANDARD
            .decode(text)
            .ok()
            .and_then(|b| ServerSetup::<Suite>::deserialize(&b).ok())
        {
            return Ok(setup);
        }
        tracing::warn!("the stored OPAQUE keys are unreadable; making new ones");
    }
    let setup = ServerSetup::<Suite>::new(&mut opaque_ke::rand::rngs::OsRng);
    db.set(OPAQUE_KEY, &STANDARD.encode(setup.serialize()))?;
    Ok(setup)
}

#[derive(Debug, thiserror::Error)]
pub enum StartError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error(transparent)]
    Cert(#[from] CertError),
    #[error("cannot listen on {0}: {1}")]
    Bind(SocketAddr, std::io::Error),
    #[error(transparent)]
    Trusted(#[from] TrustedError),
    #[error(transparent)]
    Backup(Box<BackupError>),
    #[error("tls setup failed: {0}")]
    Tls(#[from] rustls::Error),
}

/// What `GET /api/v1/info` reports, for apps to check before joining.
#[derive(Serialize)]
struct Info {
    name: String,
    version: &'static str,
}

/// A numbered event, offered to the account's other devices that are connected.
struct Pushed {
    account_id: i64,
    origin: i64,
    seq: i64,
    received_at: i64,
    envelope: Envelope,
}

/// The most events one download returns.
const MAX_PAGE: u32 = 100;
/// The most events one upload may hold.
const MAX_BATCH: usize = 100;
const MAX_BODY: usize = 8 * 1024 * 1024;

/// How long a sign-in may take between its steps.
const LOGIN_TTL: Duration = Duration::from_secs(300);
/// Sign-ins in progress at once, so strangers can't fill the memory.
const MAX_LOGINS: usize = 1000;

/// Where a sign-in is. Kept in memory only: a restart ends it.
enum Login {
    /// The client has been answered; waiting for its proof of the password.
    /// Holds the account if there is one: for an unknown username the
    /// exchange runs on a made-up record and can't succeed.
    Started(Box<ServerLogin<Suite>>, Option<Box<LoginAccount>>),
    /// The password was proven and the bundle released; one device may be added.
    Verified {
        account_id: i64,
        admin: bool,
        identity_public: Vec<u8>,
    },
}

struct State {
    logins: Mutex<HashMap<String, (Instant, Login)>>,
    push: broadcast::Sender<Arc<Pushed>>,
    db: Mutex<Db>,
    opaque: ServerSetup<Suite>,
    setup: SetupCode,
    peers: Arc<Peers>,
}

pub struct Server {
    local_addr: SocketAddr,
    fingerprint: String,
    cert_created: bool,
    name: String,
    state: Arc<State>,
    resolver: Arc<trusted::Resolver>,
    backup: Option<backup::BackupConfig>,
    db_path: std::path::PathBuf,
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
    background: Vec<JoinHandle<()>>,
}

impl Server {
    /// Open the database, load or make the certificate and setup code, bind,
    /// and start serving in the background.
    pub async fn start(config: &Config) -> Result<Server, StartError> {
        let db = Db::open(&config.data_dir)?;
        let identity = cert::load_or_create(&db, &config.cert_names)?;
        if let Some(name) = &config.name {
            db.set(NAME_KEY, name)?;
        }
        let name = db.get(NAME_KEY)?.unwrap_or_else(|| DEFAULT_NAME.into());
        let opaque = load_or_create_opaque(&db)?;

        let provider = rustls::crypto::ring::default_provider();
        let self_signed = rustls::sign::CertifiedKey::from_der(
            vec![identity.cert.clone()],
            identity.key.clone_key(),
            &provider,
        )?;
        let resolver = Arc::new(trusted::Resolver::new(Arc::new(self_signed)));
        let mut trusted_files = match (&config.tls_cert, &config.tls_key) {
            (Some(cert), Some(key)) => Some(trusted::Files::load(cert, key, &resolver)?),
            _ => None,
        };
        let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(provider))
            .with_safe_default_protocol_versions()?
            .with_no_client_auth()
            .with_cert_resolver(resolver.clone());
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = TlsAcceptor::from(Arc::new(tls));

        let listener = TcpListener::bind(config.listen)
            .await
            .map_err(|e| StartError::Bind(config.listen, e))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| StartError::Bind(config.listen, e))?;

        let state = Arc::new(State {
            logins: Mutex::new(HashMap::new()),
            opaque,
            db: Mutex::new(db),
            setup: SetupCode::generate(Instant::now()),
            peers: Arc::new(Peers::default()),
            push: broadcast::channel(256).0,
        });
        let info = Arc::new(Info {
            name: name.clone(),
            version: env!("CARGO_PKG_VERSION"),
        });
        let router = Router::new()
            .route(
                "/api/v1/info",
                get({
                    let info = info.clone();
                    move || async move {
                        Json(Info {
                            name: info.name.clone(),
                            version: info.version,
                        })
                    }
                }),
            )
            .route("/api/v1/join/start", post(join_start))
            .route("/api/v1/join/finish", post(join_finish))
            .route("/api/v1/login/start", post(login_start))
            .route("/api/v1/login/finish", post(login_finish))
            .route("/api/v1/login/device", post(login_device))
            .route("/api/v1/devices", post(devices))
            .route("/api/v1/lists", post(lists))
            .route("/api/v1/lists/register", post(register_list))
            .route("/api/v1/lists/keys", post(list_keys))
            .route("/api/v1/sync/append", post(append_batch))
            .route("/api/v1/sync/events", post(fetch_events))
            .route("/api/v1/sync/ws", get(sync_ws))
            .layer(DefaultBodyLimit::max(MAX_BODY))
            .with_state(state.clone());

        let (shutdown, stop) = watch::channel(false);
        let task = tokio::spawn(accept_loop(
            listener,
            acceptor,
            router,
            state.peers.clone(),
            stop,
        ));
        let mut background = Vec::new();
        if let Some(mut files) = trusted_files.take() {
            let (poll, mut stop) = (config.tls_poll, shutdown.subscribe());
            let resolver = resolver.clone();
            background.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(poll) => {}
                        _ = stop.changed() => break,
                    }
                    files.reload_if_changed(&resolver);
                }
            }));
        }
        let db_path = config.data_dir.join(crate::db::FILE_NAME);
        if let Some(cfg) = config.backup.clone() {
            // Fail now, not at 03:00, if the folder cannot be used.
            std::fs::create_dir_all(&cfg.dir).map_err(|e| {
                StartError::Backup(Box::new(BackupError::Folder(cfg.dir.clone(), e)))
            })?;
            let (path, mut stop) = (db_path.clone(), shutdown.subscribe());
            background.push(tokio::spawn(async move {
                loop {
                    let wait = backup::delay_until(now(), cfg.at);
                    tokio::select! {
                        _ = tokio::time::sleep(wait) => {}
                        _ = stop.changed() => break,
                    }
                    let (path, dir, keep) = (path.clone(), cfg.dir.clone(), cfg.keep);
                    match tokio::task::spawn_blocking(move || {
                        backup::run_once(&path, &dir, keep, now())
                    })
                    .await
                    {
                        Ok(Ok(file)) => {
                            tracing::info!("backed up the database to {}", file.display())
                        }
                        Ok(Err(e)) => tracing::error!("nightly backup failed: {e}"),
                        Err(e) => tracing::error!("nightly backup failed: {e}"),
                    }
                }
            }));
        }
        Ok(Server {
            resolver,
            backup: config.backup.clone(),
            db_path,
            background,
            local_addr,
            fingerprint: identity.fingerprint,
            cert_created: identity.created,
            name,
            state,
            shutdown,
            task,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// Fingerprint of the trusted certificate in use, if one is loaded.
    pub fn trusted_fingerprint(&self) -> Option<String> {
        self.resolver.trusted_fingerprint()
    }

    /// Take the configured backup now instead of at the night time. The server
    /// keeps serving meanwhile. Returns the file written.
    pub async fn backup_now(&self) -> Result<std::path::PathBuf, BackupError> {
        let cfg = self.backup.clone().ok_or_else(|| {
            BackupError::Folder(
                std::path::PathBuf::new(),
                std::io::Error::other("no backup folder is configured"),
            )
        })?;
        let path = self.db_path.clone();
        tokio::task::spawn_blocking(move || backup::run_once(&path, &cfg.dir, cfg.keep, now()))
            .await
            .map_err(|e| BackupError::Io(std::io::Error::other(e)))?
    }

    pub fn cert_created(&self) -> bool {
        self.cert_created
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The setup code as shown in the console.
    pub fn setup_code(&self) -> String {
        self.state.setup.display()
    }

    pub fn peers(&self) -> &Arc<Peers> {
        &self.state.peers
    }

    /// Whether `candidate` is the setup code and it still works. Joining
    /// checks it again inside the transaction that creates the account.
    pub fn check_setup_code(&self, candidate: &str) -> Result<bool, DbError> {
        let exists = self.state.db.lock().unwrap().account_exists()?;
        Ok(self.state.setup.accepts(candidate, Instant::now(), exists))
    }

    #[cfg(test)]
    fn with_db<T>(&self, f: impl FnOnce(&Db) -> T) -> T {
        f(&self.state.db.lock().unwrap())
    }

    pub async fn shutdown(self) {
        let _ = self.shutdown.send(true);
        let _ = self.task.await;
        for task in self.background {
            let _ = task.await;
        }
    }

    /// Run until the process is told to stop (ctrl-c or SIGTERM).
    pub async fn run_until_signal(self) {
        wait_for_signal().await;
        tracing::info!("shutting down");
        self.shutdown().await;
    }
}

enum ApiError {
    Bad(&'static str),
    Forbidden,
    Unauthorized,
    NotFound,
    Conflict(&'static str),
    /// The same answer for a wrong password, an unknown username and a sign-in
    /// that expired, so none of them can be told apart.
    BadLogin,
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            ApiError::Bad(why) => (StatusCode::BAD_REQUEST, why),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "the setup code is not valid"),
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                "the request is not signed by a device",
            ),
            ApiError::NotFound => (StatusCode::NOT_FOUND, "there is no such list"),
            ApiError::Conflict(why) => (StatusCode::CONFLICT, why),
            ApiError::BadLogin => (
                StatusCode::UNAUTHORIZED,
                "the username or password is wrong",
            ),
            ApiError::Internal => (StatusCode::INTERNAL_SERVER_ERROR, "something went wrong"),
        };
        (
            status,
            Json(ErrorBody {
                error: error.into(),
            }),
        )
            .into_response()
    }
}

impl From<DbError> for ApiError {
    fn from(e: DbError) -> ApiError {
        tracing::error!("database error: {e}");
        ApiError::Internal
    }
}

impl From<SyncError> for ApiError {
    fn from(e: SyncError) -> ApiError {
        match e {
            SyncError::UnknownList => ApiError::NotFound,
            SyncError::Forbidden => ApiError::Forbidden,
            SyncError::DuplicateId => ApiError::Conflict("that event id is already used"),
            SyncError::Db(e) => {
                tracing::error!("database error: {e}");
                ApiError::Internal
            }
        }
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The device that signed a request, and its account.
#[derive(Clone, Copy)]
struct Authed {
    device_id: i64,
    account_id: i64,
}

/// Check the device's signature on a request (see `hab_proto::auth`).
fn authenticate(
    state: &State,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
) -> Result<Authed, ApiError> {
    let header = |name: &str| headers.get(name).and_then(|v| v.to_str().ok());
    let device_id: i64 = header(HEADER_DEVICE)
        .and_then(|v| v.parse().ok())
        .ok_or(ApiError::Unauthorized)?;
    let time: i64 = header(HEADER_TIME)
        .and_then(|v| v.parse().ok())
        .ok_or(ApiError::Unauthorized)?;
    let signature = header(HEADER_SIGNATURE).ok_or(ApiError::Unauthorized)?;
    if (now() - time).abs() > MAX_REQUEST_AGE_SECS {
        return Err(ApiError::Unauthorized);
    }
    let device = state
        .db
        .lock()
        .unwrap()
        .device(device_id)?
        .ok_or(ApiError::Unauthorized)?;
    verify_request(
        &device.signing_public,
        device_id,
        method,
        path,
        time,
        body,
        signature,
    )
    .map_err(|_| ApiError::Unauthorized)?;
    Ok(Authed {
        device_id,
        account_id: device.account_id,
    })
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, ApiError> {
    serde_json::from_slice(body).map_err(|_| ApiError::Bad("the request is not valid"))
}

/// Every device of the caller's account, with the signature that vouches for each.
async fn devices(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<DeviceList>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/devices", &body)?;
    let devices = state.db.lock().unwrap().devices_of(who.account_id)?;
    Ok(Json(DeviceList { devices }))
}

/// The caller's lists, so a device that has just joined can find the personal one.
async fn lists(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ListRefs>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/lists", &body)?;
    let ids = state.db.lock().unwrap().lists_of(who.account_id)?;
    Ok(Json(ListRefs {
        lists: ids.into_iter().map(|list_id| ListRef { list_id }).collect(),
    }))
}

/// Make a list the caller's and store sealed copies of its key.
async fn register_list(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ListRef>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/lists/register", &body)?;
    let req: RegisterList = parse(&body)?;
    if !valid_id(&req.list_id) || req.keys.len() > 64 {
        return Err(ApiError::Bad("the list is not valid"));
    }
    if req
        .keys
        .iter()
        .any(|k| k.alg.len() > 80 || k.encapped.len() > 256 || k.ciphertext.len() > 256)
    {
        return Err(ApiError::Bad("a sealed key is not valid"));
    }
    state
        .db
        .lock()
        .unwrap()
        .register_list(who.account_id, &req.list_id, &req.keys, now())?;
    Ok(Json(ListRef {
        list_id: req.list_id,
    }))
}

/// The copies of a list's key sealed to the calling device.
async fn list_keys(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<SealedKeys>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/lists/keys", &body)?;
    let req: ListRef = parse(&body)?;
    let keys = state
        .db
        .lock()
        .unwrap()
        .sealed_keys(who.account_id, &req.list_id, who.device_id)?;
    Ok(Json(SealedKeys { keys }))
}

/// Why the server refused an event.
fn reject_reason(e: &ApiError) -> &'static str {
    match e {
        ApiError::Bad(why) => why,
        ApiError::Conflict(why) => why,
        ApiError::NotFound => "there is no such list",
        ApiError::Forbidden => "not allowed",
        ApiError::Unauthorized | ApiError::BadLogin => "the request is not signed by a device",
        ApiError::Internal => "something went wrong",
    }
}

/// Number an event, if it is from the calling device, signed, and not from
/// the future. The server never opens it: it checks the header and the
/// signature, which cover the ciphertext.
fn accept_event(state: &State, who: Authed, e: &Envelope) -> Result<Numbered, ApiError> {
    if !valid_id(&e.list_id) || !valid_id(&e.event_id) {
        return Err(ApiError::Bad("the list or event id is not valid"));
    }
    if e.alg != Algs::EVENT {
        return Err(ApiError::Bad("that encryption is not supported"));
    }
    if e.ciphertext.len() > MAX_EVENT_BYTES {
        return Err(ApiError::Bad("the event is too big"));
    }
    if e.device_id != who.device_id {
        return Err(ApiError::Forbidden);
    }
    let received = now();
    if e.clock > received + MAX_CLOCK_AHEAD_SECS {
        return Err(ApiError::Bad("the device's clock is set too far ahead"));
    }
    let db = state.db.lock().unwrap();
    let device = db.device(e.device_id)?.ok_or(ApiError::Forbidden)?;
    if device.account_id != who.account_id || e.verify(&device.signing_public).is_err() {
        return Err(ApiError::Bad("the event is not signed by its device"));
    }
    let numbered = db.append_event(who.account_id, e, received)?;
    drop(db);
    if !numbered.duplicate {
        // Nobody listening is fine.
        let _ = state.push.send(Arc::new(Pushed {
            account_id: who.account_id,
            origin: who.device_id,
            seq: numbered.seq,
            received_at: numbered.received_at,
            envelope: e.clone(),
        }));
    }
    Ok(numbered)
}

/// Bulk upload over HTTPS, such as joining with a standalone history. Events
/// are numbered in the order given.
async fn append_batch(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<AppendResults>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/sync/append", &body)?;
    let batch: AppendBatch = parse(&body)?;
    if batch.envelopes.len() > MAX_BATCH {
        return Err(ApiError::Bad("too many events at once"));
    }
    let mut results = AppendResults {
        numbered: Vec::new(),
        rejected: Vec::new(),
    };
    for e in &batch.envelopes {
        match accept_event(&state, who, e) {
            Ok(n) => results.numbered.push(n),
            Err(err) => results.rejected.push(Rejected {
                event_id: e.event_id.clone(),
                error: reject_reason(&err).into(),
            }),
        }
    }
    Ok(Json(results))
}

/// Bulk download over HTTPS.
async fn fetch_events(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<EventPage>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/sync/events", &body)?;
    let req: FetchEvents = parse(&body)?;
    let (events, more) = state.db.lock().unwrap().events_after(
        who.account_id,
        &req.list_id,
        req.after.max(0),
        req.limit.clamp(1, MAX_PAGE),
    )?;
    Ok(Json(EventPage { events, more }))
}

/// One WebSocket per device: its new events go in, numbered, and its
/// account's other devices' events come out.
async fn sync_ws(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let who = authenticate(&state, &headers, "GET", "/api/v1/sync/ws", b"")?;
    // Subscribe before the upgrade finishes, so nothing is missed after it.
    let pushes = state.push.subscribe();
    Ok(ws
        .max_message_size(MAX_EVENT_BYTES * 2)
        .on_upgrade(move |socket| ws_session(state, who, pushes, socket)))
}

async fn send(socket: &mut WebSocket, msg: &ServerMessage) -> bool {
    match serde_json::to_string(msg) {
        Ok(text) => socket.send(Message::text(text)).await.is_ok(),
        Err(_) => false,
    }
}

async fn ws_session(
    state: Arc<State>,
    who: Authed,
    mut pushes: broadcast::Receiver<Arc<Pushed>>,
    mut socket: WebSocket,
) {
    loop {
        tokio::select! {
            incoming = socket.recv() => {
                let Some(Ok(message)) = incoming else { return };
                let reply = match message {
                    Message::Text(text) => match serde_json::from_str::<ClientMessage>(&text) {
                        Ok(ClientMessage::Append { envelope }) => {
                            match accept_event(&state, who, &envelope) {
                                Ok(n) => ServerMessage::Numbered(n),
                                Err(e) => ServerMessage::Rejected(Rejected {
                                    event_id: envelope.event_id.clone(),
                                    error: reject_reason(&e).into(),
                                }),
                            }
                        }
                        Err(_) => return,
                    },
                    Message::Close(_) => return,
                    _ => continue,
                };
                if !send(&mut socket, &reply).await {
                    return;
                }
            }
            pushed = pushes.recv() => {
                let reply = match pushed {
                    Ok(p) if p.account_id == who.account_id && p.origin != who.device_id => {
                        ServerMessage::Event {
                            seq: p.seq,
                            received_at: p.received_at,
                            envelope: p.envelope.clone(),
                        }
                    }
                    Ok(_) => continue,
                    Err(broadcast::error::RecvError::Lagged(_)) => ServerMessage::Resync,
                    Err(broadcast::error::RecvError::Closed) => return,
                };
                if !send(&mut socket, &reply).await {
                    return;
                }
            }
        }
    }
}

fn check_names(username: &str) -> Result<(), ApiError> {
    if valid_username(username) {
        Ok(())
    } else {
        Err(ApiError::Bad("the username is not allowed"))
    }
}

/// Step 1 of creating the first account: answer OPAQUE's first message. The
/// setup code has to be right already, so strangers get nothing to work with.
async fn join_start(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<JoinStart>,
) -> Result<Json<JoinStarted>, ApiError> {
    check_names(&req.username)?;
    let exists = state.db.lock().unwrap().account_exists()?;
    if !state.setup.accepts(&req.setup_code, Instant::now(), exists) {
        return Err(ApiError::Forbidden);
    }
    let request = RegistrationRequest::<Suite>::deserialize(&req.registration_request)
        .map_err(|_| ApiError::Bad("the registration request is not valid"))?;
    let started =
        ServerRegistration::<Suite>::start(&state.opaque, request, req.username.as_bytes())
            .map_err(|_| ApiError::Bad("the registration request is not valid"))?;
    Ok(Json(JoinStarted {
        registration_response: started.message.serialize().to_vec(),
    }))
}

/// Step 2: store the account. The setup code is checked again, together with
/// whether an account exists, in the same transaction as the insert.
async fn join_finish(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<JoinFinish>,
) -> Result<Json<Joined>, ApiError> {
    check_names(&req.username)?;
    if !valid_display_name(&req.display_name) {
        return Err(ApiError::Bad("the display name is not allowed"));
    }
    if req.identity_alg != Algs::IDENTITY || req.identity_public.len() != 32 {
        return Err(ApiError::Bad("the identity key is not valid"));
    }
    if req.kdf.alg != "argon2id"
        || req.kdf.memory_kib < ARGON_MEMORY_KIB
        || req.kdf.passes < ARGON_PASSES
        || req.kdf.lanes < ARGON_LANES
        || req.kdf.memory_kib > 4 * ARGON_MEMORY_KIB
        || req.kdf.passes > 10
        || req.kdf.lanes > 16
    {
        return Err(ApiError::Bad("the password hardening is too weak"));
    }
    if req.bundle.alg != Algs::BUNDLE
        || req.bundle.nonce.len() != 24
        || req.bundle.ciphertext.len() > 1024
    {
        return Err(ApiError::Bad("the key bundle is not valid"));
    }
    if verify_device(&req.identity_public, &req.device).is_err() {
        return Err(ApiError::Bad(
            "the device is not signed by the identity key",
        ));
    }
    let upload = RegistrationUpload::<Suite>::deserialize(&req.registration_upload)
        .map_err(|_| ApiError::Bad("the registration is not valid"))?;
    let record = ServerRegistration::<Suite>::finish(upload)
        .serialize()
        .to_vec();

    let d = req.device;
    let new = NewAccount {
        username: req.username,
        display_name: req.display_name.trim().to_string(),
        identity_alg: req.identity_alg,
        identity_public: req.identity_public,
        opaque_record: record,
        kdf_alg: req.kdf.alg,
        kdf_memory_kib: req.kdf.memory_kib,
        kdf_passes: req.kdf.passes,
        kdf_lanes: req.kdf.lanes,
        bundle_alg: req.bundle.alg,
        bundle_nonce: req.bundle.nonce,
        bundle: req.bundle.ciphertext,
        device: NewDevice {
            portable: d.portable,
            alg: d.alg,
            signing_public: d.signing_public,
            sealing_public: d.sealing_public,
            signature: d.signature,
        },
    };
    let now = now();
    let code = req.setup_code;
    let created = state.db.lock().unwrap().create_first_account(
        |exists| state.setup.accepts(&code, Instant::now(), exists),
        &new,
        now,
    )?;
    match created {
        Some(c) => {
            tracing::info!("the first account was created");
            Ok(Json(Joined {
                account_id: c.account_id,
                device_id: c.device_id,
                admin: true,
            }))
        }
        None => Err(ApiError::Forbidden),
    }
}

fn new_login_id() -> String {
    use opaque_ke::rand::RngCore;
    let mut bytes = [0u8; 16];
    opaque_ke::rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sign in, step 1: answer OPAQUE's first message. A username the server
/// doesn't know gets an answer of the same shape, from a made-up record.
async fn login_start(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<LoginStart>,
) -> Result<Json<LoginStarted>, ApiError> {
    check_names(&req.username)?;
    let request = CredentialRequest::<Suite>::deserialize(&req.credential_request)
        .map_err(|_| ApiError::Bad("the sign-in request is not valid"))?;
    let account = state.db.lock().unwrap().login_account(&req.username)?;
    let record = account
        .as_ref()
        .and_then(|a| ServerRegistration::<Suite>::deserialize(&a.opaque_record).ok());
    let started = ServerLogin::start(
        &mut opaque_ke::rand::rngs::OsRng,
        &state.opaque,
        record,
        request,
        req.username.as_bytes(),
        ServerLoginParameters::default(),
    )
    .map_err(|_| ApiError::Bad("the sign-in request is not valid"))?;
    let kdf = match &account {
        Some(a) => Kdf {
            alg: a.kdf_alg.clone(),
            memory_kib: a.kdf_memory_kib,
            passes: a.kdf_passes,
            lanes: a.kdf_lanes,
        },
        None => Kdf {
            alg: "argon2id".into(),
            memory_kib: ARGON_MEMORY_KIB,
            passes: ARGON_PASSES,
            lanes: ARGON_LANES,
        },
    };
    let login_id = new_login_id();
    {
        let mut logins = state.logins.lock().unwrap();
        logins.retain(|_, (at, _)| at.elapsed() < LOGIN_TTL);
        if logins.len() >= MAX_LOGINS {
            return Err(ApiError::Conflict("too many sign-ins at once, try again"));
        }
        logins.insert(
            login_id.clone(),
            (
                Instant::now(),
                Login::Started(Box::new(started.state), account.map(Box::new)),
            ),
        );
    }
    Ok(Json(LoginStarted {
        login_id,
        credential_response: started.message.serialize().to_vec(),
        kdf,
    }))
}

/// Sign in, step 2: check the client's proof of the password. Only then is the
/// key bundle released. One try per sign-in.
async fn login_finish(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<LoginFinish>,
) -> Result<Json<LoginFinished>, ApiError> {
    let taken = state.logins.lock().unwrap().remove(&req.login_id);
    let Some((at, Login::Started(login, account))) = taken else {
        return Err(ApiError::BadLogin);
    };
    if at.elapsed() >= LOGIN_TTL {
        return Err(ApiError::BadLogin);
    }
    let finalization = CredentialFinalization::<Suite>::deserialize(&req.credential_finalization)
        .map_err(|_| ApiError::BadLogin)?;
    login
        .finish(finalization, ServerLoginParameters::default())
        .map_err(|_| ApiError::BadLogin)?;
    // A made-up record can't pass the proof, so there is an account here.
    let a = account.ok_or(ApiError::BadLogin)?;
    state.logins.lock().unwrap().insert(
        req.login_id,
        (
            Instant::now(),
            Login::Verified {
                account_id: a.id,
                admin: a.admin,
                identity_public: a.identity_public.clone(),
            },
        ),
    );
    Ok(Json(LoginFinished {
        display_name: a.display_name,
        admin: a.admin,
        identity_public: a.identity_public,
        bundle: KeyBundle {
            alg: a.bundle_alg,
            nonce: a.bundle_nonce,
            ciphertext: a.bundle,
        },
    }))
}

/// Sign in, step 3: add the device the identity key has signed. The server
/// checks the signature against the account's identity key, so a device can
/// only be added by someone who unlocked that key.
async fn login_device(
    AxumState(state): AxumState<Arc<State>>,
    Json(req): Json<LoginDevice>,
) -> Result<Json<Joined>, ApiError> {
    let taken = state.logins.lock().unwrap().remove(&req.login_id);
    let Some((
        at,
        Login::Verified {
            account_id,
            admin,
            identity_public,
        },
    )) = taken
    else {
        return Err(ApiError::BadLogin);
    };
    if at.elapsed() >= LOGIN_TTL {
        return Err(ApiError::BadLogin);
    }
    if verify_device(&identity_public, &req.device).is_err() {
        return Err(ApiError::Bad(
            "the device is not signed by the identity key",
        ));
    }
    let d = req.device;
    let new = NewDevice {
        portable: d.portable,
        alg: d.alg,
        signing_public: d.signing_public,
        sealing_public: d.sealing_public,
        signature: d.signature,
    };
    let added = state
        .db
        .lock()
        .unwrap()
        .add_device(account_id, &new, now())?;
    match added {
        Some(device_id) => {
            tracing::info!("a device signed in");
            Ok(Json(Joined {
                account_id,
                device_id,
                admin,
            }))
        }
        None => Err(ApiError::Conflict("this account has too many devices")),
    }
}

async fn wait_for_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    router: Router,
    peers: Arc<Peers>,
    mut stop: watch::Receiver<bool>,
) {
    loop {
        let (stream, peer) = tokio::select! {
            r = listener.accept() => match r {
                Ok(x) => x,
                Err(e) => {
                    tracing::warn!("accept failed: {e}");
                    continue;
                }
            },
            _ = stop.changed() => break,
        };
        // The address goes to memory for limits, and to the log only at debug.
        let guard = peers.enter(peer.ip());
        tracing::debug!(%peer, "connection opened");
        let acceptor = acceptor.clone();
        let service = TowerToHyperService::new(router.clone());
        tokio::spawn(async move {
            let _guard = guard;
            match acceptor.accept(stream).await {
                Ok(tls) => {
                    let conn = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service)
                        .with_upgrades();
                    if let Err(e) = conn.await {
                        tracing::debug!(%peer, "connection ended: {e}");
                    }
                }
                Err(e) => tracing::debug!(%peer, "tls handshake failed: {e}"),
            }
            tracing::debug!(%peer, "connection closed");
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(dir: &std::path::Path) -> Config {
        Config {
            data_dir: dir.to_path_buf(),
            listen: "127.0.0.1:0".parse().unwrap(),
            ..Config::default()
        }
    }

    #[tokio::test]
    async fn setup_code_works_until_the_first_account_exists() {
        let dir = tempfile::tempdir().unwrap();
        let server = Server::start(&config(dir.path())).await.unwrap();
        let code = server.setup_code();
        assert!(server.check_setup_code(&code).unwrap());
        assert!(!server.check_setup_code("wrong").unwrap());
        server.with_db(|db| {
            db.create_first_account(|_| true, &crate::db::test_account("chris"), 0)
                .unwrap()
                .unwrap()
        });
        assert!(!server.check_setup_code(&code).unwrap());
        server.shutdown().await;
    }

    #[tokio::test]
    async fn restarting_makes_a_new_setup_code_but_keeps_the_certificate() {
        let dir = tempfile::tempdir().unwrap();
        let first = Server::start(&config(dir.path())).await.unwrap();
        let (code, fp) = (first.setup_code(), first.fingerprint().to_string());
        assert!(first.cert_created());
        first.shutdown().await;

        let second = Server::start(&config(dir.path())).await.unwrap();
        assert!(!second.cert_created());
        assert_eq!(second.fingerprint(), fp);
        assert_ne!(second.setup_code(), code);
        assert!(!second.check_setup_code(&code).unwrap());
        second.shutdown().await;
    }

    #[tokio::test]
    async fn name_is_kept_until_changed() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = config(dir.path());
        c.name = Some("Home".into());
        Server::start(&c).await.unwrap().shutdown().await;
        c.name = None;
        let s = Server::start(&c).await.unwrap();
        assert_eq!(s.name(), "Home");
        s.shutdown().await;
    }
}
