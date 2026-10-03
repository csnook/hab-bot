# Sync, encryption and the server

> **First release:**
> - **In:**
>   - the encrypted event log with its merge rules
>   - list keys, the identity key and the cryptography below
>   - SQLite storage, and keys in the operating system's key store
>   - one WebSocket per device, plus HTTPS for bulk downloads
>   - the pinned self-signed certificate, plus a trusted certificate read from files
>   - app format versions
>   - the server as a single binary with nightly backups
>   - one listener on 443 with a configurable address, the setup code's expiry, logs without IP addresses, and the password backoff
> - **Later:** push (starting with the Doze prototype), retention limits, the separate webhook port and address, invite and webhook rate limits, and padding.

## The event log

[ADR 0005](../adr/0005-encrypted-event-log-numbered-by-server.md) records this choice. Sync is an encrypted event log per reminder list, numbered by the server.

- **Every change is an event:** made on a device, signed by it, and encrypted with its list's key.
- **The server numbers events without reading them.** It adds each one to its list's stream with the next number, and sees only the stream, the device, the size and the time received.
- **Devices build the current state** by applying a stream's events in order. Their own unsent events are applied on top, marked "not sent yet" until the server numbers them.
- **The first claim in a stream wins.** That's the "first claim the server receives".
- **The stream is the history.** Signed events make attribution provable.
- **A standalone device numbers its own streams,** and joining a server uploads them.
- **CRDT libraries were rejected:** the data is small and structured with no text to merge, the server already orders claims and key changes, and the encryption layers built for CRDTs haven't reached 1.0.

### Which edit wins

