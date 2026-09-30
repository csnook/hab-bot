# Encrypted reminder lists use a per-list key sealed to each device

Each encrypted reminder list has one symmetric key, sealed separately to the public key of every device that has access. The server stores the sealed copies and orders changes to them. Removing a user rotates the list key and seals the new one to everyone who remains; adding a user means a device of someone with manage access seals the current key to the new user's devices. We chose this over a group key agreement protocol because every account lives on one server that already orders events (it decides which claim came first), so the leaderless-group machinery those protocols exist for buys nothing here, and a simple scheme built from well-reviewed primitives is easier to get right in a learning project.

## Considered Options

- **MLS (OpenMLS, mls-rs)**: needs someone to order commits, which our server could do, but brings a large protocol for guarantees we don't need.
- **Keyhive / BeeKEM**: designed for groups without a central orderer; pre-alpha and unaudited.
- **p2panda-encryption, Megolm (vodozemac)**: also aimed at decentralised groups or messaging, with more machinery than a household server needs.

## Consequences

- No forward secrecy: anyone with current access can read the list's whole history, and a removed user keeps what they have already seen.
- Sharing between servers (federation), if it ever comes, would need someone to order key changes across servers; this scheme does not rule that out but does not solve it.
