# Local-first sync options in Rust

Research for [csnook/hab-bot#6](https://github.com/csnook/hab-bot/issues/6): what are the realistic ways, in Rust, to sync this app's data between devices and a self-hosted server?

- **Read on:** 2026-09-27. Versions and release dates are given per source (from crates.io and the npm registry on that day).
- **Requirements weighed:** (R1) every device works offline and merges offline edits; (R2) some devices hold only some reminders; (R3) some data must be unreadable to the server (end-to-end encrypted, E2EE) and some may be readable, configurable per server; (R4) the history is an append-only record of occurrences, completions and skips, each attributed to a person.
- **Project constraints noted:** one server per account; a household shares reminders across accounts on one server; sharing across servers is out of scope but must not be ruled out; the server sits on a home network while prototyping; Rust and TypeScript only.
- **Terminology:** "reminder", "occurrence", "complete" and "skip" are used as in `CONTEXT.md`. "Document" means a CRDT library's unit of sync (an Automerge/Loro/Yjs document), not a user-facing thing. "Relay" means a server that stores and forwards data without being able to read it.
- **Unverified claims:** the network proxy blocked most project websites (full list in [Access notes](#access-notes)). Where a website's source lives in a GitHub repository, it was read there instead. Claims resting only on a search-engine excerpt are marked **[unverified: search excerpt]**.
- This file surfaces facts and trade-offs. It does not choose an option.

## Summary

- **No single option meets all four requirements with production maturity today.** The mature pieces (Automerge, Loro, Yrs/Yjs) solve offline merging (R1). E2EE (R3) and person-level attribution (R4) come from separate layers that are alpha or pre-1.0, or have to be built.
- **CRDT libraries sync whole documents.** Automerge, Loro and Yrs each sync one document at a time, so syncing only some reminders (R2) comes down to where the document boundaries fall (for example, one document per reminder or per household) and which documents a device asks for. None of the three syncs part of a document. All three keep an operation history, but Yjs/Yrs garbage-collects deleted content by default, and Loro can drop old history on purpose (shallow snapshots).
- **E2EE for CRDTs is a sync-layer feature, not a library feature.** Automerge has Keyhive (group keys, access control, encryption) with the Subduction sync protocol, whose servers relay ciphertext they cannot read. Both are Rust, marked pre-alpha or "do not use in production", and unaudited. The integrated package (`@automerge/automerge-repo-keyhive`) is TypeScript alpha. Loro's sync protocol has an E2EE extension (`%ELO`, AES-GCM, TypeScript and Rust) that leaves key distribution to the app. No first-party E2EE sync server was found for Yjs. `secsync` (TypeScript beta, last release June 2024) is a third-party design for relaying encrypted Yjs or Automerge updates.
- **SQLite-based sync is weak on maintenance and on E2EE.** Upstream cr-sqlite has not published a release since January 2024. Fly.io maintains a fork (0.17.0) for its Corrosion system. cr-sqlite keeps no history, merges last-writer-wins per column, and needs a party that can read values in order to merge. The SQLite session extension produces changesets but has no merge semantics, only conflict callbacks. PowerSync and Electric sync from a central Postgres (or similar) database, so the server must read the data. Electric's server is Elixir. PowerSync's service is under the FSL licence, and its Rust and Tauri SDKs are alpha. `sqlite-sync` uses a modified Elastic licence that forbids replacing its network layer. Ditto is proprietary.
- **A plain event log fits R4 directly and makes E2EE simple, but merging becomes our job.** Occurrences, completions and skips are immutable facts, so each can be a signed, encrypted event that the server stores as an opaque blob, partitioned by reminder or household for partial sync. The app would then need its own merge rules for data that changes, such as a reminder's settings. A CRDT can fill that role, and hybrids are possible. Rust building blocks: p2panda (signed per-author append-only logs plus group encryption, pre-1.0), iroh-docs (signed key-value entries, range-based set reconciliation), and the SQLite session extension (changesets as events).
- **Encrypting data shared across a household needs group key agreement.** Options in Rust: Keyhive's BeeKEM (pre-alpha), `p2panda-encryption` (pre-1.0), MLS through OpenMLS or `mls-rs` (MLS needs someone to order its commits, and a single home server could do that), and Megolm through `vodozemac` (Matrix's scheme).

## Comparison table

"Partial sync" means whether a device can hold only some reminders. "E2EE" means whether the server can relay data it cannot read. Versions and dates are the latest stable releases on 2026-09-27 unless stated.

| Option | Kind | Latest release | Licence | Rust | TypeScript | Partial sync | E2EE compatibility | History and attribution |
|---|---|---|---|---|---|---|---|---|
| **Automerge** (+ automerge-repo / samod) | CRDT | `automerge` 0.12.0 (2026-09-16); JS 3.5.0 (2026-09-16); `samod` 0.14.0 (2026-09-17) | MIT | Core is Rust; README calls the Rust API "low level and not well documented"; `samod` (Rust automerge-repo) self-described as experimental | First-class (`@automerge/automerge`, `@automerge/automerge-repo`) | Per document: sync runs one document at a time, and a share policy chooses which documents are announced | None built in; see Keyhive + Subduction | Keeps full history. 0.12.0 has an `Author` type (free-form bytes, mapped to one or more actor IDs), which is not signed |
| **Automerge + Keyhive + Subduction** | CRDT + access control + E2EE sync | `keyhive_core` 0.6.0 (2026-09-25); `subduction_core` 0.18.2 (2026-09-13); ARK npm `0.6.0-alpha.1` (2026-09-25) | Keyhive Apache-2.0; Subduction MIT OR Apache-2.0 | Core crates in Rust; a Rust server CLI exists; `samod` 0.14.0 has no Keyhive or Subduction dependency | ARK (TypeScript) is the integrated path, alpha | Per document; per-document membership (relay/read/edit/admin) | Designed for it: the server gets `relay` access and stores ciphertext. Protected and unprotected documents can coexist. Relays see document IDs, membership, sizes and timing | Membership operations are signed. Content chunks are not signed by Keyhive; content authorship is left to the CRDT layer. Forward secrecy is a non-goal |
| **Loro** (+ loro-protocol) | CRDT | `loro` 1.16.2 (2026-09-21); npm `loro-crdt` 1.16.3; `loro-protocol` crate 0.3.0 (2026-06-12) | MIT | Rust core; `loro-websocket-server` / `-client` crates at 0.1.0 (2025-11-30) | First-class (WASM) | Per document ("room") | `%ELO` extension: AES-GCM records; the server indexes plaintext headers (peer ID, counter spans, key ID); key agreement is out of scope | Keeps full history unless shallow snapshots are used. Peer IDs are u64; commit messages and timestamps are optional. No signing |
| **Yrs / Yjs** | CRDT | `yrs` 0.28.0 (2026-09-17); `yjs` 13.6.33 (2026-09-23; v14 in beta) | MIT | Rust port that keeps binary compatibility with Yjs; Rust servers (`y-sweet` 0.9.1, `yrs-warp` 0.9.0) | Yjs is the reference implementation | Per document; Yjs subdocuments can be loaded lazily | Nothing first-party for servers. `y-webrtc` encrypts signalling only. Third-party `secsync` (beta) | Deleted content is garbage-collected by default. Client IDs are per replica. No signing |
| **cr-sqlite** (vlcn-io upstream) | SQLite CRDT extension | npm `@vlcn.io/crsqlite` 0.16.3 (2024-01-17); no later release found | Apache-2.0 | Core partly Rust, loaded as an SQLite extension; not on crates.io | `@vlcn.io/crsqlite-wasm` 0.16.0 (2023-12-16) | Changes are rows in the `crsql_changes` virtual table and can be filtered with SQL | The server can only relay: merging compares versions and then values, which needs plaintext | No history (last write wins per column). `site_id` per database |
| **cr-sqlite** (Fly.io fork, used by Corrosion) | SQLite CRDT extension | 0.17.0 per README | Apache-2.0 (Corrosion) | As upstream | Not investigated | As upstream; the app does its own gap and sequence bookkeeping | As upstream | Adds a per-transaction timestamp column; still no history |
| **SQLite session extension** | SQLite changesets | Part of SQLite; `rusqlite` 0.40.2 has a `session` feature | Public domain (SQLite) | Through `rusqlite` | Not assessed | Per table (can attach specific tables) | Changesets are blobs that can be encrypted, but applying them needs plaintext | No merge semantics: applying a changeset raises conflicts for the app to resolve. No author field |
| **PowerSync** | Server-authoritative Postgres/MongoDB/MySQL/SQL Server → SQLite | Rust crate `powersync` 0.0.7 (2026-08-03), alpha; `@powersync/web` 2.4.1 (2026-09-23) | Service FSL-1.1-ALv2; SDKs Apache-2.0 | Rust SDK and Tauri SDK are alpha | Mature JS SDKs | Yes (Sync Streams) | Only by encrypting columns in the app; the docs describe that approach | Writes go through your own backend API; history is up to you |
| **Electric** | Postgres read-path sync | `@electric-sql/client` 1.5.28 (2026-09-09) | Apache-2.0 | No Rust client found; the server is Elixir | Yes | Yes (Shapes) | Not designed for it (shapes filter on Postgres columns) | Postgres is the source of truth |
| **Ditto** | Proprietary peer-to-peer database | `dittolive-ditto` 5.1.0 (2026-08-19) | Proprietary "Ditto Binary License" (no reverse engineering) | Rust SDK | Yes | Not verified | Not verified | Not verified |
| **Plain event log** (built in-house) | Design pattern | — | — | Anything | Anything | Yes, by partitioning the log (by reminder, household, …) | Easiest case: events are opaque to the server | The natural fit for immutable, signed, attributed events. Merge rules for data that changes must be written |
| **p2panda** | Append-only log toolkit | 0.7.1 (2026-08-21) | MIT OR Apache-2.0 | Native | Only experimental FFI (Node.js through UniFFI) | Yes (topics, logs) | `p2panda-encryption`: group data and message encryption | Signed per-author append-only logs |
| **iroh-docs** | Signed key-value replicas | 0.101.0 (2026-06-15) | MIT/Apache-2.0 | Native | Not assessed | Per namespace | No encryption layer described | Entries signed by namespace and author keys |

## Access notes

| Source | How it was read | Version / date |
|---|---|---|
| Crate metadata (versions, dates, licences, downloads) | crates.io API (`crates.io/api/v1/crates/<name>`) | 2026-09-27 |
| Crate source and READMEs (`automerge` 0.12.0, `yrs` 0.28.0, `loro` 1.16.2, `samod` 0.14.0, `p2panda-*` 0.7.1, `willow25` 0.7.9, `dittolive-ditto` 5.1.0) | Crate tarballs from static.crates.io (docs.rs was blocked); cited below as docs.rs source URLs for the same version | per version |
| npm metadata | registry.npmjs.org | 2026-09-27 |
| GitHub READMEs and design documents | `raw.githubusercontent.com`, default branch (`github.com` and `api.github.com` were blocked, so commit history and release pages could not be checked) | 2026-09-27; cited as `github.com/.../blob/...` |
| automerge.org pages | Page source in [automerge/automerge.github.io](https://github.com/automerge/automerge.github.io) `content/` | 2026-09-27 |
| loro.dev pages | Page source in [loro-dev/loro-docs](https://github.com/loro-dev/loro-docs) `pages/` | 2026-09-27 |
| docs.powersync.com pages | Page source in [powersync-ja/powersync-docs](https://github.com/powersync-ja/powersync-docs) | 2026-09-27 |
| docs.yjs.dev pages | Page source in [yjs/docs](https://github.com/yjs/docs) | 2026-09-27 |
| SQLite session extension | [`ext/session/sqlite3session.h`](https://github.com/sqlite/sqlite/blob/master/ext/session/sqlite3session.h) in the official GitHub mirror (sqlite.org was blocked) | `master`, 2026-09-27 |

**Blocked hosts:** `automerge.org`, `docs.rs`, `lib.rs`, `github.com`, `api.github.com`, `codeload.github.com`, `loro.dev`, `inkandswitch.com`, `vlcn.io`, `electric-sql.com`, `powersync.com`, `docs.powersync.com`, `ditto.com`, `docs.ditto.live`, `sqlite.org`, `arxiv.org`, `eprint.iacr.org`, `codeberg.org`, `willowprotocol.org`, `p2panda.org`, `evolu.dev`, `jazz.tools`, `iroh.computer`, `matrix.org`, `spec.matrix.org`, `datatracker.ietf.org`, `rfc-editor.org`, `messaginglayersecurity.rocks`, `docs.yjs.dev`, `localfirst.fm`, `web.archive.org`. So the Ink & Switch essays and the Keyhive notebook, the papers on arXiv and ePrint, RFC 9420 (MLS), the Matrix spec and the Willow spec were **not read**. Where they matter, the claims below rest on the project's own design documents in its repository.

## 1. CRDT libraries

### 1.1 Automerge

**What it is.** "A library which provides fast implementations of several different CRDTs, a compact compression format for these CRDTs, and a sync protocol" ([README](https://github.com/automerge/automerge/blob/main/README.md)). The core is Rust, compiled to WASM for JavaScript, and it has a C API.

**Maturity and maintenance.**
- `automerge` crate 0.12.0 was released 2026-09-16, with monthly minor releases through 2026 (0.8.0 on 2026-03-25 … 0.11.0 on 2026-08-12). It is MIT, has 37 versions and about 637k downloads ([crates.io](https://crates.io/crates/automerge)). `@automerge/automerge` 3.5.0 came out the same day ([npm](https://www.npmjs.com/package/@automerge/automerge)).
- Automerge 3.0 (2025-07-14) "cut down memory usage by over 10x" and kept the Automerge 2 file format ([blog](https://automerge.org/blog/automerge-3/)).
- Two people work on it full time, with other contributors from Ink & Switch ([README](https://github.com/automerge/automerge/blob/main/README.md), "Status"). Monthly progress posts started in July 2026 ([July](https://automerge.org/blog/2026-july/), [August](https://automerge.org/blog/2026-august/)).
- **Rust API caveat:** "The rust codebase is currently oriented around producing a performant backend for the Javascript wrapper and as such the API for Rust code is low level and not well documented … you may want to look into autosurgeon" ([README](https://github.com/automerge/automerge/blob/main/README.md)). `autosurgeon` 0.14.0 (2026-09-17, MIT) derives Rust structs backed by documents ([crates.io](https://crates.io/crates/autosurgeon)).
- **Rust repo/sync layer:** `samod` 0.14.0 (2026-09-17, MIT) is "an experimental implementation of automerge-repo in Rust … very much a work in progress … don't use this anywhere serious yet" ([README](https://github.com/alexjg/samod/blob/main/README.md)). The August 2026 post describes it as "the Rust automerge-repo implementation, wire-compatible with the JavaScript one" ([blog](https://automerge.org/blog/2026-august/)). The older `automerge_repo` crate was last released 2025-10-03 ([crates.io](https://crates.io/crates/automerge_repo)).
- On npm, `@automerge/automerge-repo`'s `latest` tag points to `2.6.0-alpha.3` (2026-08-07). A `subduction` tag exists (`2.6.0-subduction.48`) and so does an `authors` tag (`2.7.0-authors.1`) ([npm](https://www.npmjs.com/package/@automerge/automerge-repo)).

**Sync model.**
- The sync protocol "assumes a reliable in-order stream between two peers who are synchronizing a document", keeps per-peer state, and loops `generate_sync_message` / `receive_sync_message` until neither side has anything to send. It is based on the paper at https://arxiv.org/abs/2012.00472 (not read: arxiv.org was blocked) ([`src/sync.rs`, 0.12.0](https://docs.rs/crate/automerge/0.12.0/source/src/sync.rs)).
- automerge-repo "works on a per-document basis" ([Concepts](https://automerge.org/docs/reference/concepts/)). The share policy decides which documents are *automatically* shared with a peer; "The default setting is to share all documents with all peers", and it "will not stop a document being *requested* by another peer by its `DocumentId`" ([automerge-repo README](https://github.com/automerge/automerge-repo/blob/main/packages/automerge-repo/README.md)).
- Automerge is built for "working offline, or using multiple independent sync servers, or not having a server at all" ([Automerge 3 blog](https://automerge.org/blog/automerge-3/)). Nothing in the model ties a document to one server.

**Partial sync (R2).** Only at document granularity. Syncing part of a document was not found in any Automerge source read.

**History and attribution (R4).**
- "Automerge stores the full history of documents" ([Automerge 3 blog](https://automerge.org/blog/automerge-3/)).
- In 0.12.0, `AutoCommit::set_author(Option<Author>)` records an `Author`, which "consists of free-form bytes, to identify a single author. For each author, there will be one or more actors", and `Change::author()` reads it back ([`src/author.rs`](https://docs.rs/crate/automerge/0.12.0/source/src/author.rs), [`src/autocommit.rs`](https://docs.rs/crate/automerge/0.12.0/source/src/autocommit.rs), [`src/change.rs`](https://docs.rs/crate/automerge/0.12.0/source/src/change.rs)). No signing or verification of the author bytes was found in that code. The July 2026 post says: "Alex Good has also begun initial work adding author provenance to Automerge … combined with Keyhive this allows for revoking permissions" ([blog](https://automerge.org/blog/2026-july/)).

**E2EE (R3).** Automerge has none built in. See 1.2.

### 1.2 Automerge + Keyhive + Subduction (Ink & Switch's E2EE stack)

**What it is.**
- **Keyhive** is "local-first access control": capabilities ("convergent capabilities"), continuous group key agreement (BeeKEM, "a concurrent TreeKEM variant") and "causal encryption" ([design/README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)). Its crates are `keyhive_core`, `keyhive_crypto`, `beekem` and `keyhive_wasm` ([README](https://github.com/inkandswitch/keyhive/blob/main/README.md)).
- **Subduction** is "a peer-to-peer synchronization protocol and implementation for CRDTs … efficient synchronization of encrypted, partitioned data between peers without requiring a central server". Its transports are WebSocket, HTTP long-poll and Iroh/QUIC, and it has a CLI server ([README](https://github.com/inkandswitch/subduction/blob/main/README.md)). It is the "successor to Beelay" ([Keyhive README](https://github.com/inkandswitch/keyhive/blob/main/README.md)). Its "Sedimentree" scheme splits commit history into layers by hash depth, so peers can diff what they hold "without exposing plaintext data" ([sedimentree design](https://github.com/inkandswitch/subduction/blob/main/design/sedimentree.md)).
- **ARK** (`@automerge/automerge-repo-keyhive`) "adds end-to-end access control and encryption to automerge-repo … document data is encrypted so sync servers relay ciphertext they cannot read" ([README](https://github.com/automerge/automerge-repo-keyhive/blob/main/README.md)).

**Maturity.**
- Keyhive: "pre-alpha … DO NOT use this release in production applications … The code has not had a security audit, and the nonce / key-commitment construction … has not been independently reviewed" ([README](https://github.com/inkandswitch/keyhive/blob/main/README.md)). `keyhive_core` 0.6.0 was released 2026-09-25 (Apache-2.0; 0.3.0 on 2026-03-25, 0.5.0 on 2026-06-26) ([crates.io](https://crates.io/crates/keyhive_core)).
- Subduction: "early release preview. It has a very unstable API … DO NOT use for production use cases" ([README](https://github.com/inkandswitch/subduction/blob/main/README.md)). `subduction_core` 0.18.2 was released 2026-09-13 (MIT OR Apache-2.0) ([crates.io](https://crates.io/crates/subduction_core)). Its server moved to a `redb` storage backend and gained metrics and rate limiting in mid-2026 ([July blog](https://automerge.org/blog/2026-july/)).
- ARK: "Alpha. The public API will change without notice between releases" ([README](https://github.com/automerge/automerge-repo-keyhive/blob/main/README.md)). npm `next` is `0.6.0-alpha.1` (2026-09-25), and `latest` still points to `0.0.0-alpha.53` (2025-10-10) ([npm](https://www.npmjs.com/package/@automerge/automerge-repo-keyhive)). ARK's TypeScript sync-protocol code "will be removed in the near future in favor of a WASM API for the Rust implementation" (same README).
- The BeeKEM paper is a preprint on the Cryptology ePrint Archive ([July blog](https://automerge.org/blog/2026-july/); the paper itself was not read because eprint.iacr.org was blocked).

**Rust vs TypeScript.** The cryptography and sync are Rust crates (`keyhive_core`, `subduction_core`, `subduction_keyhive_policy`, and the `subduction_cli` server) ([Subduction README](https://github.com/inkandswitch/subduction/blob/main/README.md)). The integrated "automerge-repo with Keyhive" package is TypeScript (ARK). `samod` 0.14.0 declares no Keyhive or Subduction dependency ([crates.io dependencies](https://crates.io/crates/samod/0.14.0/dependencies)). So a Rust-native client would have to wire `automerge`, `subduction_core` and `keyhive_core` together itself. No Rust example of that was found.

**Access model (for households).**
- Each document has members at four ordered levels: `relay` ("sync bytes but no read access"), `read`, `edit` and `admin`. Identities are Ed25519 keys, exchanged as "contact cards" ([ARK API guide](https://automerge.org/docs/keyhive/ark-api-guide/)).
- "A user group delegating to per-device keys limits the blast radius to one device, which the user group can revoke" ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md), A4). That is a person → devices structure.
- A JS sync server "can enforce keyhive access control … requires `relay` access to fetch a document, and requires `edit` access to push changes. Unprotected (pre-keyhive) document ids bypass the checks" ([ARK API guide](https://automerge.org/docs/keyhive/ark-api-guide/), "Running a sync server").

**Readable vs unreadable data (R3).** The two kinds can coexist in one repo: `repo.create2` makes a Keyhive-protected, encrypted document, and `repo.create` "silently creates an unprotected document, with no access control or encryption" ([ARK API guide](https://automerge.org/docs/keyhive/ark-api-guide/), "Creating documents").

**What the server still sees.**
- "An observer without plaintext learns document IDs, membership, operation counts, sizes, and timing … Accepted. Relays must see membership to evaluate capabilities" ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md), T8).
- Forward secrecy is a non-goal: "A current reader can decrypt all history" (same document, Non-Goals). A revoked member loses access to future updates (post-compromise security) but keeps whatever they already saw (A2).

**Attribution (R4).**
- "Content chunks carry no signature of their own; content authorship belongs to the CRDT layer" ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md), A5).
- "Distinguishing `Read` from `Edit` on the content path is the CRDT layer's responsibility" (T2).
- On back-dating: "Keyhive cannot distinguish a genuinely old operation that arrives late from a back-dated one. Applications that need this must add a timestamping or anchoring service" (T5). "Trusted time" is a non-goal.

**Why this differs from messaging E2EE.** "In an op-based CRDT like Automerge, any gap in history prevents the application from applying future messages; omitting old history is not possible." Also, "TreeKEM itself requires strict linearizability, and thus does not work in weaker consistency models", which is why BeeKEM exists ([design/README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)).

### 1.3 Loro

**What it is.** A CRDT library for "Rust, JS (via WASM), and Swift", with text (Fugue), rich text, a movable tree, a movable list, an LWW map, time travel and shallow snapshots ([README](https://github.com/loro-dev/loro/blob/main/README.md)).

**Maturity.**
- Loro 1.0 (2024-10-23) introduced "a stable encoding schema" ([blog](https://loro.dev/blog/v1.0)).
- `loro` 1.16.2 was released 2026-09-21 (MIT, 46 versions, about 723k downloads, 438k of them in the last 90 days) ([crates.io](https://crates.io/crates/loro)). `loro-crdt` 1.16.3 is on npm (2026-09-21) ([npm](https://www.npmjs.com/package/loro-crdt)).

**Sync model.**
- Peers exchange version vectors, then `export({ mode: "update", from: version })` and `import(bytes)`. "Two documents with concurrent edits can be synchronized by just two message exchanges" ([Sync tutorial](https://loro.dev/docs/tutorial/sync)).
- Export modes are `update`, `updates-in-range`, `snapshot` and `shallow-snapshot` ([Export mode](https://loro.dev/docs/tutorial/encoding)).
- **loro-protocol** is "a small, transport-agnostic syncing protocol … multiplex multiple rooms on one connection". It has TypeScript packages and a Rust workspace (`loro-protocol`, `loro-websocket-client`, and "`loro-websocket-server`: minimal async WS server with optional SQLite snapshotting"), all MIT ([README](https://github.com/loro-dev/protocol/blob/main/README.md)). Crate versions: `loro-protocol` 0.3.0 (2026-06-12), `loro-websocket-server` and `loro-websocket-client` 0.1.0 (2025-11-30) ([crates.io](https://crates.io/crates/loro-websocket-server)).

**Partial sync (R2).** Per document (room). No sub-document sync was found.

**History (R4).**
- A shallow snapshot "is like Git's Shallow Clone, which can remove old historical" operations ([Loro 1.0 blog](https://loro.dev/blog/v1.0)). Otherwise history is kept, and time travel works on it.
- `LoroDoc::set_next_commit_message` ("It will be persisted") and `set_record_timestamp` add optional metadata to commits. The peer ID is a `u64` set with `set_peer_id` ([`src/lib.rs`, 1.16.2](https://docs.rs/crate/loro/1.16.2/source/src/lib.rs)). No signing or author identity was found.

**E2EE (R3): the `%ELO` extension** ([protocol-e2ee.md](https://github.com/loro-dev/protocol/blob/main/protocol-e2ee.md), protocol version 0):
- "The server never decrypts; it indexes plaintext headers only to support backfill and routing." Records are AES-GCM (256-bit keys, a random 12-byte IV) with the encoded header as additional authenticated data.
- The plaintext header shows the server the peer ID, the counter span `[start, end)` and the key ID. The server keeps `Map<PeerID, Array<Span>>` and answers a requester's version vector with the spans it lacks.
- "Key agreement/distribution is out of scope for this document"; the app supplies a `getPrivateKey(keyId)` hook. Group key management, rotation and revocation are therefore the app's job.
- `%LOR` (plaintext) and `%ELO` rooms use the same envelope ([README](https://github.com/loro-dev/protocol/blob/main/README.md)). One server can therefore carry both readable and E2EE documents.
- Cross-language tests between TypeScript and Rust exist, including a Rust client example that "encrypts/decrypts real `%ELO` containers" (same README).

### 1.4 Yrs (Rust port of Yjs)

**What it is.** "A collection of Rust libraries oriented around implementing Yjs algorithm and protocol … It aims to maintain behavior and binary protocol compatibility with Yjs". The project includes `yrs`, `yffi` (C) and `ywasm` (JS), plus bindings for Python, Ruby, .NET, Swift, Kotlin and R ([README](https://github.com/y-crdt/y-crdt/blob/main/README.md)).

**Maturity.**
- `yrs` 0.28.0 was released 2026-09-17 (MIT, 82 versions, about 3.5M downloads) ([crates.io](https://crates.io/crates/yrs)). `yjs` 13.6.33 was released 2026-09-23. Yjs 14 has been in pre-release since at least 2025-06 (`beta` tag `14.0.0-16`, 2025-12-07) ([npm](https://www.npmjs.com/package/yjs)).
- yrs's default 53-bit client ID follows Yjs; a `small-client` feature keeps compatibility "with older versions of yrs (v0.26 and below) and yjs (pre v14)" ([`src/lib.rs`, 0.28.0](https://docs.rs/crate/yrs/0.28.0/source/src/lib.rs)).

**Sync model.**
- The y-sync protocol: sync step 1 sends a state vector, and sync step 2 replies with the missing update, plus awareness and auth messages ([`src/sync/protocol.rs`](https://docs.rs/crate/yrs/0.28.0/source/src/sync/protocol.rs)).
- Rust servers: `y-sweet` 0.9.1 ("A standalone Yjs CRDT server with built-in persistence and auth", MIT, 2025-09-16) and `yrs-warp` 0.9.0 (2025-07-21) ([crates.io y-sweet](https://crates.io/crates/y-sweet), [yrs-warp](https://crates.io/crates/yrs-warp)).

**Partial sync (R2).** Per document. Yjs subdocuments can be embedded in a root document and "are empty until they are explicitly loaded" ([Subdocuments](https://docs.yjs.dev/api/subdocuments)); `yrs` supports them (README parity table).

**History (R4).** Yjs does not aim to keep full history. In yrs, `Options::skip_gc` "Determines if transactions commits should try to perform GC-ing of deleted items. Default value: `false`", so deleted content is garbage-collected unless the app turns GC off ([`src/doc.rs`](https://docs.rs/crate/yrs/0.28.0/source/src/doc.rs)).

**E2EE (R3).**
- No first-party E2EE sync server was found. `y-webrtc`'s `password` option encrypts communication "over the signaling servers" ([README](https://github.com/yjs/y-webrtc/blob/master/README.md)). That covers the peer-to-peer WebRTC case, not a store-and-forward server.
- **secsync** (third party) "is an architecture to relay end-to-end encrypted CRDTs over a central service", with examples for Yjs and Automerge. It uses XChaCha20-Poly1305 over snapshots and updates, and the server gives each update an integer version so clients can ask for what they lack. "Exchange incl. rotation of the secret key is not part of this protocol". It is labelled "beta software" ([README](https://github.com/serenity-kit/secsync/blob/main/README.md)). The last npm release was 0.5.0 on 2024-06-04 ([npm](https://www.npmjs.com/package/secsync)). It is TypeScript; no Rust implementation was found.

## 2. SQLite-based sync

### 2.1 cr-sqlite (vlcn-io)

**What it is.** "A run-time loadable extension for SQLite and libSQL. It allows merging different SQLite databases together that have taken independent writes" ([README](https://github.com/vlcn-io/cr-sqlite/blob/main/README.md)).
- Tables are upgraded with `crsql_as_crr('table')`. Changes are read from and applied through the `crsql_changes` virtual table (columns `table, pk, cid, val, col_version, db_version, site_id, cl, seq`).
- The current approach is "History-free CRDTs … Keeps no history / only keeps the current state". The "Causal Event Log" approach is marked "To be implemented in v2" (same README).
- "Inserts into CRRs are 2.5x slower than inserts into regular SQLite tables" (same README).

**Maturity.** The last npm releases are `@vlcn.io/crsqlite` 0.16.3 (2024-01-17), `@vlcn.io/crsqlite-wasm` 0.16.0 (2023-12-16) and `@vlcn.io/ws-client` 0.2.0 (2023-12-16) ([npm](https://www.npmjs.com/package/@vlcn.io/crsqlite)). Nothing is published on crates.io (`crsqlite` / `cr-sqlite` not found). Commit activity could not be checked because github.com was blocked. The author joined Rocicorp **[unverified: search excerpt of https://www.localfirst.fm/10]**.

**Fly.io fork.**
- "This fork diverges from upstream cr-sqlite v0.15.0 and introduces significant, breaking changes to the change bookkeeping model. The current version is 0.17.0." It is "maintained by Fly.io and used as the replication engine for Corrosion".
- Merge rule: "If the incoming `col_version` is greater than the local one, the change wins; If versions are equal, the column values are compared; If values are equal, the `site_id` is used as a tiebreaker".
- The fork "expects the application to handle all bookkeeping — gap detection, seq tracking, and buffering". Old sequence entries "can disappear" when later writes supersede them. It adds a per-transaction `ts` column.
- Source: [superfly/cr-sqlite README](https://github.com/superfly/cr-sqlite/blob/main/README.md). Corrosion is Apache-2.0 and gossips changes over QUIC between cluster nodes ([Corrosion README](https://github.com/superfly/corrosion/blob/main/README.md)).

**Partial sync (R2).** `crsql_changes` can be queried with any SQL `WHERE`, for example on `db_version` and `site_id` ([README](https://github.com/vlcn-io/cr-sqlite/blob/main/README.md)). No source read shows whether filtering by table or primary key keeps a partial replica consistent (see Gaps).

**E2EE (R3).** A server that merges must be able to read values, because on equal versions the merge compares column values ([fork README](https://github.com/superfly/cr-sqlite/blob/main/README.md)). A server holding encrypted change rows could only relay them. It would still see table names, primary keys, column names and versions unless those were encrypted too, and encrypting them would stop the server from filtering by them. *(This is an inference from the documented merge rule; no source discusses E2EE for cr-sqlite.)*

**History (R4).** Upstream keeps no history. An append-only record would have to be modelled as insert-only rows.

### 2.2 SQLite session extension

The session extension records changes to attached tables and outputs a *changeset* or *patchset* blob. Another database applies it with `sqlite3changeset_apply()`, which calls an application conflict handler (`SQLITE_CHANGESET_DATA`, `NOTFOUND`, `CONFLICT`, …) that returns `OMIT`, `REPLACE` or `ABORT`. "Changes can only be recorded for tables that have a PRIMARY KEY explicitly defined". A session can attach specific tables or all tables ([`sqlite3session.h`](https://github.com/sqlite/sqlite/blob/master/ext/session/sqlite3session.h); sqlite.org's `sessionintro.html` was blocked).
- **Rust:** `rusqlite` 0.40.2 has a `session` feature (`libsqlite3-sys?/session`, `hooks`) ([crates.io](https://crates.io/crates/rusqlite/0.40.2/features)).
- **What it is not:** it has no CRDT or causal merge. Offline edits to the same row come back as conflicts for the app to resolve. Changesets are opaque blobs that can be encrypted as events (see section 4), but applying one needs the plaintext and a matching schema.

### 2.3 PowerSync

- **What it is:** it "keeps a client-side SQLite database in sync with your backend database … Supports Postgres, MongoDB, Azure DocumentDB, MySQL, and SQL Server" ([powersync-service README](https://github.com/powersync-ja/powersync-service/blob/main/README.md)). The service is a TypeScript/Node monorepo (built with `tsc`, per [`service-core/package.json`](https://github.com/powersync-ja/powersync-service/blob/main/packages/service-core/package.json)) under the **Functional Source License FSL-1.1-ALv2** ([LICENSE](https://github.com/powersync-ja/powersync-service/blob/main/LICENSE)). It can be self-hosted with Docker ([Self-Hosting](https://docs.powersync.com/intro/self-hosting)).
- **Writes** go through your own API: "Your backend application receives the write operations based on how you defined your `uploadData()` function … It's important that your API endpoint be blocking/synchronous with underlying writes to the backend source database" ([Writing Client Changes](https://docs.powersync.com/handling-writes/writing-client-changes)). The source database is authoritative. Offline writes queue on the client and are applied by the backend.
- **Partial sync:** Sync Streams, defined as SQL queries with parameters ([powersync-native README](https://github.com/powersync-ja/powersync-native/blob/main/README.md)).
- **Rust and Tauri:** the feature-status page lists "Tauri SDK | Alpha" and "Rust SDK | Alpha" ([Feature Status](https://docs.powersync.com/resources/feature-status)). The `powersync` crate is at 0.0.7 (2026-08-03) and `powersync_core` (the SQLite extension, Apache-2.0) at 0.5.3 (2026-08-13) ([crates.io](https://crates.io/crates/powersync)).
- **E2EE:** "For end-to-end encryption, the encrypted data can be synced using PowerSync. The data can then either be encrypted and decrypted directly in memory by the application, or a separate local-only table can be used to persist the decrypted data" ([Data Encryption](https://docs.powersync.com/client-sdks/advanced/data-encryption)). Sync Stream filters run on the server, so any column used in a filter has to stay plaintext *(inference)*.

### 2.4 Electric

"Electric is a read-path sync engine for Postgres. It syncs data out of Postgres into ... anything you like … Partial replication is managed using Shapes." It is Apache-2.0 and built with Elixir and Erlang ([README](https://github.com/electric-sql/electric/blob/main/README.md), [LICENSE](https://github.com/electric-sql/electric/blob/main/LICENSE)). The TypeScript client `@electric-sql/client` is at 1.5.28 (2026-09-09) ([npm](https://www.npmjs.com/package/@electric-sql/client)). Its server language is outside the project's Rust/TypeScript constraint. Postgres must be able to read whatever columns shapes filter on.

### 2.5 sqlite-sync (SQLite Cloud)

"Offline-first sync for SQLite, powered by CRDTs … Sync to SQLite Cloud, PostgreSQL, or Supabase", with server-enforced row-level security ([README](https://github.com/sqliteai/sqlite-sync/blob/main/README.md)). The licence is "Elastic License 2.0 (modified for open-source use)". Its open-source grant does **not** cover "modifying, replacing, bypassing, reimplementing, or substituting the Network Layer", and "a commercial license … is required if you … implement or use an alternative network transport … synchronization mechanism … server protocol" ([LICENSE.md](https://github.com/sqliteai/sqlite-sync/blob/main/LICENSE.md)). `@sqliteai/sqlite-sync` is at 1.1.4 (2026-09-21) ([npm](https://www.npmjs.com/package/@sqliteai/sqlite-sync)).

### 2.6 Ditto

"A cross-platform, peer-to-peer database that allows apps to sync with and without internet connectivity", over Bluetooth, P2P Wi-Fi and LAN ([crate README, 5.1.0](https://docs.rs/crate/dittolive-ditto/5.1.0/source/README.md)). It ships under the "Ditto Binary License", which is proprietary: "You agree not to attempt to decompile, disassemble, reverse engineer…" ([LICENSE.md](https://docs.rs/crate/dittolive-ditto/5.1.0/source/LICENSE.md)). `dittolive-ditto` 5.1.0 was released 2026-08-19 ([crates.io](https://crates.io/crates/dittolive-ditto)). An on-premises "Ditto Server" exists **[unverified: search excerpt of https://docs.ditto.live/ditto-server/operator/operator-quickstart]**. E2EE and partial-sync behaviour were not verified.

## 3. Other candidates found

- **Evolu** is "TypeScript library and local-first platform" (MIT; `@evolu/common` 8.11.0, 2026-09-26) ([npm](https://www.npmjs.com/package/@evolu/common), [repo](https://github.com/evoluhq/evolu)). It is described as SQLite plus CRDT with E2E-encrypted sync through a self-hostable relay that "see[s] only timestamps", using range-based set reconciliation **[unverified: search excerpt of https://www.evolu.dev/docs/local-first]**. It is TypeScript only, with no Rust implementation found. It matters here as prior art for E2EE SQLite sync.
- **Jazz 2.0** is "a local-first relational database. It runs across your frontend, backend and our global storage cloud. Sync partial tables" ([README](https://github.com/garden-co/jazz/blob/main/README.md)). The repo says: "this is the Jazz 2.0 alpha with an entirely new API". It is built from Rust crates plus TypeScript, and its server uses RocksDB. npm `alpha` is `2.0.0-alpha.57` (2026-09-26) ([npm](https://www.npmjs.com/package/jazz-tools)). E2EE in 2.0 was not verified.
- **Turso** is an "in-process SQL database written in Rust, compatible with SQLite" ([README](https://github.com/tursodatabase/turso/blob/main/README.md)). It has a `turso_sync_engine` crate (0.8.0-pre.13, 2026-09-25) and `@tursodatabase/sync` 0.7.2 (MIT) ([crates.io](https://crates.io/crates/turso_sync_engine), [npm](https://www.npmjs.com/package/@tursodatabase/sync)). No documentation of the sync model or of a self-hostable sync server was reachable, so it is not assessed here.
- **Willow / Meadowcap** (`willow25` 0.7.9, 2026-08-27; `meadowcap` 0.6.0; MIT OR Apache-2.0) implement the Willow data model (paths, entries, "groupings", read and write capabilities) in Rust ([crates.io](https://crates.io/crates/willow25), [crate docs, 0.7.9](https://docs.rs/crate/willow25/0.7.9/source/src/lib.rs)). The spec site and the Codeberg repository were blocked, so its sync and encryption story was not assessed.

## 4. Plain event log

**Idea.** Record every change as an immutable event. The history's items (an occurrence firing, completing it, skipping it) already are events. Devices exchange the events they lack and derive current state by folding over them. This is the pattern that p2panda, iroh-docs, secsync and (in "Approach 2", not built) cr-sqlite describe.

**Against the requirements.**
- **R1 (offline merge):** new events from different devices always merge as a set union. Data that changes, such as a reminder's schedule or conditions being edited on two offline devices, still needs a merge rule: last-writer-wins by a clock, multi-value, or a CRDT stored inside the event payload. This is where CRDT libraries save work. p2panda's approach is "compatibility with any CRDT" over its logs ([p2panda-core README](https://docs.rs/crate/p2panda-core/0.7.1/source/README.md)).
- **R2 (partial sync):** partition the log, for example one stream per reminder or per household. A device subscribes only to its streams.
- **R3 (E2EE):** the server needs only routing metadata (stream ID, author, sequence number or hash, size). The payload can be ciphertext. This is secsync's model (encrypted snapshots and updates, with the server assigning integer versions) ([secsync README](https://github.com/serenity-kit/secsync/blob/main/README.md)). Readable and encrypted streams can sit side by side.
- **R4 (attributed append-only history):** events can be signed per author, which gives cryptographic attribution. Signed, append-only per-author logs are p2panda's core data type.

**Rust building blocks.**
- **p2panda** (MIT OR Apache-2.0, 0.7.1 on 2026-08-21; "APIs are not yet considered stable for production use … Stability guarantees will improve with the release of v1.0.0"):
  - `p2panda-core`: "an append-only log implementation which supports history deletion, multi-writer causal-ordering, fork-tolerance, compatibility with any CRDT", with "Cryptographic signatures for authorship verification" and "Single-writer logs which can be combined to support multi-writer collaboration" ([README, 0.7.1](https://docs.rs/crate/p2panda-core/0.7.1/source/README.md)).
  - `p2panda-sync`: two-party sync protocols over append-only logs ([README](https://docs.rs/crate/p2panda-sync/0.7.1/source/README.md)).
  - `p2panda-encryption` and `p2panda-auth`, described under section 5.
  - TypeScript: only experimental "Node.js, Python and Go support via UniFFI" ([README](https://github.com/p2panda/p2panda/blob/main/README.md)).
  - p2panda is built for peer-to-peer networking (on iroh). Whether it fits a client–server shape was not checked.
- **iroh-docs** (0.101.0, 2026-06-15, MIT/Apache-2.0): "Multi-dimensional key-value documents with an efficient synchronization protocol". Entries are keyed by key, author and namespace, and signed with a namespace key ("write capability") and an author key ("proof of authorship"). Sync uses "range-based set reconciliation" ([README](https://github.com/n0-computer/iroh-docs/blob/main/README.md); paper https://arxiv.org/abs/2212.13567, not read). No encryption layer is described.
- **SQLite session extension** (section 2.2): turns local SQLite writes into changeset blobs that can serve as event payloads. Conflict handling stays with the app.

**Costs.** Custom merge rules for data that changes. Snapshotting or compaction so that new devices do not replay everything. Custom wire protocol, server and schema versioning of events. No outside test suite for convergence.

## 5. Group keys for E2EE across a household

Any E2EE option with more than one person reading needs a way to give every current member the key and rotate it when someone leaves. Only Keyhive and p2panda bundle this; Loro `%ELO` and secsync leave it to the app.

| Library | Scheme | Version (date) | Licence | Status / notes |
|---|---|---|---|---|
| `keyhive_core` / `beekem` | BeeKEM, a concurrent TreeKEM variant, plus capabilities | 0.6.0 (2026-09-25) | Apache-2.0 | Pre-alpha, unaudited; no forward secrecy by design ([threat model](https://github.com/inkandswitch/keyhive/blob/main/design/threat_model.md)) |
| `p2panda-encryption` | "Data Encryption" (a shared group key, rotated on removal) and "Message Encryption" (per-member ratchets); 2SM key agreement from the paper "Key Agreement for Decentralized Secure Group Messaging with Strong Security Guarantees" (2020) | 0.7.1 (2026-08-21) | MIT OR Apache-2.0 | Pre-1.0. "No centralised server is required for coordination of the group" ([README](https://docs.rs/crate/p2panda-encryption/0.7.1/source/README.md)) |
| `openmls` | MLS, RFC 9420 | 0.9.0 (2026-08-25) | MIT | "If members of a group merge different commits, the group state is called forked … will not be able to decrypt each others' messages" ([Fork Resolution](https://github.com/openmls/openmls/blob/main/book/src/user_manual/fork-resolution.md)); "The delivery service may reject a commit sent by a client" ([Discarding commits](https://github.com/openmls/openmls/blob/main/book/src/user_manual/discarding_commits.md)). MLS assumes someone orders commits |
| `mls-rs` (AWS Labs) | MLS, RFC 9420 | 0.56.0 (2026-08-19) | Apache-2.0 OR MIT | Not read beyond crates.io metadata ([crates.io](https://crates.io/crates/mls-rs)) |
| `vodozemac` | Olm and Megolm (Matrix) | 0.11.0 (2026-09-11) | Apache-2.0 | "used for end-to-end encryption in Matrix" ([README](https://github.com/matrix-org/vodozemac/blob/main/README.md)); the Matrix spec was not reachable |

Keyhive's designers say TreeKEM (and so MLS) "requires strict linearizability, and thus does not work in weaker consistency models", and name that as the reason BeeKEM exists ([design/README](https://github.com/inkandswitch/keyhive/blob/main/design/README.md)). RFC 9420 itself could not be read. With one server per account and household members on the same server, that server is a place where commits *could* be ordered. Whether that holds up for offline devices, and later for sharing across servers, has not been examined (see Gaps).

## 6. Gaps and open questions

**Could not be verified here.**
- Anything only on blocked sites: Ink & Switch essays and the Keyhive notebook; the arXiv sync papers (Automerge's sync-protocol basis, range-based set reconciliation); the BeeKEM preprint; RFC 9420 and the MLS architecture; the Matrix spec; vlcn.io docs (including cr-sqlite's own networking and partial-sync tutorials); electric-sql.com docs; ditto.com docs; evolu.dev; jazz.tools; willowprotocol.org.
- GitHub commit activity, open-issue counts and release pages (github.com was blocked). Maintenance status is inferred only from registry release dates and README text.
- Whether upstream cr-sqlite is formally unmaintained. There have been no releases since 2024-01-17, but no statement was found.
- Whether filtering `crsql_changes` by table or primary key gives a correct partial replica, and how version tracking behaves when a peer holds only a subset.
- Whether any Rust client (not the TypeScript ARK) runs Automerge + Keyhive + Subduction end to end. No example was found.
- Ditto's self-hosting terms, E2EE and partial sync. Evolu's and Jazz 2.0's encryption. Turso's sync server.
- Android specifics: none of the sources read discusses building these crates for Android inside a Tauri app. Each is a Rust crate, but that was not tested.

**Questions the design depends on (facts to settle before choosing).**
1. **What must the server read?** If the server has to act on reminders, for example sending alerts to other household members or evaluating triggers, that data cannot be E2EE. Keyhive relays still see document IDs, membership, sizes and timing, and `%ELO` servers still see peer IDs and spans. Which of this metadata is acceptable?
2. **What is the unit of partial sync?** Every CRDT option syncs whole documents. Should the boundary be one document per reminder, per household or per person? The choice drives the document count (Subduction's work was driven by apps loading "low thousands of Automerge documents") and what the server learns from routing.
3. **How strong must attribution be?** Is "completed by Alex" a self-asserted field (Automerge `Author` bytes, a column in a row), or must it be signed (p2panda logs, iroh-docs author keys)? Keyhive signs membership but not content (threat model A5). Keyhive also cannot tell back-dated operations from late ones (T5): does the history need trusted time?
4. **Does history have to be complete and permanent?** Yjs garbage-collects deleted content by default, Loro shallow snapshots drop history, and cr-sqlite keeps none. Automerge and Keyhive keep all history, and a reader can decrypt all of it (no forward secrecy). What retention does the append-only record need, and should a removed household member keep what they already saw?
5. **Where does sync logic run: in the Rust core or in the TypeScript UI?** In a Tauri app the Rust core can own sync, which makes TypeScript bindings optional. The Keyhive-integrated path, however, is TypeScript today (ARK). *(Tauri's documentation was not reachable, so how Tauri IPC would carry this was not assessed.)*
6. **Can group key agreement rely on the server to order commits?** MLS assumes commits are ordered, BeeKEM and p2panda's 2SM do not, and Loro `%ELO` and secsync leave keys to the app. The answer affects offline devices changing membership and, later, sharing across servers.
7. **What will sharing across servers need?** Automerge documents, Keyhive identities (keys, not accounts) and signed per-author logs are all independent of any server. The server-authoritative options (PowerSync, Electric, sqlite-sync) tie data to one backend database.
8. **Which licences are acceptable for a later public product?** FSL-1.1-ALv2 (PowerSync service), the modified Elastic 2.0 that forbids replacing the network layer (sqlite-sync) and the Ditto Binary License carry restrictions. The CRDT libraries, p2panda, iroh-docs, cr-sqlite and the MLS crates are MIT and/or Apache-2.0.