- **The setting changed last wins,** judged per setting of a reminder by a hybrid logical clock. The server rejects clocks set far ahead of its own.
- **Nothing is lost:** both changes stay in the history, so the losing one can be restored.
- **Changes to different settings** both survive.
- **Notes don't merge.** Of two concurrent edits to a note, the later wins, and the earlier stays in the history.
- **Occurrences follow their own rules** (see [Acting on occurrences](reminders.md#acting-on-occurrences)), such as a completion beating a skip. Claims follow the server's order.

### What syncs where

- **One stream per reminder list,** personal lists included. Each holds:
  - the list's reminders
  - the sources that belong to the list
  - occurrences and claims
  - everything done to them. On shared lists that includes snoozes and acknowledgements, which the others can see.
- **A reminder shared on its own** with specific users gets its own key and stream, like a one-reminder list.
- **Your personal list's stream also holds your settings,** and all of your devices always hold it:
  - custom priorities
  - quiet hours and snooze-alls
  - your cap on other users' reminders
  - your overrides on shared reminders
  - sources that belong to you
  - your devices' names, and which are portable
- **Some things stay on one device:**
  - its loudest style and "quiet this device"
  - which lists it holds
  - which calendars it lays over the timeline
  - calendar events
- **The server reads these unencrypted, outside any stream:**
  - accounts, devices and their public keys
  - groups and members
  - access and invitations
  - sealed list keys

## Keys and signatures

- **List keys** ([ADR 0002](../adr/0002-per-list-keys-sealed-to-devices.md)):
  - **Sealing:** each list has one symmetric key, sealed separately to the public key of every device with access. The server stores the sealed copies and orders changes to them.
  - **Adding a user:** a device of someone with manage access seals the current key to that user's devices.
  - **Removing a user or a device** rotates the key and seals the new one to everyone who remains. The server takes the device off the account and stores the new copies in one step, and refuses it unless every remaining device ends up with every version. The personal list's first key is derived from the personal key, so its new keys are random ([ADR 0008](../adr/0008-personal-list-key-rotates-to-a-random-key-on-device-removal.md)).
  - **History:** the new key is sealed together with the old keys, so anyone with current access can read the whole history. There's no forward secrecy, and a removed user keeps what they've already seen.
  - **Group key protocols were rejected:** MLS, Keyhive/BeeKEM, p2panda and Megolm all solve leaderless groups, which a single ordering server doesn't need.
- **The identity key** signs each of a user's devices, and devices seal keys only to devices signed that way ([ADR 0004](../adr/0004-password-unlocked-accounts-and-device-vouching.md)).
- **Signed access and membership:**
  - **Who signs:** every access change is signed by the device of the user with manage access who made it. Every group membership change is signed by a group admin's device.
  - **Where they live:** they're stored unencrypted, since the server enforces access, and numbered in the list's stream.
  - **Devices check them:** before sealing a key or accepting an event, a device checks those signatures, and that the event's author had enough access at that point in the stream. The server can't forge them.

## Cryptography

| Use | Algorithm | Rust crate |
|---|---|---|
| Events | XChaCha20-Poly1305 with the list key | `chacha20poly1305` |
| Signatures (devices signing events, the identity signing devices) | Ed25519 | `ed25519-dalek` |
| Sealing list keys to devices | HPKE (RFC 9180) with X25519 | `hpke` |
| Password | OPAQUE, with Argon2id as its hardening step (64 MiB, 3 passes, stored with the bundle). OPAQUE's export key encrypts the key bundle. | `opaque-ke` |

- **Algorithm identifiers:** every encrypted or signed item carries an identifier of the algorithms used, so they can change later.
- **Changing algorithms:**
  - New events and keys switch to a new set once every device holding the list supports it.
  - Old events keep their original encryption, and the app keeps the code to read them.
  - If an algorithm is ever broken, a device with manage access re-encrypts the list's history.
  - The password bundle is upgraded at the next password sign-in.
- **Unencrypted header of each event:** the stream, its number, the device, the size and the time received. The time received doubles as the server's timestamp in the history. Events that should start a new alert are also marked urgent here (see Push).
- **Crate versions** get checked when building.

## Storage

- **Devices use SQLite** (`rusqlite`), holding decrypted events and the state built from them. Protection at rest is the operating system's job: the Android app sandbox and storage encryption, or the Linux home directory. The web client uses IndexedDB (see [Web client](web-client.md)).
- **Keys go in the operating system's key store:** the Android Keystore, or the Secret Service on Linux (KWallet on Plasma) through `oo7`. With no Secret Service running, keys go in a file only the user can read.
- **The server uses SQLite too.** A public product could move to Postgres later.

## Connections

- **One WebSocket per device** while it can reach the server. Desktops keep it always. Phones keep it while ringing or syncing.
- **HTTPS for bulk downloads,** such as a new device's first sync or older history.
- **Never assume the server is home-only.** The app just tries, so a VPN makes everything work away from home with no changes to the app.
- **Certificates:**
  - The server makes a **self-signed certificate** on first start. Invites and sign-in codes carry its name and fingerprint, and devices pin it.
  - A new device gets the fingerprint from a QR code or link that any signed-in device can show.
  - The server can also read a **trusted certificate and key from files**, whatever issued them, and reloads them when they change. Tailscale can issue one for a machine's `ts.net` name, and a DNS-01 tool such as certbot can issue one for a domain of your own.
  - It serves the trusted certificate to connections using its name, and the self-signed one otherwise. The apps accept both.
  - The web client needs the trusted one ([ADR 0007](../adr/0007-web-client-from-a-static-host-with-a-browser-trusted-server-certificate.md)).

## Push

[ADR 0006](../adr/0006-push-through-unifiedpush.md) records this choice.

- **Phones are woken by content-free pushes,** sent as standard Web Push (RFC 8030, encrypted per RFC 8291) and signed with a VAPID key each server generates for itself. A push carries only "wake up and sync".
- **Phones with Google Play Services** use UnifiedPush's embedded FCM distributor, a library inside the app, which delivers through Google's servers. There's no Firebase project, no shared secret, and no Firebase library.
- **Phones without Google** use any UnifiedPush distributor the user installs, such as ntfy. The server speaks the same protocol.
- **With no distributor,** the app says so once, and relies on periodic sync, the check before every firing, and opening the app.
- **An early prototype must confirm** that an urgent push through the embedded FCM distributor wakes a phone in Doze. If it doesn't, or if Google closes the route, the fallback is FCM through the Firebase library. That needs a Firebase project and its service-account key on the server.
- **Urgency:**
  - **High** only for things that should start a new alert on that phone: an occurrence another device fired that this phone should alert for, and a webhook event for a list it holds. Android demotes apps whose urgent pushes don't lead to a notification.
  - **Normal** for everything else, which waits for Doze's maintenance windows.
- **At most one waiting push per device.**
- **Our own Kotlin push service** calls into Rust to sync and post notifications without opening the app. No Tauri plugin does this.
- **Desktops** keep their own connection and don't use push.
- **The web client's pushes** have their own rules (see [Web client](web-client.md#alerts)).

## Retention and deletion

- **Events are kept until a retention limit removes them.**
- **Snapshots:** a device can post an encrypted, signed snapshot of a list as of event N. A new device starts from the latest snapshot it trusts (one from its own user's devices, or from a user with manage access). It fetches older events only when a history view needs them.
- **Retention limits:** the server's limit caps everything. Beyond that:

  | List | Kept on the server | Kept on devices |
  |---|---|---|
  | Your personal list | Your limit | Your limit |
  | Shared with a group | The group's limit (the shortest, if shared with several groups) | The group's limit, whatever members' own limits are |
  | Shared only with individual users, including single reminders | The server's limit | Each user's own limit. Devices ask the server for history only back to that limit, so nobody learns anyone else's limit. |

  - **Who sets them:** users set their own, group admins set a group's (starting as its creator's), and server admins set the server's, which is shown on the invite screen.
  - **The list's settings** show the limit in effect, such as "1 year, from Household".
  - **Setting your own limit** brings a warning that history you share may be kept longer on the server and on other people's devices.
  - **Choices:** no limit (the default), 30 days, 90 days, 1 year, 2 years, or a custom length.
- **What a limit removes:** history only (occurrences, actions, and past values of settings). Current reminders, lists, sources and settings survive, and so does each countdown's last closing. A snapshot at the cut-off keeps them. A finished one-off goes once its whole history is past the limit.
- **Shortening a limit** is announced to everyone it affects, and takes effect after 7 days. The server deletes events past the limit once a snapshot covers them, and devices delete their copies. A standalone device joining a server gets the same 7-day notice before older history goes.
- **Deleting:**
  - **A reminder** keeps its history, marked deleted, unless you choose "delete with its history".
  - **A list** needs manage access, and removes its stream.
  - Copies already on removed users' devices can't be recalled.

## App versions

- **Every event says which format version it uses.**
- **A device that meets a newer format** keeps the event without applying it, shows "update the app to see recent changes to this list", and applies it after updating.
- **Newer versions read every older format.** The server accepts any version.

## What the server learns

- **It reads** accounts, devices, groups, members, access and invitations, ordering data for claims (an opaque occurrence ID, the user and the time), and profiles (display names and avatars).
- **It sees but can't read** each event's list, device, size and arrival time.
- **It never sees** reminder content, history, source settings or calendar events. Readable lists and sources are deferred until a server feature needs them.
- **IP addresses** are held in memory only, for rate limits. They're logged only if the server admin turns on debug logging.
- **Push services** see when pushes are sent, but nothing inside them. Our pushes are all the same size.
- **Padding events** to fixed sizes is deferred as a low priority.
- **Users shouldn't assume their traffic goes unlogged:** anything between them and the server can log it. The app says so in "What your server can see" (see [People, sharing and accounts](sharing-and-accounts.md#joining-a-server)).

## Running the server

- **How the server is reached is a deployment choice,** made by hand: the home network, a VPN, or the internet. Automating VPN and DNS setup comes later, possibly through Tailscale with one API key. The server has to be safe however it's deployed.
- **Packaging:** a single Rust binary run as a systemd service, or a container image if the server's host runs containers.
- **Backups:**
  - Each night the server writes a copy of its SQLite database to a folder you choose, using SQLite's online backup, so it keeps running. Restoring means putting the file back.
  - Everything about reminders in it is encrypted, so the backup can go anywhere.
  - Devices also hold full copies of their lists.
- **Listening:**
  - **Default:** sync and webhooks share port 443, since exposing several ports is cumbersome in some deployments.
  - **Separately:** each can be given its own port and its own IP address. For example, sync can listen only on a VPN address while webhooks listen publicly, or the webhook port can be exposed alone through Tailscale Funnel or a single port forward.
- **The setup code** is random and long, and is shown in the server's console. It stops working 24 hours after the server starts or once the first account exists, and restarting makes a new one.
- **Rate limits** (defaults the server admin can change):

  | What | Limit |
  |---|---|
  | Password checks | After 5 failures for an account, each attempt waits twice as long as the last, from 1 minute up to 1 hour, and the user's devices are notified. At most 20 attempts a minute per IP address. |
  | Invites | At most 10 attempts a minute per IP address |
  | Webhooks | At most 60 a minute per URL. Bodies of at most 64 KiB. The excess is refused with "too many requests". |
  | Connections | Caps on WebSockets and request sizes, per device and per IP address |

- **Certificate names** in public certificate logs are accepted. A wildcard certificate wouldn't help, since the domain under it can identify you anyway, and Tailscale's `ts.net` names are pseudonymous enough.
- **Builds:** see [First release and 1.0](first-release.md#how-it-ships).

## Decided in

- [Local-first sync options in Rust][6]
- [Which data the server may read][14]
- [One person, several devices][23]
- [Accounts, groups and permissions][24]
- [Sync design][25]
- [Security hardening before the server goes on the internet][33]
- **ADRs:** [0002](../adr/0002-per-list-keys-sealed-to-devices.md), [0004](../adr/0004-password-unlocked-accounts-and-device-vouching.md), [0005](../adr/0005-encrypted-event-log-numbered-by-server.md), [0006](../adr/0006-push-through-unifiedpush.md), [0007](../adr/0007-web-client-from-a-static-host-with-a-browser-trusted-server-certificate.md)
- **Research:** [local-first sync in Rust](../research/local-first-sync-rust.md), [Android push without Google](../research/android-push-without-google.md)

[6]: https://github.com/csnook/hab-bot/issues/6
[14]: https://github.com/csnook/hab-bot/issues/14
[23]: https://github.com/csnook/hab-bot/issues/23
[24]: https://github.com/csnook/hab-bot/issues/24
[25]: https://github.com/csnook/hab-bot/issues/25
[33]: https://github.com/csnook/hab-bot/issues/33
