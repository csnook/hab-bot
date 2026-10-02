# Push messages to Android, with and without Google

Research for [csnook/hab-bot#25](https://github.com/csnook/hab-bot/issues/25), the part that asks how content-free push messages ("wake up and sync") reach phones with and without Google's push service. It covers FCM sent from our own server, UnifiedPush and its distributors, ntfy, holding our own connection with no push service, what Tauri 2 plugins offer, and Linux desktops briefly.

- **Read on:** 2026-09-30. Versions and dates of what was read are in [Access notes](#access-notes).
- **Taken as given, not researched again:** push messages carry no content; the device then fetches and decrypts changes itself. The server is a Rust program on a home server with outbound internet access, reachable from phones only at home or over a VPN for now. Phones use Google Play Services (GMS) where present and must still work without it. A device ringing an alarm already holds a live connection in its foreground service. Other Android notifications may catch up at the next check or sync. Doze, WorkManager, exact alarms and the headless-Rust problem are in [android-background-tauri.md](android-background-tauri.md) and are not repeated here.
- **Terminology:** this document uses the terms in `CONTEXT.md`. Outside words used here:
  - **Push message** means the content-free wake-up our server sends. It is not an *alert*.
  - **Android notification** means the OS object posted through `NotificationManager`, as in the background note.
  - **App server** means our own Rust server. UnifiedPush calls it the "application server".
  - **Push server** means a third-party relay that holds the connection to the phone: FCM, an ntfy server, Mozilla's autopush.
  - **Distributor**, **connector** and **endpoint** are used as UnifiedPush defines them: the app on the phone that receives from a push server, the library inside our app that talks to it, and the URL our app server posts to ([definitions.md](https://github.com/UnifiedPush/specifications/blob/eaa0fc50c3dd37c4937da1161abeaf2d50f96760/definitions.md)).
- **Unverified claims:** the proxy blocked `firebase.google.com`, `unifiedpush.org`, `codeberg.org`, `docs.ntfy.sh`, `support.google.com` and the RFC hosts, among others (full list in [Access notes](#access-notes)). Firebase and Google Play pages could not be read at all. UnifiedPush pages and specs were read from their GitHub mirrors, which stopped updating in mid-2025, and from the current library sources on Maven Central. Claims that rest only on a search-engine excerpt are marked **[unverified: search excerpt]**. Claims drawn from reading code, with no documentation behind them, are marked **[inference from source]**.
- This file surfaces facts and trade-offs. It does not choose an option.

## Summary

- **FCM from our own server works with outbound access only.** The server needs a Firebase project and a service-account key. It mints OAuth 2.0 access tokens with the `firebase.messaging` scope and POSTs to `https://fcm.googleapis.com/v1/projects/{project}/messages:send`. The API now addresses a device by its Firebase Installation ID (`fid`), which the Android SDK adopted in 25.1.0 (June 2026). The older `token` field is deprecated but still accepted. The app needs `firebase-messaging` (latest released 25.1.3), the Firebase config values (usually from `google-services.json`), and a Kotlin `FirebaseMessagingService`. The SDK only works on phones with Play services.
- **High priority buys Doze delivery, with conditions.** A high-priority message is delivered at once in Doze. The app gets temporary network access and partial wake locks, and may start a foreground service. Since Android 13 the system downgrades an app's high-priority messages if they consistently don't lead to an Android notification. Google names "initiating data syncs" as a normal-priority use. Normal priority waits for a Doze maintenance window. A force-stopped app receives nothing.
- **FCM's trade-off for self-hosting.** Every server that sends to our app needs our Firebase project's secret credentials. The UnifiedPush project describes how decentralized apps therefore run a central gateway that holds the Google key.
- **UnifiedPush is Web Push to a push server the user picks.** The app registers with a distributor through the connector library (Kotlin, Apache-2.0, 3.3.5 of 2026-08-22). It receives an endpoint URL and a public key set, and sends both to our app server. Our server sends a standard Web Push request (RFC 8030) with the payload encrypted per RFC 8291. Since the AND_3 spec (connector 3.0, December 2024), that encryption is a MUST. A VAPID key (RFC 8292) SHOULD be registered, and some distributors require one. Messages are at most 4096 bytes. The connector decrypts messages itself.
- **The embedded FCM distributor covers GMS phones with no extra app and no Firebase project** [inference from source]. Since 3.0.0 (January 2025) it has no Firebase dependency. It registers with Play services directly, using our server's VAPID public key, and yields an endpoint at `https://fcm.googleapis.com/fcm/send/<token>`. Our server then sends ordinary Web Push with VAPID to Google. The UnifiedPush project says Google does not document this. One server-side protocol (Web Push) would then serve both GMS phones and phones with a distributor. Whether a Web Push `Urgency: high` becomes a high-priority FCM message was not verified.
- **Every other distributor is a separate app that holds its own connection.** ntfy (self-hostable, WebSocket), Sunup (Mozilla's autopush servers by default, self-hostable), NextPush (Nextcloud), Conversations (XMPP) and gCompat-UP (FCM, "mainly for testing") all need installing. They keep a foreground service running and ask for a battery-optimization exemption. ntfy's author reports "about 0-1% of battery in 17h".
- **ntfy** is a Go server under Apache-2.0 or GPLv2 (server v2.28.0, 2026-08-27; Android v1.25.2, 2026-07-23). Its Android app uses FCM only for the public `ntfy.sh` host and only in the Google Play build. Any self-hosted server always gets "instant delivery": a `specialUse` foreground service with a WebSocket and a 3-minute ping. An app server publishes with a plain HTTP POST or PUT to the topic URL. UnifiedPush topics start with `up`.
- **Holding our own connection with no push service is possible but discouraged by Google.** A process with a foreground service has unrestricted network access, including in Doze. The fitting foreground-service types each have a catch:
  - `dataSync` is capped at 6 hours per 24 on Android 15+ and can't start from `BOOT_COMPLETED`.
  - `specialUse` needs a free-form justification reviewed in Play Console.
  - `remoteMessaging` is described as transferring text messages between devices.
  - `systemExempted` is allowed for apps holding an exact-alarm permission.

  Google "strongly recommend[s]" FCM over our own persistent connection. Its battery-exemption table accepts apps that "can't use FCM because of technical dependency on another messaging service". The Play policy pages themselves were not readable.
- **No Tauri plugin delivers a push to Rust when the app is closed.** None of the four FCM plugins inspected passes a message on unless its plugin instance exists, and that instance exists only once the Activity has run. One only logs messages, and one queues them in memory until the plugin loads. In `tauri-plugin-notifications` the Firebase dependency is not optional on Android, although its README says it is. Its UnifiedPush support is Linux-only. No Android UnifiedPush plugin for Tauri was found. Either path needs our own Kotlin service that calls into Rust.
- **Linux has a UnifiedPush D-Bus protocol.** KDE's KUnifiedPush is a distributor and client library, and the official `unifiedpush` Rust crate (0.1.0) implements the connector side. A distributor must be installed and running. The design already has desktops keep their own live connection.

### At a glance

"Away from home" assumes the app server stays reachable only at home or over a VPN. In every row, the sync that follows a push still needs a route to the app server.

| Path | On the phone | On the app server | Without GMS? | Push reaches a phone away from home? | Doze |
|---|---|---|---|---|---|
| FCM HTTP v1 | `firebase-messaging` + Firebase config + Kotlin service | Firebase project, service-account key, OAuth token, outbound HTTPS to Google | No | Yes | High priority delivered at once; may be downgraded if no Android notification follows |
| UnifiedPush, embedded FCM distributor | Connector + embedded distributor library; nothing to install | Web Push + RFC 8291 encryption + VAPID key; outbound HTTPS to Google | No (it steps aside when GMS is missing) | Yes | Depends on how FCM treats Web Push urgency (not verified) |
| UnifiedPush, public push server (ntfy.sh, Sunup/Mozilla) | Connector + a distributor app the user installs | Web Push as above; outbound HTTPS to the push server | Yes | Yes | Distributor holds a foreground-service connection |
| UnifiedPush, ntfy self-hosted at home | Connector + ntfy app pointed at our ntfy | ntfy server next to the app server | Yes | Only over VPN | As above |
| Own connection, no push service | Our own foreground service | A long-lived endpoint (WebSocket or similar) | Yes | Only over VPN | Foreground service keeps network access |
| No push, periodic sync only | WorkManager job | Nothing extra | Yes | Only over VPN | Deferred to maintenance windows and standby quotas |

## 1. FCM sent from a self-hosted server

### What the server needs

- **Endpoint and scopes.** Google's discovery document for FCM v1 (revision 20260925) lists one send method: `POST v1/projects/{projectsId}/messages:send` on `https://fcm.googleapis.com/`. It accepts the OAuth scopes `https://www.googleapis.com/auth/firebase.messaging` or `https://www.googleapis.com/auth/cloud-platform` ([discovery document](https://fcm.googleapis.com/$discovery/rest?version=v1)). The official Go Admin SDK builds the same URL from `defaultMessagingEndpoint = "https://fcm.googleapis.com/v1"` ([firebase-admin-go `messaging.go`](https://github.com/firebase/firebase-admin-go/blob/dev/messaging/messaging.go)).
- **Credentials.** A service-account JSON key from the Firebase project is used to mint short-lived OAuth 2.0 access tokens, sent as `Authorization: Bearer`. The legacy server-key API was removed in June 2024 **[unverified: search excerpt]**. ntfy's docs describe the same setup from the operator's side: create a Firebase app, download its key file, and set `firebase-key-file` ([ntfy `config.md` L1571-L1596](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/config.md#L1571-L1596)).
- **Addressing a device.** The `Message` schema now has `fid`, a "Firebase Installation ID (FID) to send a message to". `token` is marked "Deprecated: Use `fid` instead. During the transition period, this field also accepts a Firebase Installation ID" ([discovery document](https://fcm.googleapis.com/$discovery/rest?version=v1)).
- **Only outbound traffic.** Nothing in the send path needs Google to reach our server **[inference from source]**. The home server needs outbound HTTPS to `fcm.googleapis.com` and Google's OAuth token endpoint.
- **Who may send.** Only holders of the Firebase project's credentials can send to our app. The UnifiedPush project's January 2025 post says this is why apps for self-hostable services (Element, Mastodon, Nextcloud) run a central gateway that holds the key and relays for every server ([UnifiedPush news, 2025-01-31](https://github.com/UnifiedPush/documentation/blob/8c61ba91f4a80231e205f6e0b2b712b591a7ca4b/content/news/20250131_push_for_decentralized.md)). For one household this is our own key. For a later public product where users run their own servers, each server would need that key, or we would run a gateway.

### Rust crates for sending FCM

Maintenance is judged from crates.io release dates only. GitHub issue pages were not reachable.

| Crate | Latest | Released | Licence | Notes |
|---|---|---|---|---|
| `firebae-cm` | 0.5.0 | 2026-08-05 | MIT | HTTP v1 message builder. Has `Receiver::Fid` and documents `Token` as deprecated. Optional `oauth` feature uses `gcp_auth`. 9.6k downloads. |
| `google-fcm1` | 7.0.0+20251212 | 2026-01-01 | MIT | Generated from Google's discovery document by google-apis-rs. Its `Message` has `token` but no `fid`. 243k downloads. |
| `fcm-service` | 0.2.3 | 2025-06-03 | MIT | Service-account auth through `gcp_auth`. 63k downloads. |
| `oauth_fcm` | 0.3.0 | 2024-12-15 | MIT | 4.5k downloads. |
| `fcm-rs` | 0.2.0 | 2024-07-20 | MIT | |
| `fcm_v1` | 0.3.0 | 2023-04-22 | MIT | |
| `firebase-admin` | 0.3.0 | 2026-07-08 | MIT OR Apache-2.0 | New (first release 2026-07-04), 581 downloads. |
| `fcm` | 0.9.2 | 2022-07-27 | MIT | Posts to `https://fcm.googleapis.com/fcm/send` with `key=`, the removed legacy API (`src/client/mod.rs` L37-L40 in the crate). |

Token minting is available from `gcp_auth` 0.12.7 (2026-06-22, MIT, 14M downloads) and `yup-oauth2` 12.1.2 (2026-01-07, MIT OR Apache-2.0) ([crates.io API](https://crates.io/api/v1/crates/gcp_auth)). The FCM send call itself is a single JSON POST, so a crate is optional.

### What the Android app needs

- **The SDK.** `firebase-messaging` 25.1.3 is the latest released version recorded in the SDK repository (mergeback of 2026-09-11). Recent changes ([CHANGELOG](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/CHANGELOG.md); [gradle.properties](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/gradle.properties)):
  - 25.0.0 raised the minimum SDK to 23 and stopped releasing the KTX module.
  - 25.1.0 (recorded in a 2026-06-19 mergeback) "Added support for FCM registration using Firebase Installation ID". `getToken`, `deleteToken` and `onNewToken` are deprecated. The replacements are `register()` and `onRegistered()`, switched on with the manifest flag `firebase_messaging_installation_id_enabled` (`FirebaseMessaging.java` and `FirebaseMessagingService.java` at the same commit).
  - The unreleased 26.0.0 raises the minimum SDK to 24.
- **The config.** Firebase reads its options (`google_app_id`, `gcm_defaultSenderId` and others) from Android resources (`FirebaseOptions.fromResource`). The Google Services Gradle plugin generates these resources from `google-services.json`. ntfy, for example, applies `com.google.gms:google-services:4.4.4` for its Play build only ([ntfy-android `build.gradle`](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/build.gradle)). `FirebaseOptions.Builder` also has setters for these values, so the file looks like a convenience rather than a requirement **[inference from source]** ([`FirebaseOptions.java`](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-common/src/main/java/com/google/firebase/FirebaseOptions.java)). Without config, the init provider only logs "FirebaseApp initialization unsuccessful" ([`FirebaseInitProvider.java` L69-L72](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-common/src/main/java/com/google/firebase/provider/FirebaseInitProvider.java#L69-L72)).
- **A Kotlin service.** The app extends `FirebaseMessagingService` and declares it with the `com.google.firebase.MESSAGING_EVENT` action. Its methods run on a background thread and "may be called when the app is in the background or not open". `onMessageReceived` "should complete within 20 seconds". `onDeletedMessages` is called when FCM dropped pending messages, and "It is recommended that the app do a full sync" ([`FirebaseMessagingService.java` L34-L125](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/src/main/java/com/google/firebase/messaging/FirebaseMessagingService.java#L34-L125)). Firebase's docs give 10 seconds instead, and suggest a WorkManager job for longer work **[unverified: search excerpt]**.
- **Play services on the phone.** The SDK talks to the `com.google.android.gms` package over its c2dm IPC ([`Metadata.java`](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/src/main/java/com/google/firebase/messaging/Metadata.java); [`GmsRpc.java`](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/src/main/java/com/google/firebase/messaging/GmsRpc.java)). On GrapheneOS, Google Play is an optional install inside the normal app sandbox. Users "should give a battery optimization exception to Google Play services for features like push notifications to work properly in the background" ([grapheneos.org `usage.html`](https://github.com/GrapheneOS/grapheneos.org/blob/main/static/usage.html), "Sandboxed Google Play"). microG was not researched.
- **Two builds is a common pattern.** ntfy ships a `play` flavor with `firebase-messaging` 25.0.1 and an `fdroid` flavor without it, switched by a `FIREBASE_AVAILABLE` build flag ([ntfy-android `app/build.gradle` L53-L61, L111-L112](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/app/build.gradle)).

### Data versus notification messages, and priority

- **Where each kind goes.** In the SDK's dispatch code, a notification message is displayed by the SDK itself unless the app is in the foreground. Only then does it reach `onMessageReceived`. A data message always reaches `onMessageReceived` ([`FirebaseMessagingService.java` L286-L306](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/src/main/java/com/google/firebase/messaging/FirebaseMessagingService.java#L286-L306)). A content-free wake-up is therefore a data message.
- **Priority, in the API's own words** ([discovery document](https://fcm.googleapis.com/$discovery/rest?version=v1), `AndroidConfig.priority`):
  - NORMAL is the "Default priority for data messages. Normal priority messages won't open network connections on a sleeping device, and their delivery may be delayed to conserve the battery. For less time-sensitive messages, such as notifications of new email or other data to sync, choose normal delivery priority."
  - HIGH is the "Default priority for notification messages … allowing the FCM service to wake a sleeping device when possible and open a network connection to your app server … Set high priority if the message is time-critical and requires the user's immediate interaction".
- **What high priority buys in Doze** ([Doze and App Standby](https://developer.android.com/training/monitoring-device-state/doze-standby)):
  - "In Doze or App Standby mode, the system delivers the message and gives the app temporary access to network services and partial wakelocks, then returns the device or app to the idle state."
  - "For messages that don't result in notifications, such as keeping app content up to date in the background or initiating data syncs, use normal priority FCM messages … If the device is in Doze mode, they are delivered during the periodic Doze maintenance windows or as soon as the user wakes the device."
  - The Doze checklist says to "Only use high priority for messages that result in a notification", and to "Provide sufficient information within the initial message payload, so subsequent network access is unnecessary". A content-free push followed by a fetch goes against the second point.
- **Downgrades.** The power-limits page says that during Doze, high priority has "No execution limits" and normal priority is "Deferred to doze maintenance window". Since Android 13, "App Standby Buckets no longer determine how many high priority FCMs an app can use. System now downgrades the high priority messages if it detects an app consistently sending high-priority messages that don't result in a notification" ([Power management resource limits](https://developer.android.com/topic/performance/power/power-details)). On 12L and lower, such misuse "can cause future messages to be deprioritized" ([App Standby Buckets](https://developer.android.com/topic/performance/appstandby)). `RemoteMessage.getPriority()` returns the priority "as delivered. This may be lower than the priority originally requested", and `getOriginalPriority()` returns what was sent ([`RemoteMessage.java` L226-L253](https://github.com/firebase/firebase-android-sdk/blob/09ee73c86d802a46473e12a4abc3b3ebb2e16224/firebase-messaging/src/main/java/com/google/firebase/messaging/RemoteMessage.java#L226-L253)).
- **Starting a foreground service.** Receiving a high-priority FCM message is an exemption from the background start restrictions. If the message was downgraded, starting one throws `ForegroundServiceStartNotAllowedException`, so the docs advise checking `getPriority()` first ([Restrictions on starting a foreground service from the background](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)). Firebase's docs also describe a "brief exemption for expedited jobs scheduled immediately after a high priority FCM onMessageReceived" **[unverified: search excerpt]**.
- **Storage and collapsing.** `ttl` is how long FCM keeps a message for an offline device: at most and by default 4 weeks, and `0` means deliver now or drop. At most 4 collapse keys may be in use at a time ([discovery document](https://fcm.googleapis.com/$discovery/rest?version=v1)). A `directBootOk` flag allows delivery before the first unlock.
- **Size and rate limits.** The payload limit is 4096 bytes **[unverified: search excerpt]**. The project quota is 600,000 messages per minute. Per Android device, the limit is 240 messages per minute and 5,000 per hour. Collapsible messages allow a burst of 20 per device, refilled at one every 3 minutes **[unverified: search excerpt of firebase.google.com/docs/cloud-messaging/throttling-and-quotas]**. A content-free push is only a few bytes.
- **Force-stopped apps get nothing.**
  - Since Android 3.1, "the system adds FLAG_EXCLUDE_STOPPED_PACKAGES to all broadcast intents", and apps are stopped "when they are first installed but are not yet launched and when they are manually stopped by the user" ([Android 3.1 platform notes](https://developer.android.com/about/versions/android-3.1)).
  - Android 15 also cancels the app's pending intents while it is stopped ([Android 15 changes, all apps](https://developer.android.com/about/versions/15/behavior-changes-all)).
  - Firebase's troubleshooter says messages to a force-stopped app are dropped **[unverified: search excerpt]**.
  - The same broadcast rule would block UnifiedPush delivery, which also arrives as a broadcast **[inference from source]**.

### Cost and terms

- FCM has no per-message fee, including for commercial use **[unverified: search excerpt of firebase.google.com/pricing]**. The Firebase and Google APIs terms were not readable. Neither were any data-processing terms that might matter for a public product.

## 2. UnifiedPush

### The spec

- **Versions.** The Android spec on the GitHub mirror is "UnifiedPush Spec: AND_3.0.0" ([specifications `android.md` L3](https://github.com/UnifiedPush/specifications/blob/eaa0fc50c3dd37c4937da1161abeaf2d50f96760/specifications/android.md)). AND_3 was merged on 2024-10-23. The mirror's last commit is 2025-05-06. The live site describes AND_3.1.0, which adds a `TEMP_UNAVAILABLE` action for when "the distributor backend is temporary unavailable" **[unverified: search excerpt of unifiedpush.org/developers/spec/android/]**. Connector 3.3.5 has matching code: an `ACTION_TEMP_UNAVAILABLE` constant and a `PushService.onTempUnavailable` callback (sources jar).
- **What changed at AND_3.** The earlier AND_2.0.0-beta1 required only that the message be "the raw POST data received by the rewrite proxy". AND_3 (`android.md` L34-L45) requires:
  - **Encryption.** "The message MUST be an encrypted content that follows [RFC8291]. Its size is between 1 and 4096 bytes (inclusive). It means the cleartext content is at most 3993 bytes."
  - **VAPID.** The registration "SHOULD" carry a VAPID public key, a change made on 2025-03-25. "Some push servers require the application server to authenticate with VAPID". Their distributors answer `REGISTRATION_FAILED` with reason `VAPID_REQUIRED`.
  - **Endpoint.** The endpoint is an RFC 8030 push-resource URL of at most 1000 bytes. It is a capability URL, and 160 random bits are recommended.
  - **Identity.** On SDK 34+, register and unregister broadcasts use `FLAG_SHARE_IDENTITY` so the distributor knows the calling app (L138, L170).
- **Lifecycle.** The spec expects the app to re-register "every time the application starts". A distributor pings registrations unused for weeks. It unregisters any that haven't acknowledged a message or ping in 30 days (L47-L85, L319).
- **Urgency.** The distributor "SHOULD follow the push message urgency as defined in [RFC8030], section 5.3". With no header, urgency is normal. Only messages above the device state's minimum urgency are passed on (L291-L298):

  | Urgency | Minimum when the device is… |
  |---|---|
  | very-low | on power and Wi-Fi |
  | low | on either power or Wi-Fi |
  | normal | on neither power nor Wi-Fi |
  | high | on low battery |

- **Waking the app.** The distributor "MUST raise the application to the foreground importance for 5 seconds" when it delivers a message. It does this by binding to a service the app exposes, which "allows the application to start a foreground service from the background without unrestricted battery usage rights" (L300-L305, L321-L327).
- **The RFCs themselves** (RFC 8030, 8291, 8292) were not readable here. Claims about them come from the UnifiedPush spec and from library code.

### The Android connector library

- **Artifacts on Maven Central** ([`org.unifiedpush.android`](https://repo1.maven.org/maven2/org/unifiedpush/android/)):

  | Artifact | Latest | Released | Licence | Depends on |
  |---|---|---|---|---|
  | `connector` | 3.3.5 | 2026-08-22 | Apache-2.0 | `kotlin-stdlib` 2.2.10, Google `tink` 1.23.0 (Web Push decryption); minSdk 16 |
  | `connector-ui` | 1.1.0 | 2024-12-19 | Apache-2.0 | distributor-picker dialog |
  | `embedded-fcm-distributor` | 3.1.0 | 2026-06-23 | Apache-2.0 | `kotlin-stdlib`, `kotlinx-coroutines`; no Firebase |
  | `distributor` / `distributor-base` | 0.7.6 | 2026-08-26 | | for building distributors |

  Connector history: 3.0.0 on 2024-12-19 (the first AND_3 release), 3.1.0 on 2025-09-30, 3.2.0 on 2026-01-05, 3.3.0 on 2026-02-16, and five patch releases since. The source lives on Codeberg (POM `scm`). The GitHub mirror stops at 3.0.10 (2025-06-12).
- **Language.** It is an Android library (AAR) written in Kotlin, and its docs show both Kotlin and Java usage. Other connectors wrap it for Flutter, React Native (`react-native-unifiedpush-connector` 0.4.0) and Expo (`expo-unified-push` 0.6.0, 2026-07-01) (npm registry). No Rust or Tauri connector exists for Android.
- **How the app registers** (connector 3.3.5 sources, `UnifiedPush.kt`):
  - Pick a distributor with `tryUseCurrentOrDefaultDistributor`, which uses the deep link `unifiedpush://link` and the OS app chooser. Or list them with `getDistributors` and save one with `saveDistributor`.
  - Call `register(context, instance, messageForDistributor, vapid)`. `vapid` is the app server's P-256 public key, base64url, 87 characters.
  - Re-register regularly, for example at every app start.
- **What the app receives** (`PushService.kt`, `data/*.kt`). The app declares a non-exported `PushService` subclass with the `org.unifiedpush.android.connector.PUSH_EVENT` action. The connector binds to it and calls:
  - `onNewEndpoint(PushEndpoint(url, pubKeySet(pubKey, auth), temporary))`. `pubKey` and `auth` are the RFC 8291 keys our app server needs. `temporary` marks a fallback distributor used while the primary is down.
  - `onMessage(PushMessage(content, decrypted))`, with the content already decrypted. If decryption fails, the connector passes the raw bytes with `decrypted = false` ("Could not decrypt message, trying with plain text", `MessagingReceiver.kt`).
  - `onRegistrationFailed`, `onUnregistered`, `onTempUnavailable`.

  The connector holds a 10-second partial wake lock while it handles a message (`internal/WakeLock.kt`).
- **What the app server sends** [inference from source and spec]. The app forwards the endpoint URL, `p256dh` and `auth` to our server. The server encrypts the payload (`aes128gcm`, RFC 8291) to those keys. It signs a VAPID JWT with its private key, the pair to the public key the app registered. It POSTs to the endpoint with `TTL`, optionally `Urgency`, and `Content-Encoding: aes128gcm` headers.

### The embedded FCM distributor

- **No Firebase SDK or project.** Version 2.5.0 (2024-08-29) depended on `firebase-messaging` 24.0.0 and `play-services-base`. Since 3.0.0 (2025-01-31) its only dependencies are Kotlin libraries (POMs on Maven Central). The 3.1.0 sources register directly with Play services (`com.google.android.gms`) over the `com.google.android.c2dm` IPC. They pass the VAPID public key as the "sender" and get back a token. The endpoint becomes `https://fcm.googleapis.com/fcm/send/%s` (`Constants.kt`, `c2m/C2mRequests.kt`, `impl/FirebaseReceiver.kt`). No `google-services.json`, Firebase project or service-account key appears anywhere in the library **[inference from source]**.
- **What the server does.** The library's own docs say: "Google FCM servers can handle webpush requests out of the box, but they require a VAPID authorization. If your application supports VAPID, you have nothing special to do" (`EmbeddedDistributorReceiver.kt`). An app that doesn't register a VAPID key must go through a gateway that adds one. The default is `https://fcm.distributor.unifiedpush.org/wpfcm`, commented "Please host your own gateway if possible" (`DefaultGateway.kt`).
- **Google's side is undocumented.** UnifiedPush's post says: "What is less known, and not (well?) documented by Google, is that you can directly send webpush requests to FCM servers." It also says that registering "a VAPID pubkey retrieved during execution is not possible with the firebase-messaging library" ([news post](https://github.com/UnifiedPush/documentation/blob/8c61ba91f4a80231e205f6e0b2b712b591a7ca4b/content/news/20250131_push_for_decentralized.md)).
- **When it is used.** The embedded distributor answers `REGISTRATION_FAILED` with `ACTION_REQUIRED` when Play services are missing. `getDistributors` then leaves the app's own package out of the list. "External distributors will be favored over embedded distributors" (`UnifiedPush.kt`). So a GMS phone with no distributor uses FCM, and a phone with a distributor uses that distributor.
- **Priority is unclear.** UnifiedPush's `common-proxies` gateway forwards the Web Push `Urgency` header to `fcm.googleapis.com/fcm/send` unchanged, defaulting to `normal` ([`rewrite/wp_fcm.go` L85-L110](https://github.com/UnifiedPush/common-proxies/blob/3a62b28c0597b93e1d1f31dcde5bac0d2fb1f85b/rewrite/wp_fcm.go#L85-L110)). No first-party source was found that says how FCM maps Web Push urgency to Android priority. Nor whether the "no notification" downgrade applies to these registrations.

### Distributors

The mirrored quickstart pages ([distributors](https://github.com/UnifiedPush/documentation/tree/8c61ba91f4a80231e205f6e0b2b712b591a7ca4b/content/users/distributors)) plus F-Droid metadata ([fdroiddata](https://gitlab.com/fdroid/fdroiddata/-/tree/master/metadata)):

| Distributor | Push server | Self-hostable? | Licence | Transport | Current Android version |
|---|---|---|---|---|---|
| ntfy | `ntfy.sh` by default, or our own ntfy | Yes: binary, packages or Docker | Apache-2.0 (app); server Apache-2.0 OR GPLv2 | WebSocket (default) or HTTP JSON stream | 1.25.2 |
| Sunup | Mozilla's autopush (`push.services.mozilla.com`) by default | Yes: `autopush-rs` (Rust, MPL-2.0; Bigtable or Postgres storage features) | Apache-2.0 | WebSocket | 1.3.3 |
| NextPush | The `uppush` Nextcloud app | Yes, on Nextcloud | AGPL-3.0-only | Server-Sent Events | 2.4.2 |
| Conversations | Any XMPP server plus a "rewrite proxy" (`up.conversations.im`, or self-hosted `up` or Prosody `mod_unified_push`) | Yes | GPL-3.0-only | XMPP | 2.20.4 |
| gCompat-UP | Google FCM | No | Apache-2.0 | FCM | "mainly design[ed] for testing purposes" |
| Gotify-UP | Gotify | Yes | MIT | WebSocket | Deprecated: "does not support new versions of the UnifiedPush protocol" |
| NoProvider2Push | None; needs a static address on the phone (VPN, Yggdrasil) | n/a | Apache-2.0 | HTTP listener on the phone | "niche … mostly useful for development purposes" |
| Embedded FCM | Google FCM | No | Apache-2.0 | FCM | Library inside our app |

- **No extra app** is needed only with the embedded FCM distributor, and only on phones with Play services. Every other distributor is an app the user installs and sets up.
- **VAPID requirements differ.** ntfy's Android app has `VAPID_REQUIRED, // Currently unused` ([`up/FailedReason.kt` L22](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/app/src/main/java/io/heckel/ntfy/up/FailedReason.kt#L22)). Mozilla's instance requires VAPID **[unverified: search excerpt]**. So does the embedded FCM distributor, unless a gateway is used.
- **Metadata.** A push server sees the endpoint, the time and size of each message, and which device receives it. The payload is encrypted to the app **[inference from source]**. This holds for Google (FCM, embedded FCM), ntfy.sh and Mozilla alike.

### How distributors behave in Doze

- **A foreground service with its own connection.** Every quickstart page tells users to grant "battery optimization exemptions to ensure it runs properly in the background". UnifiedPush's shared `distributor` library 0.7.6 declares `FOREGROUND_SERVICE_SPECIAL_USE`, `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`, `WAKE_LOCK` and `RECEIVE_BOOT_COMPLETED`, and provides a `ForegroundService` base class (AAR manifest and sources jar). ntfy's design is in §3.
- **Why this works in Doze.** A process running a foreground service has "No restrictions" on network access ([Power management resource limits](https://developer.android.com/topic/performance/power/power-details), "Resource limits based on app state").
- **Battery.** ntfy's FAQ: with its own server or instant delivery, "the app has to maintain a constant connection to the server, which consumes about 0-1% of battery in 17h of use (on my phone)" ([ntfy `faq.md` L43-L48](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/faq.md#L43-L48)). No independent measurement was found. The UnifiedPush FAQ says only that UnifiedPush "avoids the need for apps to run background services".
- **Our app's side.** Our app is woken by a broadcast and raised to foreground importance for 5 seconds. Within that window it can start its own foreground service to fetch from the app server **[inference from source]**. Whether it has network access without doing so, while the device is in Doze, was not established.

### Rust crates for sending Web Push

| Crate | Latest | Released | Licence | Notes |
|---|---|---|---|---|
| `web-push` | 0.11.0 | 2025-02-22 | Apache-2.0 | Complete client: RFC 8188/8291 encryption through Mozilla's `ece`, VAPID signing, an `Urgency` enum, isahc or hyper HTTP clients. Needs OpenSSL. "tested with Google's and Mozilla's push notification services". 913k downloads ([README](https://github.com/pimeys/rust-web-push/blob/8de73e2e6d3e56786561924dd132a76ea18ae042/README.md)). |
| `web-push-native` | 0.5.0 | 2026-07-26 | MIT OR Apache-2.0 | RustCrypto primitives, no OpenSSL. Builds an `http::Request` with VAPID (`jwt-simple`) and `TTL`; we send it with any HTTP client. No `Urgency` setter was found, but the request's headers can be edited **[inference from source]**. 154k downloads. |
| `ece` | 2.4.2 | 2026-09-15 | MPL-2.0 | Mozilla's encrypted-content-encoding; OpenSSL backend. |
| `ece-native` | 0.5.0 | 2026-07-26 | MIT OR Apache-2.0 | Companion of `web-push-native`. |
| `vapid` | 0.6.0 | 2022-12-06 | MPL-2.0 | Single release. |
| `pushicino` | 1.0.1 | 2026-06-13 | MIT | "Web Push application server components"; 58 downloads. |

UnifiedPush's own Rust crate (§6) uses `web-push-native` 0.4 for its keys. Sending to ntfy needs no special crate, though `ntfy` 0.9.1 (2026-03-24, MIT) exists (crates.io API).

## 3. ntfy

- **Server.** Written in Go (`module heckel.io/ntfy/v2`, go 1.26). "The project is dual licensed under the Apache License 2.0 and the GPLv2 License" ([README L233](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/README.md#L233)). Current releases: server v2.28.0 (August 27, 2026), Android v1.25.2 (July 23, 2026), iOS v1.7.0 (May 30, 2026) ([releases.md](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/releases.md)). v2.27.0 dropped the "experimental" label from PostgreSQL support.
- **FCM only for ntfy.sh.** "The ntfy Android app uses Firebase only for the main host `ntfy.sh`, and only in the Google Play flavor of the app. It won't use Firebase for any self-hosted servers, and not at all in the F-Droid flavor" ([`subscribe/phone.md` L101-L106](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/subscribe/phone.md#L101-L106)). In code, a subscription is instant when `!BuildConfig.FIREBASE_AVAILABLE || baseUrl != appBaseUrl || subscribeInstantDeliveryCheckbox.isChecked` ([`AddFragment.kt` L432](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/app/src/main/java/io/heckel/ntfy/ui/AddFragment.kt#L432)). Using FCM with a self-hosted server "only works if you modify and build your own Android .apk" ([`config.md` L1571-L1574](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/config.md#L1571-L1574)).
- **Instant delivery** "allows you to receive messages on your phone instantly, even when your phone is in doze mode … achieved with a foreground service, which you'll see as a permanent notification". Users can hide it by turning off its channel ([`phone.md` L73-L96](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/subscribe/phone.md#L73-L96)). In the app source:
  - `SubscriberService` has type `specialUse`, with the Play justification "This is the main feature of this application…" ([`AndroidManifest.xml` L108-L116](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/app/src/main/AndroidManifest.xml#L108-L116)).
  - It keeps one connection per server. The WebSocket pings every 3 minutes, a long interval chosen "so the modem can fully power down between pings". Network changes trigger an immediate reconnect ([`HttpUtil.kt` L47-L70](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/app/src/main/java/io/heckel/ntfy/util/HttpUtil.kt#L47-L70)).
  - It holds a wake lock only while dispatching a received message (`SubscriberService.kt` L423-L425).
  - Restarts come from a sticky service, a boot receiver and a periodic worker every 3 hours. A poll worker also runs every 60 minutes ([`MainActivity.kt` L950-L952](https://github.com/binwiederhier/ntfy-android/blob/51730a0f06cebfad59f1b7bc0cb6d5c47082b032/app/src/main/java/io/heckel/ntfy/ui/MainActivity.kt#L950-L952)). The class comment calls keeping it alive "a hot mess".
- **Publishing from an app server.** "Publishing messages can be done via HTTP PUT/POST", for example `curl -d "…" ntfy.sh/mytopic`. "The topic is essentially a password" ([`publish.md`](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/publish.md)). For UnifiedPush:
  - A request with `up=1`, or with `Content-Encoding: aes128gcm`, is treated as UnifiedPush. Binary bodies are stored base64, and such messages are never forwarded to Firebase or an upstream server ([`server.go` L1259-L1263](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/server/server.go#L1259-L1263), L940).
  - UnifiedPush topics are `up` plus 12 characters. A private server must give anonymous write access to them: `ntfy access '*' 'up*' write-only` ([`config.md` L998-L1015](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/config.md#L998-L1015)).
  - Optional subscriber-based rate limiting rejects publishes to an `up` topic with no prior subscriber (HTTP 507).
  - The default message-size limit is 4K ([`config.md` L1858](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/config.md#L1858)).
- **A content-free pattern already exists in ntfy, for iOS.** With `upstream-base-url` set, a self-hosted ntfy publishes only a "poll request" (the message ID, under a hashed topic) to `ntfy.sh`, which relays it through Firebase and APNs. The iOS app then "fetches the actual message from your server" ([`config.md` L1598-L1630](https://github.com/binwiederhier/ntfy/blob/1e6305ccecc0af15b13d239c3281543bea27b960/docs/config.md#L1598-L1630)).

## 4. Without any push service: holding our own connection

- **Google's position.** FCM "provides a single, persistent connection to the cloud … if your app requires messaging integration with a backend service, we strongly recommend you use FCM if possible, rather than maintaining your own persistent network connection". Its table of acceptable reasons to request a battery-optimization exemption includes messaging apps that "can't use FCM because of technical dependency on another messaging service", safety apps and "Task automation app[s]" ([Doze and App Standby](https://developer.android.com/training/monitoring-device-state/doze-standby)). The Play policy wording itself (support.google.com) was not readable.
- **What a foreground service gives.** A process running a foreground service has unrestricted network access. Its jobs and alarms still follow standby-bucket limits ([power limits](https://developer.android.com/topic/performance/power/power-details), app-state table). Since Android 16, jobs running alongside a foreground service count against job quotas ([Android 16 changes, all apps](https://developer.android.com/about/versions/16/behavior-changes-all)).
- **Candidate foreground-service types** ([Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types); [FGS timeouts](https://developer.android.com/develop/background-work/services/fgs/timeout); [Android 15 changes](https://developer.android.com/about/versions/15/behavior-changes-15)):

  | Type | Described as | Catch |
  |---|---|---|
  | `dataSync` | "Data upload or download … Fetch data … Transfer data between a device and the cloud" | On apps targeting 35+, "a total of 6 hours in a 24-hour period" (the timer resets when the user brings the app to the foreground). Can't be started from `BOOT_COMPLETED`. |
  | `specialUse` | "any valid foreground service use cases that aren't covered by the other … types" | Needs a `PROPERTY_SPECIAL_USE_FGS_SUBTYPE` explanation, "reviewed when you submit your app in the Google Play Console". ntfy uses this type. |
  | `remoteMessaging` | "Transfer text messages from one device to another. Assists with continuity of a user's messaging tasks when they switch devices" | Whether a sync wake-up fits this description is unclear. |
  | `connectedDevice` | "Interactions with external devices" | For peripherals, not servers. |
  | `systemExempted` | Reserved for system uses, but allowed for "Apps holding SCHEDULE_EXACT_ALARM or USE_EXACT_ALARM permission" | Play acceptance for this use was not checked. |
  | `shortService` | "about 3 minutes" | Suits a one-off fetch after a wake-up, not a standing connection. |

- **Declaring types.** From Android 14, apps must declare their types. Play Console asks for a declaration per type ([FGS types required](https://developer.android.com/about/versions/14/changes/fgs-types-required)). The declaration asks for a description, the user impact if the task is deferred, and a video **[unverified: search excerpt of support.google.com/googleplay/android-developer/answer/13392821]**.
- **Starting the service from the background.** Android 12+ allows it only under listed exemptions. These include a high-priority FCM message, an exact alarm "to complete an action that the user requests", `BOOT_COMPLETED` (with the Android 15 type limits), and "The user turns off battery optimizations for your app" ([background start restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).
- **Android 17 and the LAN.** Apps targeting Android 17 need `ACCESS_LOCAL_NETWORK` (in the Nearby devices group) to reach LAN addresses ([Android 17 changes](https://developer.android.com/about/versions/17/behavior-changes-17)). The Android 16 opt-in description says a local network "excludes cellular (WWAN) or VPN connections" ([Android 16 changes](https://developer.android.com/about/versions/16/behavior-changes-16), "Local Network Definition"). This matters for a phone that talks to the home server over Wi-Fi at home, and apparently not over a VPN **[inference from source]**.
- **Google Play distribution** is out of scope for now. Every path above except `dataSync` and FCM leans on a Play review or declaration that could not be read: `specialUse`, the battery exemption, and possibly `systemExempted`.

## 5. Tauri 2 integration

The background note established that Tauri's Rust side starts only with the Activity. A push that arrives while the app is closed reaches a Kotlin `FirebaseMessagingService` or UnifiedPush `PushService`. That Kotlin must then call Rust through JNI exports we write, or start a job or foreground service ([android-background-tauri.md §1](android-background-tauri.md#1-how-tauri-code-runs-on-android)).

| Plugin | Version (date) | Android push | What happens to a message when the Activity isn't running |
|---|---|---|---|
| `tauri-plugin-notifications` (Choochmeque) | 0.5.0-rc.14 (2026-09-16); stable 0.4.6 | FCM | `TauriFirebaseMessagingService` calls `NotificationPlugin.instance?.triggerPushMessage(...)`; with no instance, data messages are dropped [inference from source] |
| `tauri-plugin-fcm` (srod) | 0.2.0 (2026-05-06) | FCM, `firebase-messaging` 24.1.0 | `onMessageReceived` only logs: "Token-only plugin — no display handling" |
| `tauri-plugin-mobile-push` | 0.1.4 (2026-04-18) | FCM, `firebase-messaging` 23.4.1 | Forwards to `MobilePushPlugin.instance?`; dropped otherwise |
| `tauri-plugin-push-notifications` (spicavi) | 0.1.0 (2026-07-14) | FCM, `firebase-messaging` 24.1.0 | Queued in memory until the plugin loads ("very early launch") |
| `tauri-plugin-remote-push` | 1.0.10 (2025-06-23) | FCM | Not inspected; its repository field is a placeholder |

Sources: crate tarballs at the commits in [Access notes](#access-notes).

- **Firebase is not optional in `tauri-plugin-notifications`.**
  - The README says that without the `push-notifications` feature, "Firebase dependencies are not included in Android builds".
  - But `android/build.gradle.kts` adds `firebase-bom` 34.19.0 and `firebase-messaging-ktx:24.1.2` as unconditional `implementation` dependencies. The manifest always declares the Firebase service.
  - The Cargo feature only writes `enablePushNotifications` into `build.properties`, which becomes a `BuildConfig.ENABLE_PUSH_NOTIFICATIONS` flag checked at runtime.
  - So the APK contains Firebase either way **[inference from source]** ([build.gradle.kts](https://github.com/Choochmeque/tauri-plugin-notifications/blob/55be77f158b7b0ab0238861db079bd9f3f995c95/android/build.gradle.kts), [build.rs](https://github.com/Choochmeque/tauri-plugin-notifications/blob/55be77f158b7b0ab0238861db079bd9f3f995c95/build.rs)).
  - 0.4.6 is the same, with BoM 34.7.0.
  - Firebase stopped releasing the KTX module at 25.0.0, so the plugin names a 24.x module that predates FID registration.
- **UnifiedPush in Tauri.** `tauri-plugin-notifications` 0.5.0-rc added UnifiedPush for Linux only, over D-Bus with `zbus` (§6). No Tauri plugin implements the Android connector. Searches on crates.io and npm for "tauri unifiedpush", "unifiedpush", "tauri push" and "tauri fcm" (2026-09-30) found none. A fork, `@sableclient/tauri-plugin-notifications-api` 0.5.3, has the same README.
- **What we would write [inference from source].** The UnifiedPush connector is an ordinary Android library. Tauri's Gradle project can depend on it, and our Kotlin plugin code can call it. Registration needs an Activity context for the distributor chooser, which fits the normal plugin lifecycle. Receiving needs a `PushService` subclass that works without the Activity, which is the same headless problem as the FCM service.

## 6. Linux desktop

- **Spec.** UnifiedPush defines a D-Bus protocol: "UnifiedPush Spec: DBUS_0.3.0" on the mirror, with `org.unifiedpush.Distributor2` and `org.unifiedpush.Connector2` interfaces. It has the same RFC 8291 encryption requirement and 4096-byte limit as Android. The app provides a D-Bus service file so the distributor can start it by D-Bus activation. A distributor may accept only sandboxed (for example Flatpak) apps ([`dbus.md`](https://github.com/UnifiedPush/specifications/blob/eaa0fc50c3dd37c4937da1161abeaf2d50f96760/specifications/dbus.md)).
- **KUnifiedPush** (KDE) is both a client library and a distributor daemon, `org.unifiedpush.Distributor.kde`. It can use Gotify, Mozilla autopush, NextPush or ntfy as its push server. It supports RFC 8291 and VAPID. A System Settings module configures it ([README](https://github.com/KDE/kunifiedpush/blob/6e6b002f91e50e7e74a5b0b108a9ce9d3de9f50a/README.md)). It is under active development (commits in September 2026; version 26.11.70 in `CMakeLists.txt`). Licences include LGPL-2.0-or-later. No GNOME distributor was looked for.
- **Rust.** The official `unifiedpush` crate 0.1.0 (2026-04-08, Apache-2.0, by UnifiedPush/S1m) implements the `Connector2` side with `zbus`, and uses `web-push-native` for keys. Its README shows how a D-Bus service file lets a push start the app in the background (`--unifiedpush-bg`) (crate tarball). `tauri-plugin-notifications` 0.5.0-rc.14 implements the older `Connector1`/`Distributor1` interfaces. It parses message bytes as JSON, and no RFC 8291 decryption was seen [inference from source] ([`src/unifiedpush.rs`](https://github.com/Choochmeque/tauri-plugin-notifications/blob/55be77f158b7b0ab0238861db079bd9f3f995c95/src/unifiedpush.rs)).
- **Trade-off.** UnifiedPush on Linux needs the user to install and run a distributor. Nothing comparable to Doze was found limiting a desktop process that holds its own connection. The Linux background, autostart and Flatpak facts are in [linux-desktop-integration.md](linux-desktop-integration.md).

## Gaps and open questions

**Could not be verified here.**
- **Firebase documentation** (blocked): the priority page, the receive guide (10 vs 20 seconds; the expedited-job exemption), throttling and quotas, pricing and terms, the 4096-byte limit, the force-stop troubleshooter, the service-account and OAuth steps, and the legacy-API shutdown date. All such claims above rest on search excerpts.
- **Google Play policy pages** (blocked): foreground-service declarations, `specialUse` review, battery-optimization exemptions, and whether `systemExempted` use by an app holding an exact-alarm permission passes review.
- **Current UnifiedPush documentation and specs** (unifiedpush.org and Codeberg blocked). The GitHub mirrors stop in May–July 2025. Not read: the AND_3.1.0 text and exact `TEMP_UNAVAILABLE` rules, the current distributor list (the live site has a KUnifiedPush page the mirror lacks), and the current sources of Sunup, NextPush, gCompat-UP and `common-proxies`.
- **RFC 8030, 8291 and 8292** were not read. Their content is described through the UnifiedPush spec and library code.
- **FCM's Web Push path.** How it maps `Urgency` to Android priority, whether the "no notification" downgrade applies, and whether Google will keep accepting VAPID-keyed registrations from apps. UnifiedPush calls that path undocumented.
- **De-Googled devices.** microG's support for `firebase-messaging` and for the embedded distributor's c2dm IPC. GrapheneOS sandboxed Play with the embedded distributor.
- **OEM battery killers** and their effect on distributors' foreground services.
- **Maintenance signals.** GitHub commit history and issues for the Rust crates and Tauri plugins; only release dates were seen.
- **Nothing was tested on a device.**

**Questions the design depends on (facts to settle before choosing).**
1. **Does a sync push "result in a notification"?** Many of our pushes would lead to a silent sync, or to updating or cancelling an existing Android notification. If Android doesn't count those, repeated high-priority FCM messages will be downgraded to normal. Normal priority waits for Doze maintenance windows.
2. **Which pushes need to beat Doze at all?** The ringing alarm already has a live connection, and other Android notifications "catch up". If no push must beat Doze, normal priority (Google's recommendation for sync) may be enough everywhere.
3. **One server-side protocol or two?** Web Push with VAPID works for the embedded FCM distributor and for every UnifiedPush distributor. FCM v1 works only for GMS phones, and is the documented path. Does it matter that the Web Push-to-FCM path is undocumented?
4. **Who holds push credentials in a later public product?** With FCM v1, every self-hosted server needs our Firebase secret, or we run a gateway. With Web Push, each server has its own VAPID key.
5. **Where does the push server live?** A self-hosted ntfy at home reaches phones only at home or over a VPN. That is the same reach as our own connection, so it adds little over holding it ourselves. A public push server (ntfy.sh, Mozilla, Google) needs only outbound access from home, but it sees timing and size metadata. In every case the sync after a push needs a route to the app server.
6. **What do de-Googled phones get?** Should they be asked to install a distributor, get our own foreground-service connection, or rely only on periodic sync and the check before firing? The first adds an app to install. The second adds a permanent Android notification and a Play declaration.
7. **Is the FCM dependency acceptable in the APK?** The official notification plugin has no push. The community plugin links Firebase unconditionally. A two-flavor build like ntfy's would keep a Google-free APK possible.

## Access notes

| Source | How it was read | Version / date |
|---|---|---|
| FCM v1 API | Discovery document at `fcm.googleapis.com/$discovery/rest?version=v1` | revision 20260925 |
| Firebase Android SDK (`firebase-messaging`, `firebase-common`) | Blobless clone of [firebase/firebase-android-sdk](https://github.com/firebase/firebase-android-sdk) through the git proxy | `main` at `09ee73c`, 2026-09-29 |
| Firebase Admin SDK (Go) | `raw.githubusercontent.com/firebase/firebase-admin-go/dev/...` | `dev` branch, read 2026-09-30 |
| UnifiedPush Android libraries (`connector` 3.3.5, `embedded-fcm-distributor` 2.5.0, 3.0.0 and 3.1.0, `distributor` and `distributor-base` 0.7.6) | POMs, AARs and sources jars from `repo1.maven.org` | Release dates from the Maven directory listing |
| UnifiedPush specs | Clone of the GitHub mirror [UnifiedPush/specifications](https://github.com/UnifiedPush/specifications) | `eaa0fc5`, 2025-05-06 (the mirror is not current) |
| UnifiedPush documentation site source | Clone of the GitHub mirror [UnifiedPush/documentation](https://github.com/UnifiedPush/documentation) | `8c61ba9`, 2025-05-04 (not current) |
| UnifiedPush common-proxies | Clone of the GitHub mirror | `3a62b28`, 2025-01-29 |
| ntfy server and docs | Clone of [binwiederhier/ntfy](https://github.com/binwiederhier/ntfy) (docs.ntfy.sh blocked) | `1e6305c`, 2026-09-28 |
| ntfy Android app | Clone of [binwiederhier/ntfy-android](https://github.com/binwiederhier/ntfy-android) | `51730a0`, 2026-07-09 (v1.25.2) |
| KUnifiedPush | Clone of the GitHub mirror [KDE/kunifiedpush](https://github.com/KDE/kunifiedpush) (invent.kde.org blocked) | `6e6b002`, 2026-09-23 |
| autopush-rs | `raw.githubusercontent.com/mozilla-services/autopush-rs/master/...` | 1.84.2 in `Cargo.toml`, read 2026-09-30 |
| GrapheneOS usage guide | `raw.githubusercontent.com/GrapheneOS/grapheneos.org/main/static/usage.html` (grapheneos.org blocked) | `main`, read 2026-09-30 |
| F-Droid metadata (Sunup, ntfy, NextPush, Conversations) | `gitlab.com/fdroid/fdroiddata` raw files and API (f-droid.org blocked) | last changed 2026-07-22 to 2026-09-28 |
| Rust crates (`web-push` 0.11.0, `web-push-native` 0.5.0, `ece` 2.4.2, `firebae-cm` 0.5.0, `fcm-service` 0.2.3, `google-fcm1` 7.0.0+20251212, `fcm` 0.9.2, `unifiedpush` 0.1.0) | crates.io API for metadata; tarballs from `static.crates.io` for source (docs.rs blocked) | versions as listed |
| Tauri plugins (`tauri-plugin-notifications` 0.5.0-rc.14 at `55be77f` and 0.4.6 at `e85ed7d`; `tauri-plugin-fcm` 0.2.0 at `b9d4d18`; `tauri-plugin-mobile-push` 0.1.4 at `fd23bae`; `tauri-plugin-push-notifications` 0.1.0 at `ef5f095`) | Crate tarballs; commits from `.cargo_vcs_info.json` | as listed |
| Plugin and crate discovery | crates.io search API ("web push", webpush, unifiedpush, fcm, firebase, "firebase messaging", ntfy, vapid, "tauri push", "tauri fcm", "tauri unifiedpush", "tauri notifications", "tauri firebase"); npm registry search | 2026-09-30 |
| developer.android.com | Fetched directly. "Last updated": Doze 2026-08-18; Power limits 2026-05-19; App Standby 2026-09-16; FGS background start 2026-09-16; FGS types 2026-09-21; FGS timeouts 2026-09-16; FGS types required (14) 2026-09-21; Android 15 changes and all-apps changes 2026-09-16; Android 16 changes 2026-09-16; Android 17 changes 2026-09-16; Android 3.1 notes 2024-01-03 | read 2026-09-30 |
| This repo's issue #25 | GitHub MCP | read 2026-09-30 |

**Blocked hosts:** `firebase.google.com`, `firebase.blog`, `developers.google.com`, `support.google.com`, `play.google.com`, `dl.google.com` (Google Maven metadata), `android-developers.googleblog.com`, `developer.chrome.com`, `groups.google.com`, `unifiedpush.org`, `codeberg.org` (web and git), `docs.ntfy.sh`, `ntfy.sh`, `f-droid.org`, `apps.nextcloud.com`, `modules.prosody.im`, `invent.kde.org`, `api.kde.org`, `www.volkerkrause.eu`, `s1m.fr`, `grapheneos.org`, `bugzilla.mozilla.org`, `mozilla-services.github.io`, `www.rfc-editor.org`, `datatracker.ietf.org`, `www.ietf.org`, `tools.ietf.org`, `docs.rs`, `lib.rs`, `tauri.app`, `web.archive.org`, `jitpack.io`, `central.sonatype.com`, `search.maven.org`. `github.com` web pages and `api.github.com` were refused for repositories not attached to the session, but `git clone` and `raw.githubusercontent.com` worked. GitHub links above point to files read that way, at the commits shown.
