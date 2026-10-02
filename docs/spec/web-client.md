# Web client

> **First release:** not included. The web client comes after the apps, which leaves time for the Rust core to settle before it's compiled to WebAssembly. Its setup work comes with it:
> - a domain or Tailscale name with a trusted certificate
> - the GitHub Pages site
> - a check that Argon2id and OPAQUE run acceptably in WebAssembly on a mid-range phone

The **web client** is the app running in a web browser, loaded from a static web host rather than from the user's server. Each browser that signs in is a device of its own.

## Where the code comes from

[ADR 0007](../adr/0007-web-client-from-a-static-host-with-a-browser-trusted-server-certificate.md) records this choice.

- **Not from the sync server.** The web client is a static bundle served from a static host and CDN. It contains:
  - HTML and JavaScript
  - the Rust core compiled to WebAssembly
  - the same Preact UI as the apps

  A dishonest server admin therefore can't change the code that handles your password and keys. You trust whoever publishes the bundle, as you trust whoever publishes the apps.
- **The official host is GitHub Pages for now,** built from tagged releases. The site shows its version and links to the exact source.
  - **Caching:** file names carry hashes and have Subresource Integrity, so caching stays correct without custom headers, which Pages can't set. Only `index.html` and the service worker change between versions.
  - **Passwords:** Pages' "no passwords" rule is about sending passwords to Pages. Ours only go to your own server, through OPAQUE.
  - **Other hosts:** anyone can deploy the same bundle to their own static host. A host with custom headers, or a CDN, can replace Pages later.
- **The known limit:** whoever controls the static host could replace the service worker, which browsers re-check at least daily. Subresource Integrity can't pin the page or the worker. To make a quiet swap noticeable:
  - each release lists the hashes of `index.html` and the service worker
  - About shows the running version and its hash
  - the publisher's GitHub account uses two-factor sign-in and protected release tags
  - from 1.0, builds run in GitHub Actions with signed build attestations
- **Updates:** the service worker fetches a new version in the background and shows **"Update available · Reload"**. It never swaps code in the middle of a session. If the web client is too old for the server's events, it shows "update the app to see recent changes".

## Finding and reaching your server

- **Invite links and sign-in codes** can open the web client directly. The server's address goes after the `#`, so the static host never learns which server you use.
- **One account per browser profile.** Data in IndexedDB is kept separately per server.
- **The server needs a browser-trusted certificate.**
  - **Why:** browsers can't pin a self-signed certificate, and Chrome's service workers reject one even after you click through.
  - **Where it comes from:** the server reads it from files, whatever issued it:
    - Tailscale, for a machine's `ts.net` name
    - Let's Encrypt's DNS-01 check for a domain of your own, through a tool such as certbot, which works without the server being reachable from the internet
  - **Automation:** getting it is automated later, only for providers whose API clients fit easily into the Rust server, and possibly through Tailscale with one API key. Until then it's set up by hand.
  - **The apps** keep pinning, and also accept the trusted certificate.
  - **Costs:** a domain (unless using Tailscale), the name appearing in public certificate logs, and on some routers a local DNS override for names that point to a private address.
- **The local-network prompt:** Chrome and desktop Firefox ask before a public page reaches a LAN or VPN address. The web client explains "access other devices on your local network" before the prompt appears.
- **Without a trusted certificate,** the web client works only **standalone**, keeping your data in that browser alone, with a clear warning that browsers can delete stored data.

## Storage and keys

- **Storage:** IndexedDB for the data, and a service worker that caches the app itself for offline use. Persistent storage is requested.
- **Keys:** the device keys are non-extractable WebCrypto keys (Ed25519 and X25519, supported in all three browser engines), stored in IndexedDB. As in the apps, this relies on the platform to protect them.
- **Sign-in asks "Remember this browser?",** defaulting to yes. If you say no, nothing is stored and you sign in on every visit, which suits a shared computer.
- **Each browser that signs in is a device,** announced on your other devices.
- **Portable or stationary:** portable by default on small screens, and stationary on large ones.

## Alerts

- **Every Web Push must show a notification.** Chrome allows about 6 silent pushes a day, and Firefox and Safari drop the subscription after 16 and 3. So wake-ups are used only where they always lead to a notification.
- **Time-based reminders without conditions:**
  - Devices tell the server the due times.
  - The server sends a content-free Web Push at each one.
  - The service worker decrypts the reminder from IndexedDB and shows it.
  - The server learns due times, which it already sees as timing.
- **Reminders with conditions, webhooks and other sensors** alert only while the web client is open. When it's closed, they're left to devices that can sense them.
- **No alarms in a browser.** High and Maximum become notifications that stay until dealt with, with Done and Snooze. Browsers allow two action buttons, so the third action needs the app open. The web client says plainly that only the apps ring.
- **The app badge** follows the tray badge's rule: Low and above, red only when something Medium or above is overdue.

## Sources

| Source class | In the web client |
|---|---|
| Time (schedules, countdowns, sun) | Yes |
| Weather, places, webhooks | Only while the web client is open |
| Bluetooth, Wi-Fi, USB | Never |
| Calendar | Not for now. Occurrences that other devices fire from calendars still appear. |

A device that can't sense a trigger leaves it to devices that can.

## Layout

- **Small screens** copy the [Android app](android.md): bottom navigation with five views, bottom sheets, and a full-screen editor.
- **Large screens** copy the [desktop window](desktop.md) and its dialogs.
- **Not available in a browser:** the tray, the Linux alarm window, start at login, and the app's own check of the system's Do Not Disturb.
- **Still works:** "Quiet this device" and the loudest-style cap. It can be installed as a PWA where the browser supports it.

## Browsers

- **Tested:** Chromium and Firefox, on Linux and Android.
- **Best effort:** every other current browser, including on iOS and Windows, which the web client happens to reach. Native apps for them are out of scope.

## Decided in

- [Web client][31]
- [Security hardening before the server goes on the internet][33]
- [ADR 0007](../adr/0007-web-client-from-a-static-host-with-a-browser-trusted-server-certificate.md)
- **Research:** [web client browser constraints](../research/web-client-browser-constraints.md)

[31]: https://github.com/csnook/hab-bot/issues/31
[33]: https://github.com/csnook/hab-bot/issues/33
