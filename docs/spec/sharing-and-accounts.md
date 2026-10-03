# People, sharing and accounts

> **First release:**
> - **In:** one user on their own server:
>   - the one-time setup code, which creates the first account as server admin
>   - a password, through OPAQUE and Argon2id
>   - signing in on a second device with the password, or approving it by QR code
>   - the device list, where Remove rotates the keys
>   - changing the password
>   - a standalone device joining later
>   - the password backoff and failed sign-in notice
>
>   When the password is set, the app warns that losing every device *and* the password loses the data.
> - **Later:** invites for other people, groups, sharing, invitations, claims, assignments, rotation, verification, escrow, admin resets, leaving the server and deleting the account.

## Users, accounts and devices

- **A user** is someone who uses the app, usually a person. A screen several people share, such as a kitchen tablet, is a user of its own.
- **An account** is a user's registration on a server. A standalone user has none, and a user has at most one. Say "account" only when joining, signing in, recovering or deleting, and "user" otherwise.
- **A device** is a phone, tablet, computer or browser running the app, belonging to exactly one user.
  - Two people with separate logins on one Linux computer are two devices.
  - Whatever a user does to an occurrence applies on all of their devices.
- **Standalone:** the app works on one device with no account and no server. Sharing, several devices and recovery need a server.

## Joining a server

- **Servers are invite-only.**
  - **The first account:** on first start the server shows a one-time **setup code** in its console. Whoever joins with it becomes the first server admin. The code is random and long, and stops working 24 hours after the server starts or once the first account exists. Restarting makes a new one, so a server exposed before setup can't be taken over.
  - **Everyone after that:** joining needs an **invite** from a server admin. It's a single-use link or QR code that carries the server's name, address and certificate fingerprint, and expires after 7 days. It can also add the person to groups.
  - **Open sign-up** may become a server setting for a public product later.
- **Joining steps:**
  1. Open the invite. Confirm the server, its fingerprint, how long it keeps history, and any groups the invite adds you to.
  2. Choose a username and display name.
  3. Choose a password. It must pass the strength check, and Continue stays disabled until it does.
  4. Turn on escrow, if you want it and server trust allows it.
  5. Set your cap on other users' reminders, with Maximum preselected.
  6. Say whether this device is portable. Phones and tablets skip this step, since they're portable.
- **"What your server can see"** is shown while joining, and again in Account settings. It explains:
  - what the server stores: accounts, devices, access, and when each list changed
  - that IP addresses aren't logged unless the server admin turns on debug logging
  - that anything in between, such as a VPN provider, Google's push service or your internet provider, can log when and from where you connect, whatever the server does

## Passwords and devices

[ADR 0004](../adr/0004-password-unlocked-accounts-and-device-vouching.md) records this design.

- **What the password unlocks:**
  - Each user has an **identity key**, which signs their devices, and **personal keys**.
  - Devices derive a key from the password with **Argon2id**, and that key encrypts the identity and personal keys. The server stores only the encrypted bundle.
- **The server never sees the password.** It hands out the bundle only after an **OPAQUE** check, so outsiders can't download it and guess offline. Whoever holds the server still could, so:
  - **Strength is enforced.** The app refuses passwords estimated to be guessable, and suggests a passphrase of four or more random words.
  - **Password checks are rate-limited.** After 5 failures for an account, each attempt waits twice as long as the last, from 1 minute up to 1 hour. The user's devices get a notice, "5 failed sign-ins to your account". Each IP address is also limited to 20 attempts a minute, counted in memory only.
