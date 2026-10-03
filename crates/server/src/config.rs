//! Settings, from command-line flags with environment variables as fallback.

use std::net::SocketAddr;
use std::path::PathBuf;

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
}

impl Default for Config {
    fn default() -> Self {
        Config {
            data_dir: PathBuf::from("."),
            listen: DEFAULT_LISTEN.parse().expect("valid default"),
            name: None,
            cert_names: Vec::new(),
            debug: false,
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
  --debug               Debug logging, which includes IP addresses   HAB_SERVER_DEBUG=1
  --help";

#[derive(Debug, PartialEq, Eq)]
pub enum Parsed {
    Run(Config),
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
    Ok(Parsed::Run(config))
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
    fn rejects_unknown_options_and_bad_values() {
        assert!(run(&["--nope"], &[]).is_err());
        assert!(run(&["--port", "x"], &[]).is_err());
        assert!(run(&["--port"], &[]).is_err());
    }
}
