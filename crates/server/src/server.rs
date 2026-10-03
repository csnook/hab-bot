//! The HTTPS listener and the routes behind it.

use crate::backup::{self, BackupError};
use crate::cert::{self, CertError};
use crate::config::{Config, DEFAULT_NAME};
use crate::db::{Db, DbError, LoginAccount, NewAccount, NewDevice, RemoveError, SyncError};
use crate::guard::{self, Clock, Guard};
use crate::peers::Peers;
use crate::setup::SetupCode;
use crate::trusted::{self, TrustedError};
use axum::body::Bytes;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{DefaultBodyLimit, State as AxumState};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Extension;
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
    valid_display_name, valid_id, valid_username, Algs, AppendBatch, AppendResults, ApprovalBlob,
    ApprovalCollect, ApprovalCollected, ApprovalFetched, ApprovalGrant, ApprovalGranted,
    ApprovalOpen, ApprovalRef, ApprovalRequest, ClientMessage, DeviceList, Envelope, ErrorBody,
    EventPage, FetchEvents, FetchNotices, JoinFinish, JoinStart, JoinStarted, Joined, Kdf,
    KeyBundle, ListRef, ListRefs, LoginDevice, LoginFinish, LoginFinished, LoginStart,
    LoginStarted, NoticeKinds, Notices, Numbered, PasswordFinish, PasswordStart, PasswordStarted,
    RegisterList, Rejected, RemoveDevice, SealedKeys, ServerMessage, ServerNotice,
    MAX_APPROVAL_BLOB, MAX_CLOCK_AHEAD_SECS, MAX_EVENT_BYTES,
};
use hab_proto::{verify_device, Suite, ARGON_LANES, ARGON_MEMORY_KIB, ARGON_PASSES};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use serde::Serialize;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
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

/// The address a request came from, put on each connection's requests. It
/// is only ever used for limits, held in memory.
#[derive(Clone, Copy)]
struct PeerIp(IpAddr);

