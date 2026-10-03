# Running the server

The sync server is one Rust binary, `hab-server`, with its state in one SQLite file. Run it as a systemd service or as a container. How it is reached (home network, VPN, internet) is up to you. See [Sync and server](spec/sync-and-server.md#running-the-server) for the design.

Every option is a flag and an environment variable; `hab-server --help` lists them.

| Flag | Variable | Default |
|---|---|---|
| `--data-dir` | `HAB_SERVER_DATA_DIR` | `.` |
| `--listen`, or `--address` and `--port` | `HAB_SERVER_LISTEN`, `HAB_SERVER_ADDRESS`, `HAB_SERVER_PORT` | `0.0.0.0:443` |
| `--name` | `HAB_SERVER_NAME` | `hab-bot server` |
| `--cert-name` (repeat) | `HAB_SERVER_CERT_NAMES` (comma separated) | `localhost` only |
| `--tls-cert`, `--tls-key` | `HAB_SERVER_TLS_CERT`, `HAB_SERVER_TLS_KEY` | none |
| `--backup-dir` | `HAB_SERVER_BACKUP_DIR` | none (no backups) |
| `--backup-time` | `HAB_SERVER_BACKUP_TIME` | `03:00` (UTC) |
| `--backup-keep` | `HAB_SERVER_BACKUP_KEEP` | `14` |
| `--debug` | `HAB_SERVER_DEBUG=1` | off; the only way IP addresses reach the logs |

## Installing with systemd

1. Build the binary and put it in place:

   ```sh
   cargo build --release -p hab-server
   sudo install -m 755 target/release/hab-server /usr/local/bin/hab-server
   ```

2. Make the service user and install the unit:

   ```sh
   sudo install -m 644 packaging/hab-server.sysusers /usr/lib/sysusers.d/hab-server.conf
   sudo systemd-sysusers
   sudo install -m 644 packaging/hab-server.service /etc/systemd/system/hab-server.service
   ```

3. Choose settings in `/etc/hab-server/hab-server.env`, one `NAME=value` per line, for example:

   ```
   HAB_SERVER_NAME=Home
   HAB_SERVER_BACKUP_DIR=/var/backups/hab-server
   ```

   and make the backup folder: `sudo install -d -o hab-server -g hab-server -m 700 /var/backups/hab-server`.

4. Start it: `sudo systemctl enable --now hab-server`.

The unit runs as the unprivileged `hab-server` user, keeps the database in `/var/lib/hab-server` and may bind port 443 without being root. Only `/var/lib/hab-server` and `/var/backups/hab-server` are writable to it.

## Installing as a container

The image is built from `Containerfile` at the repository root, with Podman or Docker:

```sh
podman build -t hab-server -f Containerfile .
podman run -d --name hab-server --restart unless-stopped \
  -p 443:8443 -v hab-data:/data -v /srv/hab-backups:/backups \
  hab-server
```

The container runs as an unprivileged user and listens on 8443, so publish it as 443 with `-p 443:8443`. `/data` holds the database and `/backups` the nightly backups (already on in the image). Settings are the same `HAB_SERVER_*` variables, passed with `-e`. Read the setup code with `podman logs hab-server`.

## The setup code

On start the server prints a **setup code** to its console, never to its log file: with systemd the console is the journal (`journalctl -u hab-server | grep "Setup code"`), in a container `podman logs hab-server`. The code creates the first account, which becomes the server admin. It stops working 24 hours after the server starts or once the first account exists, and restarting makes a new one. The same output shows the server's certificate fingerprint, which invites carry.

## Backups

Set `--backup-dir` (or `HAB_SERVER_BACKUP_DIR`) and each night, at `--backup-time` UTC (03:00 unless changed), the server copies its database there using SQLite's online backup. It keeps serving while the copy runs. The copy is written as `hab-server-YYYY-MM-DD.db`, replaced if one already exists for that day, and only the newest `--backup-keep` such files (14 unless changed) are kept. A failed backup is logged as an error and leaves no half-written file. The folder is created at start, and an unusable folder stops the server from starting.

Everything about reminders in the database is encrypted, so a backup can be kept anywhere. It does hold the server's certificate private key and OPAQUE keys, so keep it as private as the database itself (the files are readable by their owner only). Devices also hold full copies of their lists.

**Restoring** is putting the file back: stop the server, copy the chosen backup over `hab-server.db` in the data folder (with its owner and mode `600`), and start the server.

```sh
sudo systemctl stop hab-server
sudo install -o hab-server -g hab-server -m 600 \
  /var/backups/hab-server/hab-server-2026-10-02.db /var/lib/hab-server/hab-server.db
sudo systemctl start hab-server
```

## Certificates

The server always has a **self-signed certificate**, made on first start and kept in the database. Apps pin its fingerprint from the invite. Add names it should cover with `--cert-name` before first start.

It can also serve a **trusted certificate**, from any issuer, read from files. Give it the full chain and the key, both PEM:

```
HAB_SERVER_TLS_CERT=/etc/hab-server/tls/cert.pem
HAB_SERVER_TLS_KEY=/etc/hab-server/tls/key.pem
```

- A connection that asks for one of the certificate's names gets it. Every other connection, such as one by IP address or another name, gets the self-signed one. The apps accept both: the pinned fingerprint, or a certificate that chains to a public authority and is valid for the name they used. Browsers need the trusted one ([ADR 0007](adr/0007-web-client-from-a-static-host-with-a-browser-trusted-server-certificate.md)).
- The files are checked every 30 seconds and reloaded when either changes, so a renewal needs no restart. Replace the key first, then the certificate. A file that cannot be used (half written, or a key that doesn't match) is logged and the previous certificate stays in use.
- At start the files must be usable, or the server doesn't start. The service user must be able to read them.

### A Tailscale certificate (`ts.net`)

With HTTPS enabled for your tailnet, on the server's machine:

```sh
sudo install -d -o hab-server -g hab-server -m 700 /etc/hab-server/tls
cd /etc/hab-server/tls
sudo tailscale cert --cert-file cert.pem --key-file key.pem myserver.tailnet-name.ts.net
sudo chown hab-server:hab-server cert.pem key.pem
```

Then set `HAB_SERVER_TLS_CERT` and `HAB_SERVER_TLS_KEY` as above and restart once. Tailscale certificates last 90 days. Run the `tailscale cert` command again from a weekly timer or cron job (and the `chown`), and the server picks up the new files by itself. Reach the server as `myserver.tailnet-name.ts.net`.

### A certificate for your own domain

Any ACME client that can use the DNS-01 check works without exposing the server to the internet, for example certbot with your DNS provider's plugin. Point the server at its output, such as `/etc/letsencrypt/live/home.example.org/fullchain.pem` and `privkey.pem`, and make them readable by the service user (certbot's `live/` links are followed). Renewals are then picked up automatically. If your router drops DNS answers that point a public name at a private address, add a local override for the name.
