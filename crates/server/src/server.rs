//! The HTTPS listener and the routes behind it.

use crate::cert::{self, CertError};
use crate::config::{Config, DEFAULT_NAME};
use crate::db::{Db, DbError, NewAccount, NewDevice};
use crate::peers::Peers;
use crate::setup::SetupCode;
use axum::extract::State as AxumState;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{routing::get, routing::post, Json, Router};
use base64::{engine::general_purpose::STANDARD, Engine};
use hab_proto::opaque_ke::{
    self, RegistrationRequest, RegistrationUpload, ServerRegistration, ServerSetup,
};
use hab_proto::wire::{
    valid_display_name, valid_username, Algs, ErrorBody, JoinFinish, JoinStart, JoinStarted, Joined,
};
use hab_proto::{verify_device, Suite, ARGON_LANES, ARGON_MEMORY_KIB, ARGON_PASSES};
use hyper_util::rt::TokioIo;
use hyper_util::service::TowerToHyperService;
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::watch;
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
    #[error("tls setup failed: {0}")]
    Tls(#[from] rustls::Error),
}

/// What `GET /api/v1/info` reports, for apps to check before joining.
#[derive(Serialize)]
struct Info {
    name: String,
    version: &'static str,
}

struct State {
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
    shutdown: watch::Sender<bool>,
    task: JoinHandle<()>,
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

        let tls = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(vec![identity.cert.clone()], identity.key.clone_key())?;
        let mut tls = tls;
        tls.alpn_protocols = vec![b"http/1.1".to_vec()];
        let acceptor = TlsAcceptor::from(Arc::new(tls));

        let listener = TcpListener::bind(config.listen)
            .await
            .map_err(|e| StartError::Bind(config.listen, e))?;
        let local_addr = listener
            .local_addr()
            .map_err(|e| StartError::Bind(config.listen, e))?;

        let state = Arc::new(State {
            opaque,
            db: Mutex::new(db),
            setup: SetupCode::generate(Instant::now()),
            peers: Arc::new(Peers::default()),
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
            .with_state(state.clone());

        let (shutdown, stop) = watch::channel(false);
        let task = tokio::spawn(accept_loop(
            listener,
            acceptor,
            router,
            state.peers.clone(),
            stop,
        ));
        Ok(Server {
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
    Internal,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            ApiError::Bad(why) => (StatusCode::BAD_REQUEST, why),
            ApiError::Forbidden => (StatusCode::FORBIDDEN, "the setup code is not valid"),
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
    if req.device.name.trim().is_empty()
        || req.device.name.chars().count() > 64
        || verify_device(&req.identity_public, &req.device).is_err()
    {
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
            name: d.name.trim().to_string(),
            portable: d.portable,
            alg: d.alg,
            signing_public: d.signing_public,
            sealing_public: d.sealing_public,
            signature: d.signature,
        },
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
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
                        .serve_connection(TokioIo::new(tls), service);
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