- **Signing in on a new device** takes the server address, username and password, and the device is vouched for straight away. Approving it from an existing device by QR code is an alternative that needs no typing.
  - **Either device shows the code:** an existing device (Settings → Account → Approve a new device) shows a link, also as a QR code, for the new device to scan; or the new device shows one and the existing device scans it. Where a device has no camera, the link is pasted instead.
  - **The code carries** the server's address and certificate fingerprint, the approval's id and a random key made by the device that shows it. The two devices send each other their messages through the server, sealed under that key, so the server can neither read them nor put a device of its own in place of the new one. The code works once and expires after five minutes.
  - **The existing device shows the new device's name** and asks for confirmation. Approving signs the new device with the identity key, gives it the account's keys (sealed under the code's key) and seals it every version of the personal list's key. The new device then signs in as after a password sign-in, and announces itself to the other devices.
- **Every new sign-in is announced** on the user's other devices: "New device signed in: Pixel 9, just now. Not you? Remove it".
- **The device list:** Settings → Account lists the user's devices with their names and when each last synced. The names come from the user's encrypted settings, and the time from the server, which sees when a device last connected but nothing else about it.
- **Removing a device** from any other device of the same user takes it off the account and rotates the keys of every list it held:
  - the new key is sealed to every device that remains, together with the older keys, so they can still read the whole history
  - the server deletes the device's row and its sealed keys in the same step, so the device can no longer fetch or send anything, and its open connection is closed
  - "Not you? Remove it" in the new-device notice opens Settings → Account at that device, to confirm
  - the personal list's first key is derived from the personal key, which the removed device holds, so the new key is random instead ([ADR 0008](../adr/0008-personal-list-key-rotates-to-a-random-key-on-device-removal.md))
- **Changing the password** from any signed-in device re-encrypts the bundle. Nothing else changes.
- **A forgotten password:**
  - **With a device still signed in:** set a new password there. On Android this asks for the screen lock first. On Linux an unlocked session is enough.
  - **With no device left:** escrow restores the personal keys, if it was on. Otherwise a server admin resets the account: the user gets a new identity, and their personal data is lost.
- **Devices vouch for each other:** other users' devices seal list keys only to devices signed by the user's identity, never on the server's word alone. That stops a dishonest server admin from adding a device to someone's account to receive their lists.

## Server admins

- **Who:** the first account. Server admins can make other users server admins, and the last one can't step down.
- **Can:**
  - make and revoke invites
  - see the accounts and their devices
  - remove an account
  - reset an account whose owner has no device, no password and no escrow
  - verify a user before releasing their escrow
  - change server settings, including the server's retention limit
- **Can't:** read encrypted data (unless that user chose escrow), or change anyone's access, including their own.
- **The affected user is always told** about any action on their account.
- **Wording:** always "server admin" or "group admin", never a bare "admin".

## Escrow, resets and verification

- **Escrow** is optional and off by default. The server keeps a copy of the user's personal keys and releases it after verifying the user.
  - **The opt-in screen says plainly** that whoever runs the server could read that user's personal reminders.
  - **Coverage:** personal data only. No one else's privacy depends on one user's escrow choice.
  - **Server trust:** a device whose server trust forbids it refuses to turn escrow on. **Server trust** is a device's own limit, per server, on how much it lets that server read.
  - **Turning it off** deletes the escrowed copy and rotates the personal keys.
- **Releasing escrow:**
  - A server admin first verifies the person in person or by phone.
  - Release then waits **1 hour**. The user's remaining devices are notified, and any of them can cancel the release.
  - An admin reset gets the same wait and notices.
  - The wait is to be reconsidered before 1.0. Email verification waits for a public product.
- **Neither escrow nor a reset restores the identity.**
  - Others see "Sam's account was reset", and the user is **unverified** on each shared list until someone with manage access on it **verifies** them.
  - **Verifying:** both screens show the same five everyday words, such as "maple · orbit · candle · river · flint", to compare in person.
  - If they match, the list key is sealed to the user's new devices. If they don't, nothing is shared and both people are warned.

## Standalone and back

- **Joining from a standalone device** makes it your first device. Everything on it is uploaded, encrypted: reminders, history, sources and custom priorities.
- **Linking a standalone device to an existing account** brings its reminders in as a separate list named after the device.
- **Leaving a server:**
  - The device keeps your personal lists and their history, and becomes standalone.
  - Shared lists are removed from it. You can keep a copy of a shared list's reminders, without history, as a personal list.
  - The device is removed from your account, and the keys it held rotate.
  - On your last device, you're asked whether to delete the account.
- **Deleting an account** is handled as if you'd lost access to everything. The server deletes your encrypted data.

## Groups

- **A group** is a defined set of users, such as a household, that reminders can be shared with as a whole.
- **Creating one:** any user can create a group, and becomes its first **group admin**.
- **Membership:**
  - Group admins send **invitations**, which the user accepts or declines.
  - Group admins can remove members and make other members group admins.
  - A person can belong to several groups.
- **Leaving:** members can leave at any time. The last group admin must hand over first, unless they're the last member, in which case the group is deleted.
- **No other roles.** Different access for different members comes from sharing a list with the group at one level, and with some members individually at a higher one.
- **Members see everyone in the group.** Joining needs consent, so home and public servers behave the same.
- **A group's retention limit** is set by its group admins, and starts as its creator's.

## Sharing and access

- **What sharing is for:** a shared reminder coordinates something that *someone* needs to do, not something *everyone* does. Personal routines such as medicine are each person's own reminders.
- **What can be shared:**
  - A reminder list can be shared with users, groups, or both. For example, the household gets "Chores" and "Pets", and a dog-sitter gets only "Pets".
  - A single reminder can also be shared with specific users, like inviting guests to an event. It then moves into a stream of its own, so they never hold the key to its original list.
  - The personal list itself can't be shared. A reminder can be shared from it instead.
- **Access levels:**

  | Level | Can |
  |---|---|
  | **View** | See it. Never alerted. |
  | **Act** | Also be alerted, and claim, decline, complete, skip, snooze and acknowledge. |
  | **Edit** | Also create, change and delete reminders. |
  | **Manage** | Also share it and change access. |

  - Users a reminder is shared with directly get act by default, and the owner can give them edit.
  - **With access in several ways,** a user gets the highest. For example, "Chores" is shared with the household at act and with the parents at edit.
  - **Everyone with access sees who else has it,** and at what level.
  - **A list always keeps at least one user with manage access.**
- **How shares arrive:**
  - **Individual shares arrive as invitations.** There are no alerts until the user accepts.
  - **Group shares arrive without one,** for every member including later joiners, once a device of someone with manage access comes online and seals the key.
- **Changing a level** takes effect at once. Dropping to view stops alerts, and no key is rotated.
- **Losing access** means being removed, leaving a list or group, or deleting the account:
  - the list key rotates, and the user's devices drop the list
  - their claims on open and upcoming occurrences, and their assignments, are released, and the others are told
  - they leave any rotation
  - the history keeps their name on past actions, marked as a former member
- **Invitations** arrive with one gentle notification and never ring or repeat. Until answered, they show as a banner with **Accept**, **Decline** and Details, and a badge on Settings → Groups.

## Claims and turns

- **Claiming:** notifications for shared reminders include **Claim**. When several users claim the same occurrence, the first claim the server receives wins.
  - **After a claim,** everyone else's alerts stop and they see "claimed by Sam". The claimant's current alert is silenced, but the occurrence keeps ageing and escalates for them.
  - **If it goes overdue while claimed,** the claim stays on record and everyone it's shared with is alerted again, at their own priority.
  - **Releasing** a claim alerts the others straight away.
  - **Completing without claiming** counts as a claim and a completion in one step.
  - **Offline claims** are provisional ("claiming…") until they reach the server. If someone else's claim got there first, you're told then.
- **Assigning:** someone with edit access can assign a reminder's occurrences to a user. That's a claim made ahead of time on their behalf, so the occurrence fires already claimed.
- **Rotation:** turns among some of the users with act access.
  - The next turn goes to whoever did it least recently, so doing it on someone else's turn evens out on its own.
  - A skip or a miss leaves the same person up.
  - Someone can be paused in a rotation without being removed.
- **Claiming ahead of time:** a user can claim an expected occurrence ("I'll do next Tuesday's trash"). That overrides the rotation for that occurrence, and a swap works out on its own.
- **Recorded for each shared occurrence:**
  - whose turn it was, or who it was assigned to
  - every claim, including ones that lost or were released, with who made it and when
  - who completed or skipped it
  - who was alerted

## What users see of each other

- **Profile:** a display name and an optional avatar, both readable by the server.
- **Who you can see:** the members of your groups, and everyone with access to a list or reminder you have.
- **Sharing with someone new:** enter their exact username or scan their QR code. There's no server-wide directory.
- **No presence:** no online status, no "last seen", no location.
- **Others see who did something, never which device.** Device names live in each user's own encrypted settings.

## Shared screens

- **A shared screen** such as a kitchen tablet is an ordinary account, made with an invite, with a password the household knows.
- **It's marked as a shared screen,** so its name appears as "Kitchen (shared screen)".
- **Its access comes the usual way,** for example act access through the household group. What's done there is recorded as that user.
- **A "who did this?" picker** can come later without changing anything.

## Decided in

- [How shared reminders work across a group][10]
- [Which data the server may read][14]
- [One person, several devices][23]
- [Accounts, groups and permissions][24]
- [Desktop dialogs and settings][29]
- [Security hardening before the server goes on the internet][33]
- [ADR 0002](../adr/0002-per-list-keys-sealed-to-devices.md) and [ADR 0004](../adr/0004-password-unlocked-accounts-and-device-vouching.md)

[10]: https://github.com/csnook/hab-bot/issues/10
[14]: https://github.com/csnook/hab-bot/issues/14
[23]: https://github.com/csnook/hab-bot/issues/23
[24]: https://github.com/csnook/hab-bot/issues/24
[29]: https://github.com/csnook/hab-bot/issues/29
[33]: https://github.com/csnook/hab-bot/issues/33
