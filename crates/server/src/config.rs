//! Settings, from command-line flags with environment variables as fallback.

use crate::backup::{BackupConfig, TimeOfDay};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

/// The default listener: every interface, port 443.
pub const DEFAULT_LISTEN: &str = "0.0.0.0:443";
pub const DEFAULT_NAME: &str = "hab-bot server";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The folder holding the SQLite file.
    pub data_dir: PathBuf,
    /// The one HTTPS listener.
    pub listen: SocketAddr,
    /// The name apps show before joining. `None` keeps the stored name.
    pub name: Option<String>,
    /// Extra DNS names for a newly made certificate (`localhost` is always in).
    pub cert_names: Vec<String>,
    /// Debug logging, the only way IP addresses reach the logs.
    pub debug: bool,
    /// A trusted certificate chain (PEM) read from a file, with its key.
    /// Both or neither.
    pub tls_cert: Option<PathBuf>,
    pub tls_key: Option<PathBuf>,
    /// How often the trusted certificate's files are checked for changes.
    pub tls_poll: Duration,
    /// The nightly backup, off unless a folder is given.
    pub backup: Option<BackupConfig>,
}

/// Backups kept unless `--backup-keep` says otherwise.
pub const DEFAULT_BACKUP_KEEP: usize = 14;

impl Default for Config {
    fn default() -> Self {
        Config {
            data_dir: PathBuf::from("."),
            listen: DEFAULT_LISTEN.parse().expect("valid default"),
            name: None,
            cert_names: Vec::new(),
            debug: false,
            tls_cert: None,
            tls_key: None,
            tls_poll: Duration::from_secs(30),
            backup: None,
        }
    }
}

pub const HELP: &str = "\
hab-server [options]

Options (each also reads the environment variable shown):
  --data-dir <folder>   Folder for the SQLite file             HAB_SERVER_DATA_DIR  (default .)
  --listen <addr:port>  Address and port for HTTPS             HAB_SERVER_LISTEN    (default 0.0.0.0:443)
  --address <ip>        Change only the address                HAB_SERVER_ADDRESS
  --port <port>         Change only the port                   HAB_SERVER_PORT
  --name <name>         Server name shown to apps              HAB_SERVER_NAME
  --cert-name <dns>     Extra name for a new certificate       HAB_SERVER_CERT_NAMES (comma separated)
  --tls-cert <file>     Trusted certificate chain (PEM), reloaded on change   HAB_SERVER_TLS_CERT
  --tls-key <file>      Private key (PEM) for --tls-cert       HAB_SERVER_TLS_KEY
  --backup-dir <folder> Copy the database here every night     HAB_SERVER_BACKUP_DIR
  --backup-time <HH:MM> When the nightly backup runs, UTC      HAB_SERVER_BACKUP_TIME (default 03:00)
  --backup-keep <n>     How many backups to keep               HAB_SERVER_BACKUP_KEEP (default 14)
  --debug               Debug logging, which includes IP addresses   HAB_SERVER_DEBUG=1
  --help";

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Box<Config>),
    Help,
}

