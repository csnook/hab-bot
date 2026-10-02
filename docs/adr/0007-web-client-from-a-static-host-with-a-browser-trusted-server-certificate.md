# The web client loads from a static host, and syncs only with a server that has a browser-trusted certificate

The web client is a static bundle (the Rust core compiled to WebAssembly and the shared Preact UI) that we publish to a static host, GitHub Pages for now, from tagged releases. The sync server never serves it. The code a browser runs handles the user's password and keys, so if the server served it, a dishonest server admin could change it and read everything the encryption is there to hide (ADR 0002, ADR 0004). From a separate host, the trust is the same as for the apps: users trust whoever publishes the code, not whoever runs their server. To sync, a browser must also trust the server's certificate, and browsers can't pin a self-signed one as the apps do. So a server that browsers sync with gets a Let's Encrypt certificate for a real domain through the DNS-01 check, which works without the server being reachable from the internet. We accept that this costs a domain, that the server's name is published in certificate logs, and that whoever controls the static host can change the code.

## Considered Options

- **Serving the web client from the sync server**: the simplest to deploy, and always the server's version, but it puts the code that handles passwords and keys in the hands of the server admin.
- **The self-signed certificate the apps pin, clicked through in the browser**: browsers can't pin a certificate, Chrome's service workers reject one even after a click-through, and it teaches users to click through warnings.
- **A household certificate authority installed on each device**: no domain and nothing published, but every device needs it installed by hand, Firefox for Android ignores it unless a hidden setting is on, and an authority installed that way can vouch for any website.
- **WebTransport with certificate hashes**: lets a page trust a self-signed certificate by its hash, but only certificates valid for two weeks or less, over a WebTransport server, and not for ordinary fetches or WebSockets.
- **Let's Encrypt's HTTP-01 or TLS-ALPN-01 checks**: need the server reachable from the internet, which a home server isn't until security hardening.

## Consequences

- Whoever controls the static host could replace the service worker, which browsers re-check at least daily. Subresource Integrity pins the scripts and WebAssembly but not the page or the worker. To make a quiet swap noticeable, each release lists the hashes of `index.html` and the service worker, and About shows the running version and its hash. The publisher's account uses two-factor sign-in and protected release tags.
- Asset names carry hashes and have Subresource Integrity, so caching stays correct without custom headers, which GitHub Pages can't set. Only `index.html` and the service worker change between versions, and an update waits for "Update available · Reload".
- Invite links put the server's address after the `#`, so the static host never learns which server a user has.
- Anyone can publish the same bundle on their own static host, and a host with custom headers or a CDN can replace GitHub Pages later.
- Without a browser-trusted certificate, the web client works only standalone.
- The server reads the certificate and key from files, whatever issued them, and reloads them when they change. Tailscale can issue one for a machine's `ts.net` name, and a DNS-01 tool such as certbot for a domain of your own. Getting it is automated later, only where an API client fits easily into the Rust server, and set up by hand until then.
- The apps keep pinning the certificate from the invite, and also accept the trusted one.
- Some routers drop DNS answers that point a public name to a private address, and need a local override.
- Chrome and desktop Firefox ask before a public page reaches a local address, so the web client explains that prompt before it appears.