/// A device was taken off an account, and the account's keys rotated.
#[derive(Clone, Copy)]
struct Removal {
    account_id: i64,
    device_id: i64,
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

/// How long an approval of a new device by an existing one may take, from the
/// moment its code is made.
const APPROVAL_TTL: Duration = Duration::from_secs(300);
/// Approvals in progress at once, so strangers can't fill the memory.
const MAX_APPROVALS: usize = 1000;
/// Approvals one account may have in progress.
const MAX_APPROVALS_PER_ACCOUNT: usize = 5;

/// One new device being approved by an existing one (ADR 0004). Kept in
/// memory only, like a sign-in: a restart ends it. Its id is 128 random bits
/// and it is used once, so there is nothing to guess.
struct Approval {
    at: Instant,
    /// The account whose device offered the approval, or None if the new
    /// device started it and the first device of an account to approve it
    /// (having read the code) takes it.
    account_id: Option<i64>,
    request: Option<(ApprovalBlob, String)>,
    granted: Option<ApprovalGranted>,
}

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
    approvals: Mutex<HashMap<String, Approval>>,
    push: broadcast::Sender<Arc<Pushed>>,
    removals: broadcast::Sender<Removal>,
    /// Notices for an account's devices, as (account id, notice).
    notices: broadcast::Sender<(i64, ServerNotice)>,
    /// Failed sign-in protection and per-address limits (see `guard`).
    guard: Mutex<Guard>,
    clock: Clock,
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
        Server::start_with_clock(config, guard::system_clock()).await
    }

    /// [`Server::start`] with the clock that the sign-in backoff and the
    /// per-address limits run on, so a test can move it.
    pub async fn start_with_clock(config: &Config, clock: Clock) -> Result<Server, StartError> {
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
            approvals: Mutex::new(HashMap::new()),
            opaque,
            db: Mutex::new(db),
            setup: SetupCode::generate(Instant::now()),
            peers: Arc::new(Peers::default()),
            push: broadcast::channel(256).0,
            removals: broadcast::channel(64).0,
            notices: broadcast::channel(64).0,
            guard: Mutex::new(Guard::default()),
            clock,
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
            .route("/api/v1/approvals/create", post(approval_create))
            .route("/api/v1/approvals/open", post(approval_open))
            .route("/api/v1/approvals/request", post(approval_request))
            .route("/api/v1/approvals/fetch", post(approval_fetch))
            .route("/api/v1/approvals/grant", post(approval_grant))
            .route("/api/v1/approvals/collect", post(approval_collect))
            .route("/api/v1/password/start", post(password_start))
            .route("/api/v1/password/finish", post(password_finish))
            .route("/api/v1/notices", post(notices))
            .route("/api/v1/devices", post(devices))
            .route("/api/v1/devices/remove", post(remove_device))
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
        {
            // Failed sign-ins are only known once an attempt has gone
            // unfinished for a while, so look for them as time passes.
            let (state, mut stop) = (state.clone(), shutdown.subscribe());
            background.push(tokio::spawn(async move {
                loop {
                    tokio::select! {
                        _ = tokio::time::sleep(Duration::from_secs(1)) => {}
                        _ = stop.changed() => break,
                    }
                    settle_failures(&state);
                }
            }));
        }
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
    /// The same answer for an approval that never was, expired, was used, or
    /// is someone else's.
    NoApproval,
    Conflict(&'static str),
    /// The same answer for a wrong password, an unknown username and a sign-in
    /// that expired, so none of them can be told apart.
    BadLogin,
    /// An address made too many calls in a minute. Says nothing about any
    /// account.
    TooMany,
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
            ApiError::NoApproval => (
                StatusCode::NOT_FOUND,
                "that approval has expired, been used, or never existed",
            ),
            ApiError::Conflict(why) => (StatusCode::CONFLICT, why),
            ApiError::BadLogin => (
                StatusCode::UNAUTHORIZED,
                "the username or password is wrong",
            ),
            ApiError::TooMany => (
                StatusCode::TOO_MANY_REQUESTS,
                "too many requests from this address, try again in a minute",
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
    // Only timing is kept, and a failure to keep it must not fail the request.
    let _ = state.db.lock().unwrap().touch_device(device_id, now());
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
    let db = state.db.lock().unwrap();
    let devices = db.devices_of(who.account_id)?;
    let retired = db.retired_of(who.account_id)?;
    Ok(Json(DeviceList { devices, retired }))
}

/// Take another device of the caller's account off it, and store the rotated
/// keys of the account's lists, in one step. The removed device can no longer
/// sign a request, so it can't fetch or send anything, and its open
/// connection is closed.
async fn remove_device(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/devices/remove", &body)?;
    let req: RemoveDevice = parse(&body)?;
    let sealed = req.lists.iter().map(|l| l.keys.len()).sum::<usize>();
    if req.lists.len() > 64 || sealed > 4096 {
        return Err(ApiError::Bad("the request is too big"));
    }
    if req
        .lists
        .iter()
        .flat_map(|l| &l.keys)
        .any(|k| k.alg.len() > 80 || k.encapped.len() > 256 || k.ciphertext.len() > 256)
    {
        return Err(ApiError::Bad("a sealed key is not valid"));
    }
    state
        .db
        .lock()
        .unwrap()
        .remove_device(
            who.account_id,
            who.device_id,
            req.device_id,
            &req.lists,
            now(),
        )
        .map_err(|e| match e {
            RemoveError::Db(e) => {
                tracing::error!("database error: {e}");
                ApiError::Internal
            }
            RemoveError::OwnDevice => {
                ApiError::Bad("a device can only be removed from another device")
            }
            RemoveError::UnknownDevice => ApiError::NotFound,
            RemoveError::Incomplete => {
                ApiError::Bad("the new keys do not cover every list and every remaining device")
            }
            RemoveError::Conflict => {
                ApiError::Conflict("the account's devices or keys changed; try again")
            }
        })?;
    tracing::info!("a device was removed");
    // Nobody listening is fine.
    let _ = state.removals.send(Removal {
        account_id: who.account_id,
        device_id: req.device_id,
    });
    Ok(Json(serde_json::json!({})))
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
    if !valid_id(&req.list_id) || req.keys.len() > 4096 {
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
        ApiError::NotFound | ApiError::NoApproval => "there is no such list",
        ApiError::Forbidden => "not allowed",
        ApiError::Unauthorized | ApiError::BadLogin => "the request is not signed by a device",
        ApiError::TooMany => "too many requests",
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
    let removals = state.removals.subscribe();
    let notices = state.notices.subscribe();
    Ok(ws
        .max_message_size(MAX_EVENT_BYTES * 2)
        .on_upgrade(move |socket| ws_session(state, who, pushes, removals, notices, socket)))
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
    mut removals: broadcast::Receiver<Removal>,
    mut notices: broadcast::Receiver<(i64, ServerNotice)>,
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
                    Message::Ping(_) => {
                        // A connected device is syncing.
                        let _ = state.db.lock().unwrap().touch_device(who.device_id, now());
                        continue;
                    }
                    _ => continue,
                };
                if !send(&mut socket, &reply).await {
                    return;
                }
            }
            removed = removals.recv() => match removed {
                Ok(r) if r.account_id != who.account_id => continue,
                Ok(r) if r.device_id == who.device_id => return,
                Ok(_) => {
                    if !send(&mut socket, &ServerMessage::KeysChanged).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if !still_on_account(&state, who) {
                        return;
                    }
                    if !send(&mut socket, &ServerMessage::KeysChanged).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            noticed = notices.recv() => match noticed {
                Ok((account_id, _)) if account_id != who.account_id => continue,
                Ok((_, notice)) => {
                    if !send(&mut socket, &ServerMessage::Notice(notice)).await {
                        return;
                    }
                }
                // Missed some: the device fetches the ones it lacks.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    if !send(&mut socket, &ServerMessage::Resync).await {
                        return;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            pushed = pushes.recv() => {
                // Once a device is off the account it hears nothing more, even
                // before its connection is closed.
                if !still_on_account(&state, who) {
                    return;
                }
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

/// Whether the device is still one of its account's.
fn still_on_account(state: &State, who: Authed) -> bool {
    matches!(
        state.db.lock().unwrap().device(who.device_id),
        Ok(Some(d)) if d.account_id == who.account_id
    )
}

/// A password's hardening must be at least what this release does, and not
/// so much that signing in becomes a way to exhaust a device.
fn check_kdf(kdf: &Kdf) -> Result<(), ApiError> {
    if kdf.alg != "argon2id"
        || kdf.memory_kib < ARGON_MEMORY_KIB
        || kdf.passes < ARGON_PASSES
        || kdf.lanes < ARGON_LANES
        || kdf.memory_kib > 4 * ARGON_MEMORY_KIB
        || kdf.passes > 10
        || kdf.lanes > 16
    {
        return Err(ApiError::Bad("the password hardening is too weak"));
    }
    Ok(())
}

fn check_bundle(bundle: &KeyBundle) -> Result<(), ApiError> {
    if bundle.alg != Algs::BUNDLE || bundle.nonce.len() != 24 || bundle.ciphertext.len() > 1024 {
        return Err(ApiError::Bad("the key bundle is not valid"));
    }
    Ok(())
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
    check_kdf(&req.kdf)?;
    check_bundle(&req.bundle)?;
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
///
/// Every start is a password attempt. The password is checked on the client,
/// which sees a wrong one fail and never sends the last message, so the server
/// can't wait to be told. It counts each start for the account and for the
/// address, and an account that is waiting out its backoff is answered like an
/// unknown username: with nothing a password could be tried against.
async fn login_start(
    AxumState(state): AxumState<Arc<State>>,
    Extension(PeerIp(ip)): Extension<PeerIp>,
    Json(req): Json<LoginStart>,
) -> Result<Json<LoginStarted>, ApiError> {
    let now = (state.clock)();
    if !state.guard.lock().unwrap().password_attempt_from(ip, now) {
        return Err(ApiError::TooMany);
    }
    check_names(&req.username)?;
    let request = CredentialRequest::<Suite>::deserialize(&req.credential_request)
        .map_err(|_| ApiError::Bad("the sign-in request is not valid"))?;
    let mut account = state.db.lock().unwrap().login_account(&req.username)?;
    if let Some(a) = &account {
        if !state.guard.lock().unwrap().start_attempt(a.id, now) {
            account = None;
        }
    }
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
    // Notices come due as time passes; this is as good a moment as the tick.
    settle_failures(&state);
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
    // The password was proven, so the attempts that came before were not
    // failures of a stranger's.
    state.guard.lock().unwrap().succeeded(a.id);
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

/// Notices that have come due for failed sign-ins: keep each and offer it to
/// the account's connected devices.
fn settle_failures(state: &State) {
    let due = state.guard.lock().unwrap().settle((state.clock)());
    for (account_id, count) in due {
        let kept = state.db.lock().unwrap().add_notice(
            account_id,
            NoticeKinds::FAILED_SIGN_INS,
            count,
            now(),
        );
        match kept {
            // Nobody listening is fine: devices fetch what they missed.
            Ok(notice) => {
                let _ = state.notices.send((account_id, notice));
            }
            Err(e) => tracing::error!("cannot keep a notice: {e}"),
        }
    }
}

// ---- Changing the password ----

/// Changing the password, step 1: answer OPAQUE's first registration message
/// for the caller's own account. The caller is a device of the account, which
/// is all that a forgotten password needs, and the old password is not asked
/// for. This is not a password attempt and nothing here counts as a failed
/// sign-in.
async fn password_start(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<PasswordStarted>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/password/start", &body)?;
    let req: PasswordStart = parse(&body)?;
    let username = state
        .db
        .lock()
        .unwrap()
        .username_of(who.account_id)?
        .ok_or(ApiError::Internal)?;
    let request = RegistrationRequest::<Suite>::deserialize(&req.registration_request)
        .map_err(|_| ApiError::Bad("the registration request is not valid"))?;
    let started = ServerRegistration::<Suite>::start(&state.opaque, request, username.as_bytes())
        .map_err(|_| ApiError::Bad("the registration request is not valid"))?;
    Ok(Json(PasswordStarted {
        registration_response: started.message.serialize().to_vec(),
    }))
}

/// Step 2: store the new OPAQUE record and the keys sealed under the new
/// password, in one update. The identity key, the devices and their sealed
/// list keys stay as they are, and devices sign requests with their own keys,
/// so none of them notices. A sign-in that is under way is ended, so the old
/// password can't finish it, and the account's failed attempts are forgotten:
/// whoever was guessing the old password has nothing to go on now.
async fn password_finish(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/password/finish", &body)?;
    let req: PasswordFinish = parse(&body)?;
    check_kdf(&req.kdf)?;
    check_bundle(&req.bundle)?;
    let upload = RegistrationUpload::<Suite>::deserialize(&req.registration_upload)
        .map_err(|_| ApiError::Bad("the registration is not valid"))?;
    let record = ServerRegistration::<Suite>::finish(upload)
        .serialize()
        .to_vec();
    {
        // Held across the swap, so a sign-in can't begin on the old record
        // after the logins were cleared.
        let mut logins = state.logins.lock().unwrap();
        state
            .db
            .lock()
            .unwrap()
            .set_password(who.account_id, &record, &req.kdf, &req.bundle)?;
        logins.retain(|_, (_, login)| match login {
            Login::Started(_, Some(a)) => a.id != who.account_id,
            Login::Verified { account_id, .. } => *account_id != who.account_id,
            Login::Started(_, None) => true,
        });
    }
    state.guard.lock().unwrap().forget(who.account_id);
    tracing::info!("a password was changed");
    Ok(Json(serde_json::json!({})))
}

/// The account's notices after the caller's last, and the newest number.
async fn notices(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Notices>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/notices", &body)?;
    let req: FetchNotices = parse(&body)?;
    let db = state.db.lock().unwrap();
    Ok(Json(Notices {
        notices: db.notices_after(who.account_id, req.after.max(0))?,
        latest: db.latest_notice(who.account_id)?,
    }))
}

// ---- Approving a new device from an existing one (ADR 0004) ----

/// Count a call to an approval route that no password guards against its
/// address's limit. Approval failures are not failed sign-ins and never touch
/// an account's count.
fn approval_call(state: &State, ip: IpAddr) -> Result<(), ApiError> {
    if state
        .guard
        .lock()
        .unwrap()
        .approval_call_from(ip, (state.clock)())
    {
        Ok(())
    } else {
        Err(ApiError::TooMany)
    }
}

fn new_approval_id() -> String {
    use opaque_ke::rand::RngCore;
    let mut bytes = [0u8; 16];
    opaque_ke::rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn blob_ok(b: &ApprovalBlob) -> bool {
    b.alg == Algs::APPROVAL && b.nonce.len() == 24 && b.ciphertext.len() <= MAX_APPROVAL_BLOB
}

fn token_ok(t: &str) -> bool {
    (16..=128).contains(&t.len())
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Constant-time comparison, so a token can't be found out a byte at a time.
fn same_token(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// Forget approvals that ran out of time, and make room for one more.
fn make_room(approvals: &mut HashMap<String, Approval>) -> Result<(), ApiError> {
    approvals.retain(|_, a| a.at.elapsed() < APPROVAL_TTL);
    if approvals.len() >= MAX_APPROVALS {
        return Err(ApiError::Conflict("too many approvals at once, try again"));
    }
    Ok(())
}

/// The approval `id`, if it is still good and, when it belongs to an account,
/// that account's.
fn live<'a>(
    approvals: &'a mut HashMap<String, Approval>,
    id: &str,
    account_id: Option<i64>,
) -> Result<&'a mut Approval, ApiError> {
    let a = approvals.get_mut(id).ok_or(ApiError::NoApproval)?;
    if a.at.elapsed() >= APPROVAL_TTL {
        approvals.remove(id);
        return Err(ApiError::NoApproval);
    }
    // Re-borrow: the removal above ends the first borrow.
    let a = approvals.get_mut(id).ok_or(ApiError::NoApproval)?;
    if let (Some(owner), Some(caller)) = (a.account_id, account_id) {
        if owner != caller {
            return Err(ApiError::NoApproval);
        }
    }
    Ok(a)
}

/// A signed-in device offers to approve a new device that will scan its code.
async fn approval_create(
    AxumState(state): AxumState<Arc<State>>,
    Extension(PeerIp(ip)): Extension<PeerIp>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ApprovalRef>, ApiError> {
    approval_call(&state, ip)?;
    let who = authenticate(&state, &headers, "POST", "/api/v1/approvals/create", &body)?;
    let mut approvals = state.approvals.lock().unwrap();
    make_room(&mut approvals)?;
    if approvals
        .values()
        .filter(|a| a.account_id == Some(who.account_id))
        .count()
        >= MAX_APPROVALS_PER_ACCOUNT
    {
        return Err(ApiError::Conflict("too many approvals at once, try again"));
    }
    let id = new_approval_id();
    approvals.insert(
        id.clone(),
        Approval {
            at: Instant::now(),
            account_id: Some(who.account_id),
            request: None,
            granted: None,
        },
    );
    Ok(Json(ApprovalRef { approval_id: id }))
}

/// A new device that shows a code starts an approval, with its request. It
/// proves nothing yet: only an existing device that has read the code (which
/// holds the id and the key) can answer it, and only the new device can
/// collect the answer.
async fn approval_open(
    AxumState(state): AxumState<Arc<State>>,
    Extension(PeerIp(ip)): Extension<PeerIp>,
    Json(req): Json<ApprovalOpen>,
) -> Result<Json<ApprovalRef>, ApiError> {
    approval_call(&state, ip)?;
    let id = req.approval_id;
    let id_ok = id.len() == 32 && id.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f'));
    if !id_ok || !blob_ok(&req.request) || !token_ok(&req.collect_token) {
        return Err(ApiError::Bad("the approval is not valid"));
    }
    let mut approvals = state.approvals.lock().unwrap();
    make_room(&mut approvals)?;
    if approvals.contains_key(&id) {
        return Err(ApiError::Conflict("that approval already exists"));
    }
    approvals.insert(
        id.clone(),
        Approval {
            at: Instant::now(),
            account_id: None,
            request: Some((req.request, req.collect_token)),
            granted: None,
        },
    );
    Ok(Json(ApprovalRef { approval_id: id }))
}

/// A new device that scanned an existing device's code sends its request.
async fn approval_request(
    AxumState(state): AxumState<Arc<State>>,
    Extension(PeerIp(ip)): Extension<PeerIp>,
    Json(req): Json<ApprovalRequest>,
) -> Result<Json<serde_json::Value>, ApiError> {
    approval_call(&state, ip)?;
    if !blob_ok(&req.request) || !token_ok(&req.collect_token) {
        return Err(ApiError::Bad("the approval is not valid"));
    }
    let mut approvals = state.approvals.lock().unwrap();
    let a = live(&mut approvals, &req.approval_id, None)?;
    // Offered by a device, and not yet answered by another new one.
    if a.account_id.is_none() || a.request.is_some() {
        return Err(ApiError::NoApproval);
    }
    a.request = Some((req.request, req.collect_token));
    Ok(Json(serde_json::json!({})))
}

/// The existing device reads the new device's request, to show its name.
async fn approval_fetch(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<ApprovalFetched>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/approvals/fetch", &body)?;
    let req: ApprovalRef = parse(&body)?;
    let mut approvals = state.approvals.lock().unwrap();
    let a = live(&mut approvals, &req.approval_id, Some(who.account_id))?;
    Ok(Json(ApprovalFetched {
        request: a.request.as_ref().map(|(blob, _)| blob.clone()),
    }))
}

/// The existing device approves: the server checks the identity key signed the
/// new device, adds it, and keeps the sealed keys for it to collect.
async fn approval_grant(
    AxumState(state): AxumState<Arc<State>>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Joined>, ApiError> {
    let who = authenticate(&state, &headers, "POST", "/api/v1/approvals/grant", &body)?;
    let req: ApprovalGrant = parse(&body)?;
    if !blob_ok(&req.grant) {
        return Err(ApiError::Bad("the approval is not valid"));
    }
    let mut approvals = state.approvals.lock().unwrap();
    let a = live(&mut approvals, &req.approval_id, Some(who.account_id))?;
    if a.request.is_none() || a.granted.is_some() {
        return Err(ApiError::NoApproval);
    }
    let db = state.db.lock().unwrap();
    let (identity_public, admin) = db.identity_of(who.account_id)?.ok_or(ApiError::Internal)?;
    if verify_device(&identity_public, &req.device).is_err() {
        return Err(ApiError::Bad(
            "the device is not signed by the identity key",
        ));
    }
    let d = req.device.clone();
    let new = NewDevice {
        portable: d.portable,
        alg: d.alg,
        signing_public: d.signing_public,
        sealing_public: d.sealing_public,
        signature: d.signature,
    };
    let Some(device_id) = db.add_device(who.account_id, &new, now())? else {
        return Err(ApiError::Conflict("this account has too many devices"));
    };
    let joined = Joined {
        account_id: who.account_id,
        device_id,
        admin,
    };
    a.account_id = Some(who.account_id);
    a.granted = Some(ApprovalGranted {
        joined: joined.clone(),
        device: req.device,
        grant: req.grant,
    });
    tracing::info!("a device was approved");
    Ok(Json(joined))
}

/// The new device collects the answer, once.
async fn approval_collect(
    AxumState(state): AxumState<Arc<State>>,
    Extension(PeerIp(ip)): Extension<PeerIp>,
    Json(req): Json<ApprovalCollect>,
) -> Result<Json<ApprovalCollected>, ApiError> {
    approval_call(&state, ip)?;
    let mut approvals = state.approvals.lock().unwrap();
    let a = live(&mut approvals, &req.approval_id, None)?;
    match &a.request {
        Some((_, token)) if same_token(token, &req.collect_token) => {}
        _ => return Err(ApiError::NoApproval),
    }
    if a.granted.is_none() {
        return Ok(Json(ApprovalCollected { granted: None }));
    }
    let done = approvals.remove(&req.approval_id).and_then(|a| a.granted);
    Ok(Json(ApprovalCollected { granted: done }))
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
        // The address goes on the connection's requests for the limits.
        let service = TowerToHyperService::new(router.clone().layer(Extension(PeerIp(peer.ip()))));
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