/// Parse `args` (without the program name) over `env`.
pub fn parse<A, E>(args: A, env: E) -> Result<Parsed, String>
where
    A: IntoIterator<Item = String>,
    E: Fn(&str) -> Option<String>,
{
    let mut config = Config::default();
    let mut listen = env("HAB_SERVER_LISTEN");
    let mut address = env("HAB_SERVER_ADDRESS");
    let mut port = env("HAB_SERVER_PORT");
    if let Some(v) = env("HAB_SERVER_DATA_DIR") {
        config.data_dir = v.into();
    }
    config.name = env("HAB_SERVER_NAME");
    if let Some(v) = env("HAB_SERVER_CERT_NAMES") {
        config.cert_names = v
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    config.debug = matches!(
        env("HAB_SERVER_DEBUG").as_deref(),
        Some("1" | "true" | "yes")
    );

    config.tls_cert = env("HAB_SERVER_TLS_CERT").map(PathBuf::from);
    config.tls_key = env("HAB_SERVER_TLS_KEY").map(PathBuf::from);
    let mut backup_dir = env("HAB_SERVER_BACKUP_DIR").map(PathBuf::from);
    let mut backup_time = env("HAB_SERVER_BACKUP_TIME");
    let mut backup_keep = env("HAB_SERVER_BACKUP_KEEP");

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let mut value = |flag: &str| args.next().ok_or(format!("{flag} needs a value"));
        match arg.as_str() {
            "--help" | "-h" => return Ok(Parsed::Help),
            "--debug" => config.debug = true,
            "--data-dir" => config.data_dir = value("--data-dir")?.into(),
            "--listen" => listen = Some(value("--listen")?),
            "--address" => address = Some(value("--address")?),
            "--port" => port = Some(value("--port")?),
            "--name" => config.name = Some(value("--name")?),
            "--cert-name" => config.cert_names.push(value("--cert-name")?),
            "--tls-cert" => config.tls_cert = Some(value("--tls-cert")?.into()),
            "--tls-key" => config.tls_key = Some(value("--tls-key")?.into()),
            "--backup-dir" => backup_dir = Some(value("--backup-dir")?.into()),
            "--backup-time" => backup_time = Some(value("--backup-time")?),
            "--backup-keep" => backup_keep = Some(value("--backup-keep")?),
            other => return Err(format!("unknown option `{other}`")),
        }
    }

    let mut addr: SocketAddr = listen
        .as_deref()
        .unwrap_or(DEFAULT_LISTEN)
        .parse()
        .map_err(|e| format!("bad listen address: {e}"))?;
    if let Some(a) = address {
        addr.set_ip(a.parse().map_err(|e| format!("bad address: {e}"))?);
    }
    if let Some(p) = port {
        addr.set_port(p.parse().map_err(|e| format!("bad port: {e}"))?);
    }
    config.listen = addr;

    if config.tls_cert.is_some() != config.tls_key.is_some() {
        return Err("--tls-cert and --tls-key go together".into());
    }
    match backup_dir {
        Some(dir) => {
            let at = match backup_time {
                Some(t) => TimeOfDay::parse(&t)?,
                None => TimeOfDay::DEFAULT,
            };
            let keep = match backup_keep {
                Some(k) => k
                    .parse::<usize>()
                    .ok()
                    .filter(|k| *k > 0)
                    .ok_or("--backup-keep must be a number above 0")?,
                None => DEFAULT_BACKUP_KEEP,
            };
            config.backup = Some(BackupConfig { dir, at, keep });
        }
        None if backup_time.is_some() || backup_keep.is_some() => {
            return Err("--backup-time and --backup-keep need --backup-dir".into());
        }
        None => {}
    }
    Ok(Parsed::Run(Box::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(args: &[&str], env: &[(&str, &str)]) -> Result<Parsed, String> {
        let env: Vec<(String, String)> = env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        parse(args.iter().map(|s| s.to_string()), move |k| {
            env.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone())
        })
    }

    #[test]
    fn defaults_to_port_443_on_every_address() {
        let Parsed::Run(c) = run(&[], &[]).unwrap() else {
            panic!()
        };
        assert_eq!(c.listen.port(), 443);
        assert_eq!(c.listen.to_string(), "0.0.0.0:443");
        assert!(!c.debug);
    }

    #[test]
    fn address_and_port_are_configurable() {
        let Parsed::Run(c) = run(&["--address", "10.1.2.3", "--port", "8443"], &[]).unwrap() else {
            panic!()
        };
        assert_eq!(c.listen.to_string(), "10.1.2.3:8443");
        let Parsed::Run(c) = run(&["--listen", "[::1]:9000"], &[]).unwrap() else {
            panic!()
        };
        assert_eq!(c.listen.to_string(), "[::1]:9000");
    }

    #[test]
    fn flags_beat_environment() {
        let Parsed::Run(c) = run(
            &["--port", "1"],
            &[("HAB_SERVER_PORT", "2"), ("HAB_SERVER_DEBUG", "1")],
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(c.listen.port(), 1);
        assert!(c.debug);
    }

    #[test]
    fn trusted_certificate_needs_both_files() {
        assert!(run(&["--tls-cert", "c.pem"], &[]).is_err());
        assert!(run(&[], &[("HAB_SERVER_TLS_KEY", "k.pem")]).is_err());
        let Parsed::Run(c) = run(
            &["--tls-key", "k.pem"],
            &[("HAB_SERVER_TLS_CERT", "/certs/c.pem")],
        )
        .unwrap() else {
            panic!()
        };
        assert_eq!(c.tls_cert.unwrap().to_str(), Some("/certs/c.pem"));
        assert_eq!(c.tls_key.unwrap().to_str(), Some("k.pem"));
    }

    #[test]
    fn backups_are_off_until_a_folder_is_given() {
        let Parsed::Run(c) = run(&[], &[]).unwrap() else {
            panic!()
        };
        assert!(c.backup.is_none());
        let Parsed::Run(c) = run(&["--backup-dir", "/b"], &[]).unwrap() else {
            panic!()
        };
        let b = c.backup.unwrap();
        assert_eq!(b.at, TimeOfDay::DEFAULT);
        assert_eq!(b.keep, DEFAULT_BACKUP_KEEP);
        let Parsed::Run(c) = run(
            &["--backup-time", "04:15"],
            &[
                ("HAB_SERVER_BACKUP_DIR", "/b"),
                ("HAB_SERVER_BACKUP_KEEP", "3"),
            ],
        )
        .unwrap() else {
            panic!()
        };
        let b = c.backup.unwrap();
        assert_eq!((b.at.hour, b.at.minute, b.keep), (4, 15, 3));
        assert!(run(&["--backup-time", "04:15"], &[]).is_err());
        assert!(run(&["--backup-dir", "/b", "--backup-time", "25:00"], &[]).is_err());
        assert!(run(&["--backup-dir", "/b", "--backup-keep", "0"], &[]).is_err());
    }

    #[test]
    fn rejects_unknown_options_and_bad_values() {
        assert!(run(&["--nope"], &[]).is_err());
        assert!(run(&["--port", "x"], &[]).is_err());
        assert!(run(&["--port"], &[]).is_err());
    }
}
