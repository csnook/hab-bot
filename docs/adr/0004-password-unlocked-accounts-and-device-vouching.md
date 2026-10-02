# Accounts are unlocked by a password, and a user's devices vouch for each other

Each user has an identity key that signs each of their devices, and other users' devices seal list keys (ADR 0002) only to devices signed that way, never on the server's word alone. The identity and personal keys are stored on the server, encrypted under a key that the user's devices derive from their password with Argon2id. The password never leaves the devices, and the server hands out the encrypted keys only after an OPAQUE check. A new device therefore signs in with a username and password alone, and is vouched for straight away. We chose this so that signing in on a new device needs only something the user remembers, accepting that whoever holds the server can try to guess the password offline. The memory-hard derivation and a minimum password strength are what stand against that.

## Considered Options

- **Device keys only, with a recovery key kept on paper**: nothing on the server to guess, but a new device needs an existing device or the recovery key to hand.
- **A password only for signing in to the server**, with the keys still coming from existing devices: one more thing to forget, and it protects little, since the server can't unlock anything with it.
- **Trusting the server's list of a user's devices**: simpler, but a dishonest server admin could add a device to someone's account and receive the next list key.

## Consequences

- A weak password would expose the user's personal data and every list they can read, so the app refuses passwords estimated to be guessable.
- Every new sign-in is announced on the user's other devices, and removing a device rotates the keys it held.
- A forgotten password can be replaced from any device still signed in. With no device left, escrow restores the personal keys, or a server admin resets the account. Neither restores the identity, so each shared list's managers must verify the user before that list comes back. This keeps anyone else's privacy from depending on one user's escrow choice.
- The exact parameters and libraries belong to the sync design.
- The same rule covers access and group membership. Every access change is signed by the device of the user with manage access who made it, and every membership change by a group admin's device. They are stored unencrypted, since the server reads access, and numbered in the list's stream (ADR 0005). Before sealing a key or accepting an event, a device checks those signatures, and checks that the event's author had enough access at that point in the stream. The server enforces access from the same records but can't forge them.
