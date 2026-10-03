use hab_server::config::{self, Parsed};
use hab_server::Server;
use std::process::exit;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    let config = match config::parse(std::env::args().skip(1), |k| std::env::var(k).ok()) {
        Ok(Parsed::Run(c)) => *c,
        Ok(Parsed::Help) => {
            println!("{}", config::HELP);
            return;
        }
        Err(e) => {
            eprintln!("{e}\n\n{}", config::HELP);
            exit(2);
        }
    };

    // Debug logging is the only way IP addresses reach the logs, so it is a
    // deliberate switch and not something RUST_LOG can flip by accident.
    let filter = if config.debug {
        "warn,hab_server=debug"
    } else {
        "warn,hab_server=info"
    };
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::new(filter))
        .init();

    let server = match Server::start(&config).await {
        Ok(s) => s,
        Err(e) => {
            eprintln!("cannot start: {e}");
            exit(1);
        }
    };

    if server.cert_created() {
        tracing::info!("made a new self-signed certificate");
    }
    tracing::info!("listening on https://{}", server.local_addr());
    if config.debug {
        tracing::warn!("debug logging is on: IP addresses will appear in the logs");
    }
    // The setup code and fingerprint go to the console only, not the log.
    println!("Server name:         {}", server.name());
    println!("Listening on:        https://{}", server.local_addr());
    println!("Certificate (SHA-256): {}", server.fingerprint());
    if let Some(fp) = server.trusted_fingerprint() {
        println!("Trusted certificate (SHA-256): {fp}");
        println!("It is served to connections that use one of its names; the one above otherwise.");
    }
    if let Some(b) = &config.backup {
        println!(
            "Nightly backup:      {} at {:02}:{:02} UTC, newest {} kept",
            b.dir.display(),
            b.at.hour,
            b.at.minute,
            b.keep
        );
    }
    println!("Setup code:          {}", server.setup_code());
    println!("The setup code creates the first account. It stops working in 24 hours,");
    println!("or once the first account exists. Restarting makes a new one.");

    server.run_until_signal().await;
}
