//! The HTTPS listener and the routes behind it.

use crate::cert::{self, CertError};
use crate::config::{Config, DEFAULT_NAME};
use crate::db::{Db, DbError};
use crate::peers::Peers;
use crate::setup::SetupCode;
use axum::{routing::get, Json, Router};
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
            db: Mutex::new(db),
            setup: SetupCode::generate(Instant::now()),
            peers: Arc::new(Peers::default()),
        });
        let info = Arc::new(Info {
            name: name.clone(),
            version: env!("CARGO_PKG_VERSION"),
        });
        let router = Router::new().route(
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
        );

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

    /// Whether `candidate` is the setup code and it still works. Joining (#56)
    /// calls this before it creates the first account.
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
        server.with_db(|db| db.add_account("chris", 0).unwrap());
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
