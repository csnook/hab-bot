# Push goes through UnifiedPush, with its embedded FCM distributor on phones with Google

Phones are woken by content-free pushes sent as standard Web Push (RFC 8030, encrypted per RFC 8291), signed with a VAPID key that each server generates for itself. On phones with Google Play Services, UnifiedPush's embedded FCM distributor, a library inside our app, registers that key with Play services and delivers through Google's servers, so no extra app is needed. On phones without Google, any UnifiedPush distributor the user installs, such as ntfy, receives the same pushes. We chose this over Google's Firebase library so that one server protocol serves every phone, with no Firebase project, no secret that every server would need, and no Firebase code in the app. That also suits a later public product where others run their own servers. We accept that Google doesn't document delivering Web Push this way.

## Considered Options

- **FCM through the Firebase library**: the documented route, but it needs our Firebase project, whose secret key every server would need (or a gateway we run), and Firebase inside the app, so a Google-free build would need a second variant of the app.
- **Our own foreground service holding a connection**: needs a permanent notification and a special declaration to Google Play, and reaches the phone only where the server does (at home or over a VPN).
- **A self-hosted ntfy server as the only route**: every phone needs the ntfy app, and it reaches no further than our own connection would.
- **No push at all**: periodic sync, the check before every firing, and live connections while ringing. This stays the fallback when a phone has no distributor.

## Consequences

- An early prototype must confirm that an urgent push through the embedded FCM distributor wakes a phone in Doze. If it doesn't, or if Google closes the route, we fall back to FCM through the Firebase library.
- Only pushes that should start a new alert are sent as urgent, since Android demotes apps whose urgent pushes don't lead to a notification. The sending device marks those events as urgent in their unencrypted header.
- Push servers (Google, or whichever server a distributor uses) see when pushes are sent and how big they are, but nothing inside them.
- No Tauri plugin supports this on Android, so we write the Kotlin push service that calls into Rust.
- Desktops keep their own connection to the server and don't use push.
