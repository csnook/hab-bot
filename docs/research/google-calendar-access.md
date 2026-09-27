# Reading Google Calendar on Android and Linux

Research for [csnook/hab-bot#5](https://github.com/csnook/hab-bot/issues/5): the ways the app can read a user's Google Calendar events on Android and on desktop Linux, what each costs, where each can run (device or server), and which ones make it easy to add other calendar providers later.

- **Read on:** 2026-09-27. Versions and dates of what was read are given per source in [Access notes](#access-notes).
- **Terminology:** `CONTEXT.md` terms are used as defined there. A Google calendar that the user sets up is a **source** of the calendar **source class**. It supplies **triggers** ("30 minutes before an event") and **conditions** ("not during a meeting"). Google and Android use "reminder" and "alert" for their own per-event notifications. Those clash with the `CONTEXT.md` **Reminder** and **Alert**, so this document calls them **event reminders**, except when quoting an API name such as `CalendarContract.Reminders` or `ACTION_EVENT_REMINDER`.
- **Unverified claims:** this environment's proxy blocked `developers.google.com`, `support.google.com` and the other Google documentation and help hosts. It also blocked the KDE and GNOME documentation sites, `docs.rs`, the DAVx5 and GrapheneOS sites, the Tauri site and IETF hosts (full list in [Access notes](#access-notes)). Claims that rest only on a search-engine excerpt of a blocked page are marked **[unverified: search excerpt]**. Claims that are inferences no source here confirms are marked **[unverified]**. Before a design decision rests on a Google policy detail, check it against the live page.

## Summary

- **There are four routes in.** Each has a fixed place where it can run:

  | Route | Runs on | hab-bot needs its own Google Cloud project and OAuth client? | Needs Google Play services? | Adding other providers later |
  |---|---|---|---|---|
  | Android Calendar Provider (`CalendarContract`) | Android device only | No. Whatever syncs the calendar handles Google sign-in. | Not for hab-bot. Google's own calendar sync adapter is proprietary Google software. DAVx5 is an open-source alternative that needs no Play services. | Easy: any account with an Android sync adapter (CalDAV, iCal feeds, Exchange and so on) appears in the same tables |
  | Google Calendar API v3 (REST/JSON) | Device or server | Yes | Only for Google's recommended Android auth library (`play-services-auth`). Browser-based OAuth needs no Play services. | Google-only. Each other provider needs its own client. |
  | Google CalDAV | Device or server | Yes (Google allows OAuth 2.0 only) | No | Easy: the same protocol as other CalDAV servers. The Google-specific parts are the sign-in and the URL layout. |
  | Desktop stores: KDE Akonadi, GNOME Evolution Data Server (EDS) | Linux desktop only | No. KDE and GNOME ship their own Google clients. | n/a | Easy: the store merges every account configured on the desktop |

- **Calendar data is a "sensitive" scope for Google OAuth.** An unverified app shows users an "unverified app" warning and is capped at 100 new users. Personal use (fewer than 100 users, all known to the developer) is exempt from verification. Leaving the OAuth consent screen in **Testing** status makes refresh tokens expire after 7 days, which would force a new sign-in every week on every device or server that holds one. **[unverified: search excerpt]** for all of these.
- **The API has the pieces a calendar source class needs.** `events.list` can expand recurring events into instances (`singleEvents`), sync incrementally (`syncToken`, 410 GONE means a full resync is needed) and report busy or free per event (`transparency`). `freebusy.query` exists. Newer narrow read scopes exist, such as `calendar.events.readonly`, `calendar.freebusy`, `calendar.events.public.readonly` and `calendar.calendarlist.readonly` ([discovery document](https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest), revision 20260826). Push notifications need a publicly reachable HTTPS URL with a valid certificate **[unverified: search excerpt]**, which fits the server but not a device.
- **OAuth on the desktop:** the loopback redirect (`http://127.0.0.1:<port>`) with the system browser. KDE's LibKGAPI and vdirsyncer both use it ([LibKGAPI](https://raw.githubusercontent.com/KDE/libkgapi/master/src/core/private/fullauthenticationjob.cpp), [vdirsyncer](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/vdirsyncer/storage/google.py)).
- **OAuth on Android:** Google recommends `AuthorizationClient` from `com.google.android.gms:play-services-auth` ([Android docs](https://developer.android.com/identity/authorization)). The Play-free path is AppAuth with a browser Custom Tab, which is what DAVx5 does ([DAVx5 source](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/kotlin/at/bitfire/davdroid/network/OAuthGoogle.kt)). Google's docs say custom URI scheme redirects and loopback are not supported for *Android-type* OAuth clients **[unverified: search excerpt]**. Which client type and redirect a new Play-free Android client could use is the main open question.
- **Public calendars** can be read on the server without any user's token: the API requires at least an API key ("Required unless you provide an OAuth 2.0 token", [discovery document](https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest)). An unauthenticated request was refused with 403 "Please use API Key" (probe, 2026-09-27).
- **freedesktop.org has no calendar standard.** xdg-desktop-portal 23.0 defines no calendar portal ([portal list](https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/meson.build)). Akonadi (KDE's default) is reached through C++/Qt libraries. The EDS D-Bus interfaces sit in a library EDS names "private". No Rust bindings for either were found on crates.io.

## Access notes

| Source | How it was read | Version / date |
|---|---|---|
| Android developer docs (Calendar Provider guide, `CalendarContract` reference, permissions, JobScheduler, broadcast exceptions, identity/authorization) | `developer.android.com`, fetched directly | Per-page "last updated" dates are given at each citation, from 2024-05-22 (Calendar Provider guide) to 2026-09-16 |
| AOSP Calendar Provider source | GitHub mirror `aosp-mirror/platform_packages_providers_calendarprovider`, `main`, via `raw.githubusercontent.com` | read 2026-09-27 |
| GApps package layout (what is proprietary, what is separate) | MindTheGapps `vendor_gapps` on GitLab, branch `baklava` (Android 16), via the GitLab API | last activity 2026-09-15 |
| Google Calendar API v3 | Machine-readable [discovery document](https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest) from `www.googleapis.com`, plus one live unauthenticated request | **revision 20260826** |
| Google OAuth endpoints | [`accounts.google.com/.well-known/openid-configuration`](https://accounts.google.com/.well-known/openid-configuration) | live, 2026-09-27 |
| Google Identity, OAuth policy, Calendar and CalDAV guides, Cloud Console help | **Blocked** (`developers.google.com`, `support.google.com`, `developers.google.cn`). Only search excerpts were seen. | — |
| DAVx5, ICSx5, AppAuth-Android, vdirsyncer | Source and README files via `raw.githubusercontent.com` | DAVx5 `main-ose`, AppAuth `master`, vdirsyncer `main`, read 2026-09-27 |
| KDE Akonadi, kdepim-runtime, LibKGAPI, kdepim-addons, kaccounts-providers | KDE GitHub mirrors via `raw.githubusercontent.com` (`invent.kde.org` and `api.kde.org` blocked) | Akonadi `master` = PIM 6.8.40 / release service 26.11.40 (development branch) |
| GNOME Evolution Data Server, GNOME Online Accounts | GNOME GitHub mirrors via `raw.githubusercontent.com` (`gitlab.gnome.org` blocked) | EDS `master` = 3.63.1 (development toward 3.64); GOA `master` |
| xdg-desktop-portal | `flatpak/xdg-desktop-portal` `main`, `data/meson.build` | 23.0 |
| Tauri docs and plugins | `tauri-apps/tauri-docs` `v2` and `tauri-apps/plugins-workspace` `v2` via `raw.githubusercontent.com` (`v2.tauri.app` blocked) | read 2026-09-27 |
| Rust crates | crates.io API (metadata and READMEs); `docs.rs` blocked | versions as listed in [section 6](#6-rust-and-tauri-building-blocks) |

Blocked hosts: `developers.google.com`, `developers.google.cn`, `support.google.com`, `knowledge.workspace.google.com`, `calendar.google.com`, `console.developers.google.com`, `console.cloud.google.com`, `docs.cloud.google.com`, `workspaceupdates.googleblog.com`, `developers.googleblog.com`, `android-developers.googleblog.com`, `policies.google.com`, `play.google.com`, `apidata.googleusercontent.com`, `android.googlesource.com`, `source.android.com`, `cs.android.com`, `api.kde.org`, `develop.kde.org`, `invent.kde.org`, `userbase.kde.org`, `gitlab.gnome.org`, `docs.gtk.org`, `developer.gnome.org`, `gitlab.freedesktop.org`, `specifications.freedesktop.org`, `flatpak.github.io`, `docs.rs`, `lib.rs`, `www.davx5.com`, `manual.davx5.com`, `grapheneos.org`, `microg.org`, `v2.tauri.app`, `git.sr.ht`, `codeberg.org`, `oauth.net`, `datatracker.ietf.org`, `www.rfc-editor.org`, `web.archive.org`. `github.com` web pages and the GitHub search API were also unavailable (only raw file access worked), so repositories could not be listed or searched.

## 1. Android Calendar Provider (`CalendarContract`)

### What it is and what hab-bot would get

- **A platform content provider, part of AOSP.** The guide calls it "a repository for a user's calendar events" that "allows you to perform query, insert, update, and delete operations on calendars, events, attendees, reminders, and so on" ([Calendar Provider guide](https://developer.android.com/identity/providers/calendar-provider), last updated 2024-05-22). In AOSP it is the package `com.android.providers.calendar`, with authority `com.android.calendar` and `android:readPermission="android.permission.READ_CALENDAR"` ([AOSP manifest](https://raw.githubusercontent.com/aosp-mirror/platform_packages_providers_calendarprovider/main/AndroidManifest.xml)).
- **Permission:** `READ_CALENDAR`, "Protection level: dangerous" ([Manifest.permission](https://developer.android.com/reference/android/Manifest.permission)). "Runtime permissions, also known as dangerous permissions … you need to request runtime permissions in your app before you can access the restricted data" ([Permissions overview](https://developer.android.com/guide/topics/permissions/overview), last updated 2026-09-16). It grants read access to every calendar on the device; no per-calendar grant appears in the sources read.
- **Tables:** Calendars, Events, Instances, Attendees, Reminders, ExtendedProperties, SyncState and others ([`CalendarContract`](https://developer.android.com/reference/android/provider/CalendarContract), last updated 2026-08-03).
- **Recurrences are expanded by the provider.** "For recurring events, multiple rows will automatically be generated which correspond to multiple occurrences of that event" in the Instances table ([`CalendarContract`](https://developer.android.com/reference/android/provider/CalendarContract)). `Instances.CONTENT_URI` is queried with "the begin and end of the range … added as path segments" ([`CalendarContract.Instances`](https://developer.android.com/reference/android/provider/CalendarContract.Instances)). A trigger such as "30 minutes before an event" can therefore read instance begin times without an RRULE engine.
- **Fields for a "not during a meeting" condition:** `AVAILABILITY` ("If this event counts as busy time or is still free time", one of `AVAILABILITY_BUSY`, `AVAILABILITY_FREE`, `AVAILABILITY_TENTATIVE`), `SELF_ATTENDEE_STATUS` ("so that we can efficiently filter out events that are declined") and `STATUS` (tentative, confirmed or cancelled) ([`EventsColumns`](https://developer.android.com/reference/android/provider/CalendarContract.EventsColumns)).
- **Only synced calendars have events on the device.** `Calendars.SYNC_EVENTS`: "0 - Do not sync this calendar or store events for this calendar. 1 - Sync down events for this calendar." ([`CalendarColumns`](https://developer.android.com/reference/android/provider/CalendarContract.CalendarColumns)).
- **Watching for changes:** `JobInfo.Builder.addTriggerContentUri` runs a job when a `content:` URI changes. It cannot be combined with periodic or persisted jobs, so the job has to be rescheduled after each change ([JobInfo.Builder](https://developer.android.com/reference/android/app/job/JobInfo.Builder)).
- **Event reminders the user set in their calendar:** `ACTION_EVENT_REMINDER` is "the intent that gets fired when an alarm notification needs to be posted for a reminder" (API 14, [`CalendarContract`](https://developer.android.com/reference/android/provider/CalendarContract)). It is exempt from the implicit-broadcast restrictions: "Sent by the calendar provider to post an event reminder to the calendar app. Since the calendar provider doesn't know what the calendar app is, this broadcast must be implicit." ([broadcast exceptions](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions), last updated 2026-02-26). It covers event reminders stored in the Reminders table, "specified in minutes before the event" ([`CalendarContract`](https://developer.android.com/reference/android/provider/CalendarContract)). It is not a general hook for offsets the app chooses itself.

### Who puts Google events into it, and the Play services question

- **The provider holds only what sync adapters write.** "A sync adapter synchronizes the calendar data on a user's device with another server or data source", and "different calendars can be associated with different types of accounts (Google Calendar, Exchange, and so on)". `ACCOUNT_TYPE_LOCAL` calendars "do not get synced" ([Calendar Provider guide](https://developer.android.com/identity/providers/calendar-provider)).
- **Google's sync adapter is a proprietary Google app, packaged apart from Play services.** In MindTheGapps (the GApps set built for LineageOS-style ROMs), `GoogleCalendarSyncAdapter` sits in `common/proprietary/product/app` next to `GoogleContactsSyncAdapter`. Play services (`GmsCore`) and `Phonesky` sit in `arm64/proprietary/product/priv-app`, and `GoogleServicesFramework` in `system_ext/priv-app` ([product/app listing](https://gitlab.com/MindTheGapps/vendor_gapps/-/tree/baklava/common/proprietary/product/app), [priv-app listing](https://gitlab.com/MindTheGapps/vendor_gapps/-/tree/baklava/arm64/proprietary/product/priv-app), branch `baklava`). The provider itself is AOSP and has no Google dependency ([AOSP manifest](https://raw.githubusercontent.com/aosp-mirror/platform_packages_providers_calendarprovider/main/AndroidManifest.xml)).
- **Not verified:** whether `GoogleCalendarSyncAdapter` works without full Play services, for example with microG or GrapheneOS's sandboxed Google Play. GrapheneOS's usage guide ([source](https://raw.githubusercontent.com/GrapheneOS/grapheneos.org/main/static/usage.html)) says nothing about calendar sync. **[unverified]**
- **A Play-free alternative: DAVx5.** DAVx5 is a GPLv3 CalDAV/CardDAV sync app for Android ([README](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/README.md)) with a "Google Contacts / Calendar" login ([strings](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/res/values/strings.xml)). It:
  - signs in with OpenID AppAuth, using the redirect URI `<packageName>:/oauth2/redirect` ([OAuthIntegration.kt](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/kotlin/at/bitfire/davdroid/network/OAuthIntegration.kt))
  - requests the scopes `https://www.googleapis.com/auth/calendar` and `.../auth/carddav`, against `https://apidata.googleusercontent.com/caldav/v2/<account>/user` ([OAuthGoogle.kt](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/kotlin/at/bitfire/davdroid/network/OAuthGoogle.kt))
  - ships its own client ID, but lets the user enter their own ("Client ID (optional)", [strings](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/res/values/strings.xml))
  - states that it "complies with the Google API Services User Data Policy, including the Limited Use requirements" ([strings](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/res/values/strings.xml))

  Its README says it does "Contacts, calendars, and tasks sync", with "content provider access" in its `synctools` library ([README](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/README.md)).
- **iCal feeds:** ICSx5, from the same developers, is "an Android app to subscribe to remote Webcal feeds / iCalendar files" ([README](https://raw.githubusercontent.com/bitfireAT/icsx5/main/README.md)). The README does not say that it writes into the Calendar Provider. **[unverified]**

### Cost to hab-bot

- **No Google Cloud project, no OAuth and no verification for hab-bot.** The sync adapter the user already has deals with Google.
- **A native Android plugin.** Tauri mobile plugins are Kotlin (or Java) classes that extend `app.tauri.plugin.Plugin`, with `@Command` methods callable from Rust or JS and runtime permissions declared in `@TauriPlugin(permissions = …)` ([Tauri mobile plugin docs](https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/develop/Plugins/develop-mobile.mdx)). The only matching crate found on crates.io was `clepsydre-android` 0.1.0 ("Manage calendars and events from Android providers", 20 downloads) ([crates.io search](https://crates.io/search?q=calendar%20provider%20android)).
- **Device only.** The server sees the data only if the device uploads it, which ties into each device's server-trust setting.
- **Freshness depends on the sync adapter's schedule**, which none of the sources read documents for Google's adapter. **[unverified]**
- **Other providers come for free:** anything with an Android sync adapter lands in the same tables.

## 2. Google Calendar API v3

### API surface relevant to a calendar source class

From the [discovery document](https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest) (revision 20260826):

- **Resources:** `settings`, `calendars`, `calendarList`, `acl`, `colors`, `events`, `channels`, `freebusy`.
- **Recurrence expansion:** `events.list` with `singleEvents` means "Whether to expand recurring events into instances and only return single one-off events and instances of recurring events". `events.instances` also exists.
- **Incremental sync:** `syncToken` "makes the result of this list request contain only entries that have changed since then. All events deleted since the previous list request will always be in the result set". It cannot be combined with `timeMin`, `timeMax`, `updatedMin`, `q` or `orderBy`. "If the syncToken expires, the server will respond with a 410 GONE response code and the client should clear its storage and perform a full synchronization." Pages hold 250 events by default and at most 2,500.
- **Busy or free:** `transparency` is `opaque` (default, "Show me as to Busy") or `transparent`. `eventType` is one of `default`, `focusTime`, `outOfOffice`, `workingLocation`, `birthday` or `fromGmail`. `freebusy.query` returns busy intervals.
- **Push:** `events.watch` creates a channel of type `web_hook` with an `address`, an optional `expiration` and a `token`. Google's push guide says the address must be HTTPS with a valid SSL certificate, which rules out self-signed ones, and that notifications carry no body, so the client has to call the API again to see what changed ([push guide](https://developers.google.com/workspace/calendar/api/guides/push)) **[unverified: search excerpt]**.

**Read scopes and the methods that accept them** (all from the [discovery document](https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest)):

| Scope (`https://www.googleapis.com/auth/…`) | Google's description | Accepted by |
|---|---|---|
| `calendar.readonly` | See and download any calendar you can access using your Google Calendar | `events.list`/`instances`/`watch`, `freebusy.query`, `calendarList.list`, `calendars.get`, `settings.list`/`get` |
| `calendar.events.readonly` | View events on all your calendars | `events.list`/`instances`/`watch` |
| `calendar.events.owned.readonly` | See the events on Google calendars you own | `events.list`/`instances`/`watch` |
| `calendar.events.public.readonly` | See the events on public calendars | `events.list`/`instances`/`watch` |
| `calendar.events.freebusy` | See the availability on Google calendars you have access to | `events.list`/`instances`/`watch`, `freebusy.query` |
| `calendar.freebusy` | View your availability in your calendars | `freebusy.query` |
| `calendar.calendarlist.readonly` | See the list of Google calendars you're subscribed to | `calendarList.list` |
| `calendar.calendars.readonly` | See the title, description, default time zone, and other properties of Google calendars you have access to | `calendars.get` |
| `calendar.settings.readonly` | View your Calendar settings | `settings.list`/`get` |
| `calendar` | See, edit, share, and permanently delete all the calendars you can access | everything |

- **Public calendars and API keys.** The `key` parameter is described as "Required unless you provide an OAuth 2.0 token" ([discovery document](https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest)). An unauthenticated `events.list` on the public US holidays calendar returned `403 PERMISSION_DENIED`: "Method doesn't allow unregistered callers … Please use API Key or other form of API consumer identity" ([probe URL](https://www.googleapis.com/calendar/v3/calendars/en.usa%23holiday%40group.v.calendar.google.com/events?maxResults=1), 2026-09-27). So even public data needs at least an API key from a Cloud project. Whether an API key alone is enough for a public calendar was not tested. **[unverified]**
- **Quotas:** quotas apply per minute per project and per minute per user per project, and Google advises push over repeated polling ([usage limits](https://developers.google.com/workspace/calendar/api/guides/quota)) **[unverified: search excerpt]**. From May 1, 2026 Google is changing Calendar API quotas under a "standardized tiering model". Projects created on or after that date get the new quotas, and a paid increase above a standard daily limit is planned ([tools-safety page](https://developers.google.com/workspace/tools-safety), [Workspace Updates, May 2026](https://workspaceupdates.googleblog.com/2026/05/agent-tools-and-security-updates-for-workspace-developers.html)) **[unverified: search excerpt]**. The actual numbers were not seen.

### Google Cloud project and OAuth client setup

- **What has to be set up:** a Cloud project, with the API enabled, an OAuth consent screen configured (branding and audience), and OAuth clients created. On Android, Google's docs say to "create an Android client ID … You will need to specify your app's package name and SHA-1 signature", optionally verify app ownership through Play Console, and create a "Web application" client ID for a backend that exchanges codes ([Android authorization](https://developer.android.com/identity/authorization), last updated 2025-10-27).
- **Google's OAuth server:** it supports PKCE (`S256`, `plain`), the `authorization_code`, `refresh_token` and `device_code` grants, and has a `device_authorization_endpoint` ([OpenID configuration](https://accounts.google.com/.well-known/openid-configuration)).
- **The device flow ("TVs and limited-input devices") supports only a limited list of scopes** ([limited-input device guide](https://developers.google.com/identity/protocols/oauth2/limited-input-device)) **[unverified: search excerpt]**. Whether any Calendar scope is on that list could not be confirmed. If one is, the server could run OAuth itself without a redirect.

### OAuth for installed apps

**Desktop Linux:**

- **Loopback redirect.** The loopback IP flow "will continue to be supported on desktop apps". It is deprecated for Android, iOS and Chrome client types: new clients of those types were blocked from March 14, 2022 and existing ones from August 31, 2022 ([loopback migration](https://developers.google.com/identity/protocols/oauth2/resources/loopback-migration)) **[unverified: search excerpt]**. The manual copy/paste (OOB) flow was blocked for all clients on January 31, 2023 ([OOB migration](https://developers.google.com/identity/protocols/oauth2/resources/oob-migration)) **[unverified: search excerpt]**.
- **Implementations that do this:**
  - KDE's LibKGAPI starts a `QTcpServer` on `LocalHost`, opens the consent URL with `QDesktopServices::openUrl` and uses `redirect_uri=http://127.0.0.1:<port>` ([fullauthenticationjob.cpp](https://raw.githubusercontent.com/KDE/libkgapi/master/src/core/private/fullauthenticationjob.cpp)).
  - vdirsyncer runs a local WSGI server on `127.0.0.1` and says "The correct application type is 'Desktop application'" ([google.py](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/vdirsyncer/storage/google.py), [config.rst](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/docs/config.rst)).
- **No embedded webviews.** Google's policy prohibits OAuth requests in embedded webviews ([OAuth 2.0 policies](https://developers.google.com/identity/protocols/oauth2/policies)) **[unverified: search excerpt]**. For a Tauri app, this would mean opening consent in the system browser rather than the app's own webview **[unverified]**. AppAuth says the same independently: "`WebView` is explicitly *not* supported" ([AppAuth README](https://raw.githubusercontent.com/openid/AppAuth-Android/master/README.md)).
- **Client secrets in open-source code.** KDE's Google resource hard-codes its client ID and secret ([googlesettings.cpp](https://raw.githubusercontent.com/KDE/kdepim-runtime/master/resources/google-groupware/googlesettings.cpp)). GOA builds its ID in and exposes `ClientId` and `ClientSecret` as D-Bus properties ([GOA D-Bus interfaces](https://raw.githubusercontent.com/GNOME/gnome-online-accounts/master/data/dbus-interfaces.xml)). vdirsyncer instead makes each user register their own client, saying that hard-coding them "into opensource software" is against Google's Terms of Service, "Confidential Matters" ([config.rst](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/docs/config.rst)). The ToS page could not be read. **[unverified]**

**Android:**

- **Google's recommended path needs Play services libraries.** `Identity.getAuthorizationClient(activity)` with `AuthorizationRequest…requestOfflineAccess(serverClientId)`, from the "Google Identity Services library" `com.google.android.gms:play-services-auth:22.0.0`. The `serverAuthCode` goes to the backend "to exchange for an access and refresh token" ([Android authorization](https://developer.android.com/identity/authorization)). Sign in with Google via Credential Manager uses `androidx.credentials:credentials-play-services-auth` and `googleid`, and covers authentication only: for data in a Google Account, "use the AuthorizationClient API" ([Credential Manager SIWG](https://developer.android.com/identity/sign-in/credential-manager-siwg), [implementation](https://developer.android.com/identity/sign-in/credential-manager-siwg-implementation), last updated 2026-09-16). That these calls fail on a device without the Play services app is implied by the `com.google.android.gms` package but not stated on the pages read. **[unverified]**
- **The Play-free path: AppAuth-Android.** It uses Custom Tabs, "Both Custom URI Schemes … and App Links (Android M / API 23+) can be used" for redirects, and it recommends custom-scheme redirects ([AppAuth README](https://raw.githubusercontent.com/openid/AppAuth-Android/master/README.md)). Its dependencies are AndroidX only (`androidx.browser`, `annotation`, `appcompat`) ([build.gradle](https://raw.githubusercontent.com/openid/AppAuth-Android/master/library/build.gradle)). DAVx5 uses it against Google today (see §1).
- **Google's restrictions on Android redirects:** Google's docs say custom URI schemes are "no longer supported" for Android and Chrome app clients because of app-impersonation risk, and loopback is deprecated for Android clients ([native-app guide](https://developers.google.com/identity/protocols/oauth2/native-app)) **[unverified: search excerpt]**. Two real apps show other patterns:
  - DAVx5: the package-name scheme `<packageName>:/oauth2/redirect` (above)
  - GNOME Online Accounts: the reversed client ID, `com.googleusercontent.apps.<id>:/oauth2redirect` ([goagoogleprovider.c](https://raw.githubusercontent.com/GNOME/gnome-online-accounts/master/src/goabackend/goagoogleprovider.c))

  Which client types those are registered as, and whether a *new* client can do the same, could not be verified. **[unverified]**
- **Tauri deep links on Android:** the deep-link plugin supports "Custom URI schemes (no host required, no verification)" and App Links backed by `.well-known/assetlinks.json` ([deep-linking docs](https://raw.githubusercontent.com/tauri-apps/tauri-docs/v2/src/content/docs/plugin/deep-linking.mdx), [config.rs](https://raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/plugins/deep-link/src/config.rs)). On Linux, "deep links are delivered as a command line argument to a new app process", and the single-instance plugin passes them to the running instance (same docs).

### Verification, Testing status and limits for a personal project

All **[unverified: search excerpt]**; the pages were blocked:

- **Calendar data is sensitive.** "Examples of sensitive scopes include reading events stored in Google Calendar", and apps requesting sensitive or restricted scopes must complete verification "unless your app's use qualifies for an exception". The Cloud Console labels each scope as non-sensitive, sensitive or restricted when it is added. Sensitive-scope verification typically takes 3–5 business days. Restricted scopes add an annual third-party security assessment ([sensitive-scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/sensitive-scope-verification), [restricted-scope verification](https://developers.google.com/identity/protocols/oauth2/production-readiness/restricted-scope-verification), [Calendar scopes page](https://developers.google.com/workspace/calendar/api/auth)).
- **Not verified:** how Google classifies the newer granular scopes (`calendar.events.public.readonly`, `calendar.freebusy`, `calendar.calendarlist.readonly`, `calendar.events.owned.readonly`), and whether any Calendar scope is restricted. **[unverified]**
- **Personal-use exception:** "The app is not shared with anyone else or will be used by fewer than 100 people (all of whom are known personally to you)". Such users click through the "unverified app" warning ([unverified apps](https://support.google.com/cloud/answer/7454865), [when verification is not needed](https://support.google.com/cloud/answer/13464323)).
- **Unverified apps in production:** "the unverified app screen will be displayed before the consent screen, and your app will be limited to 100 new users until it is verified" ([unverified apps](https://support.google.com/cloud/answer/7454865)).
- **Testing status:**
  - up to 100 listed test users; other accounts get "Access denied" ([Manage App Audience](https://support.google.com/cloud/answer/15549945))
  - with External user type and Testing status, "a refresh token expiring in 7 days" is issued, "unless the only OAuth scopes requested are a subset of name, email, and profile" ([Using OAuth 2.0](https://developers.google.com/identity/protocols/oauth2))
- **Refresh-token limits in production:** tokens generally last until revoked or unused for about six months. There is "a limit of 100 refresh tokens per Google Account per OAuth 2.0 client ID". Past it, "creating a new refresh token automatically invalidates the oldest refresh token without warning" ([Using OAuth 2.0](https://developers.google.com/identity/protocols/oauth2)). Every device plus the server signing in separately with the same client counts toward this limit.

### Device or server

- **On the device:** Linux uses the loopback flow. Android uses either `AuthorizationClient` (Play services) or AppAuth (Play-free, with the client-type question above). The device then holds the refresh token.
- **On the server:** the server holds a refresh token, and then holds whatever scope was granted. There are three ways to get one there:
  1. Android `serverAuthCode`, which needs Play services ([Android authorization](https://developer.android.com/identity/authorization)).
  2. The web-server flow with a registered redirect. Production web clients need HTTPS redirect URIs ([web-server guide](https://developers.google.com/identity/protocols/oauth2/web-server)) **[unverified: search excerpt]**. Whether a LAN-only or private hostname is accepted is **[unverified]**.
  3. A device completes the flow and hands the token to the server. Whether Google permits that for a Desktop or Android client is **[unverified]**.
- **Change detection on the server:** push, if the server has a public HTTPS URL with a valid certificate; otherwise polling with `syncToken`.
- **Public calendars on the server:** an API key, with no user token (see above).
- **Extensibility:** Google-only. Each other provider (Microsoft Graph, CalDAV and so on) would be a separate client.

## 3. CalDAV access to Google Calendar

- **Endpoint and authentication.** The principal URL is `https://apidata.googleusercontent.com/caldav/v2/CALENDAR_ID/user`. "The CalDAV server refuses to authenticate a request unless it arrives over HTTPS with OAuth 2.0 authentication of a Google Account" ([CalDAV guide](https://developers.google.com/workspace/calendar/caldav/v2/guide)) **[unverified: search excerpt]**. The same host and path are used in code by DAVx5 ([OAuthGoogle.kt](https://raw.githubusercontent.com/bitfireAT/davx5-ose/main-ose/core/src/main/kotlin/at/bitfire/davdroid/network/OAuthGoogle.kt)), GNOME Online Accounts ([goagoogleprovider.c](https://raw.githubusercontent.com/GNOME/gnome-online-accounts/master/src/goabackend/goagoogleprovider.c)) and vdirsyncer ([google.py](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/vdirsyncer/storage/google.py)).
- **The same Cloud costs as the REST API.** A project with the "CalDAV" API enabled ("**not** the Calendar and Contacts APIs, those are different and won't work") and an OAuth client ([vdirsyncer config.rst](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/docs/config.rst); [CalDAV guide](https://developers.google.com/workspace/calendar/caldav/v2/guide) **[unverified: search excerpt]**). The CalDAV API has no entry in Google's public discovery directory; only `calendar v3` is listed ([discovery directory](https://www.googleapis.com/discovery/v1/apis)).
- **Scope: every implementation checked asks for full `calendar`.** DAVx5, GOA ("Google Calendar API (CalDAV and GData)") and vdirsyncer all request `https://www.googleapis.com/auth/calendar`, which is read and write (sources above). Whether Google's CalDAV accepts a read-only scope was not verified. **[unverified]**
- **Password logins are gone for Workspace accounts.** From May 1, 2025, Workspace accounts no longer support apps that sign in with only a username and password, and CalDAV and CardDAV need OAuth ([Transition from less secure apps](https://support.google.com/a/answer/14114704)) **[unverified: search excerpt]**. Whether app passwords still work for CalDAV on consumer accounts is **[unverified]**.
- **Known limitations:**
  - Google rejects `VTODO`
  - which calendars CalDAV exposes is set on a separate settings page, `https://calendar.google.com/calendar/syncselect`

  ([vdirsyncer config.rst](https://raw.githubusercontent.com/pimutils/vdirsyncer/main/docs/config.rst)). EDS's CalDAV backend carries a Google flag, commented as a "Temporary hack to indicate it's talking to a google calendar", with Google-specific branches, for example "Google can reject saving for events organized by someone else" ([e-cal-backend-caldav.c](https://raw.githubusercontent.com/GNOME/evolution-data-server/master/src/calendar/backends/caldav/e-cal-backend-caldav.c)).
- **Data model:** CalDAV returns iCalendar objects. The client expands RRULEs itself unless it relies on server-side expansion; whether Google supports that was not checked. **[unverified]**
- **Runs on:** the device or the server. The OAuth question is the same as for the REST API.
- **Extensibility:** the same client code can talk to other CalDAV servers. The Google-specific parts are the OAuth sign-in, the URL layout and the quirks above.

## 4. Desktop calendar stores (KDE Akonadi, GNOME EDS)

**freedesktop.org:** xdg-desktop-portal 23.0 ships portals for Account, Background, Camera, Documents, Email, FileChooser, Location, Notification, OpenURI, Secret, Settings and others. It has **no calendar or contacts portal** ([data/meson.build](https://raw.githubusercontent.com/flatpak/xdg-desktop-portal/main/data/meson.build)). There is no cross-desktop standard for reading calendars, so each desktop's store is its own integration.

### KDE Akonadi (the Plasma default)

- **What it is:** "an extensible cross-desktop storage service for PIM data", with a server and client libraries ([README](https://raw.githubusercontent.com/KDE/akonadi/master/README.md); master is PIM 6.8.40, Qt 6.9, KF 6.28, per [CMakeLists.txt](https://raw.githubusercontent.com/KDE/akonadi/master/CMakeLists.txt)).
- **Protocols:** D-Bus is "used for management tasks and change notifications". ASAP, "based on the well-known IMAP protocol", carries data. "Accessing the Akonadi server using the ASAP and D-Bus interfaces directly is cumbersome", so clients use its libraries ([server docs](https://raw.githubusercontent.com/KDE/akonadi/master/docs/server.md)). Those are C++/Qt job classes (`ItemFetchJob`, `Monitor` for change notifications and so on) ([client docs](https://raw.githubusercontent.com/KDE/akonadi/master/docs/client_libraries.md)). Storage is MySQL, SQLite or PostgreSQL ([server docs](https://raw.githubusercontent.com/KDE/akonadi/master/docs/server.md)).
- **How Google gets in:** through the kdepim-runtime "Google Groupware" resource.
  - It uses LibKGAPI, which implements "Calendar API v3" ([LibKGAPI README](https://raw.githubusercontent.com/KDE/libkgapi/master/README.md)).
  - It stores the Calendar API `syncToken` as the collection's remote revision ([calendarhandler.cpp](https://raw.githubusercontent.com/KDE/kdepim-runtime/master/resources/google-groupware/calendarhandler.cpp)).
  - It requests account-info, `calendar`, `calendar.events`, People and Tasks scopes ([googlescopes.cpp](https://raw.githubusercontent.com/KDE/kdepim-runtime/master/resources/google-groupware/googlescopes.cpp)).
  - It keeps tokens in the system keychain through QtKeychain and uses KDE's own client ID ([googlesettings.cpp](https://raw.githubusercontent.com/KDE/kdepim-runtime/master/resources/google-groupware/googlesettings.cpp)).
- **Other providers** arrive through other resources in the same package, such as DAV groupware and iCal file ([dav](https://raw.githubusercontent.com/KDE/kdepim-runtime/master/resources/dav/CMakeLists.txt), [ical](https://raw.githubusercontent.com/KDE/kdepim-runtime/master/resources/ical/CMakeLists.txt)).
- **Plasma's clock calendar** reads events through the kdepim-addons `pimevents` plugin, which links `KPim6::AkonadiCore`, `KPim6::AkonadiCalendar` and `KF6::CalendarEvents` ([CMakeLists.txt](https://raw.githubusercontent.com/KDE/kdepim-addons/master/plugins/plasma/pimeventsplugin/CMakeLists.txt)).
- **KAccounts** has a Google provider definition that includes the `calendar` scope ([google.provider.in](https://raw.githubusercontent.com/KDE/kaccounts-providers/master/providers/google.provider.in)), but the Akonadi Google resource does its own OAuth rather than using it (googlesettings.cpp above).
- **From Rust:** a crates.io search for "akonadi" returned nothing ([search](https://crates.io/search?q=akonadi)). A C++ bridge would be needed; its feasibility was not investigated. **[unverified]**
- **Cost to hab-bot:** no Google project, since KDE's client handles Google. Desktop only. Depends on the user having set up the Google account in KDE's PIM stack. Other providers come for free.

### GNOME Evolution Data Server (EDS) and GNOME Online Accounts (GOA)

- **What it is:** "a unified backend for programs that work with contacts, tasks, calendar information, and notes". GNOME Shell, GNOME Calendar, elementary, Cinnamon and Phosh use it ([README](https://raw.githubusercontent.com/GNOME/evolution-data-server/master/README.md); master 3.63.1, [CMakeLists.txt](https://raw.githubusercontent.com/GNOME/evolution-data-server/master/CMakeLists.txt)).
- **No dedicated Google calendar backend.** The calendar backends are `caldav`, `contacts`, `file`, `gtasks`, `http`, `webdav-notes` and `weather` ([backends/CMakeLists.txt](https://raw.githubusercontent.com/GNOME/evolution-data-server/master/src/calendar/backends/CMakeLists.txt)). Google calendars go through CalDAV.
  - GOA's Google provider attaches `https://apidata.googleusercontent.com/caldav/v2/<email>/user` as the calendar URI.
  - It requests full `calendar` together with mail, contacts and tasks scopes.
  - Its redirect is the reversed client ID, `…:/oauth2redirect` ([goagoogleprovider.c](https://raw.githubusercontent.com/GNOME/gnome-online-accounts/master/src/goabackend/goagoogleprovider.c)).
- **Token sharing:** GOA's D-Bus interface `org.gnome.OnlineAccounts.OAuth2Based` has `GetAccessToken` ("Note that calls to this method are logged") and read-only `ClientId`/`ClientSecret` properties ([dbus-interfaces.xml](https://raw.githubusercontent.com/GNOME/gnome-online-accounts/master/data/dbus-interfaces.xml)). Another app in the session could therefore use GOA's Google token.
- **EDS's D-Bus API:** `org.gnome.evolution.dataserver.Calendar` (with `GetObjectList`, `GetView`, `GetFreeBusy` and so on) and `CalendarFactory` are defined under `src/private` and built into a library named `edbus-private` ([Calendar.xml](https://raw.githubusercontent.com/GNOME/evolution-data-server/master/src/private/org.gnome.evolution.dataserver.Calendar.xml), [src/private/CMakeLists.txt](https://raw.githubusercontent.com/GNOME/evolution-data-server/master/src/private/CMakeLists.txt)). The supported client API is presumably the C/GObject `libecal`. **[unverified]**
- **From Rust:** `gnome-online-accounts-rs` 0.0.1 (2023, "A very simple wrapper for the GNOME Online Accounts DBus API") exists. No EDS bindings were found ([crates.io search](https://crates.io/search?q=evolution-data-server)). `zbus` 5.19.0 is available for plain D-Bus.
- **Relevance on this desktop:** EDS and GOA are GNOME's stack. On KDE Plasma they are present only if installed. **[unverified]**

## 5. Where each approach runs, and what the server would hold

| Approach | Android device | Linux device | Server | What sits on the server |
|---|---|---|---|---|
| Calendar Provider | Yes | — | — | nothing, unless the device uploads events |
| Calendar API v3 | Yes (Play `AuthorizationClient`, or AppAuth with an unresolved client-type question) | Yes (loopback) | Yes | a user refresh token (scope as granted), or only an API key for public calendars |
| CalDAV | Yes (DAVx5 as a sync adapter, or hab-bot's own client) | Yes | Yes | a user refresh token for the full `calendar` scope, as seen in every implementation checked |
| Akonadi / EDS | — | Yes | — | nothing |

**Adding providers later:**

- The Calendar Provider, CalDAV and the desktop stores are provider-agnostic: new accounts show up through the same interface.
- The Calendar API is Google-only.
- Only CalDAV works in all three places (Android, Linux and the server). On Android, though, the least-work CalDAV route is DAVx5 feeding the Calendar Provider, which puts it back on the device only.

## 6. Rust and Tauri building blocks

Versions from the crates.io API, 2026-09-27:

| Need | Crate | Version (updated) | Notes |
|---|---|---|---|
| Calendar API client | [`google-calendar3`](https://crates.io/crates/google-calendar3) | 7.0.0+20251214 (2026-01-01) | generated from discovery (Byron/google-apis-rs) |
| | [`google-calendar`](https://crates.io/crates/google-calendar) | 0.10.0 (2025-12-30) | generated (oxidecomputer) |
| OAuth | [`oauth2`](https://crates.io/crates/oauth2) | 5.0.0 (2025-01-21) | "strongly-typed implementation of OAuth2 (RFC 6749)" |
| | [`yup-oauth2`](https://crates.io/crates/yup-oauth2) | 12.1.2 (2026-01-07) | device, installed-app and service-account flows; "The provider we have been testing the code against is also Google" |
| | [`tauri-plugin-oauth`](https://crates.io/crates/tauri-plugin-oauth) | 2.1.0 (2026-07-07) | "spawns a temporary localhost server to capture OAuth redirects" |
| | [`tauri-plugin-deep-link`](https://crates.io/crates/tauri-plugin-deep-link) | 2.5.0 (2026-09-26) | Linux, Windows, macOS, Android, iOS |
| | [`tauri-plugin-opener`](https://crates.io/crates/tauri-plugin-opener) | 2.6.0 (2026-09-26) | open a URL in the default browser |
| CalDAV | [`libdav`](https://crates.io/crates/libdav) | 0.11.0 (2026-09-05) | "CalDAV and CardDAV client implementations"; part of pimsync |
| | [`fast-dav-rs`](https://crates.io/crates/fast-dav-rs) | 0.17.0 (2026-09-10) | hyper + rustls |
| | [`minicaldav`](https://crates.io/crates/minicaldav) | 0.8.0 (2023-07-02) | |
| | [`kitchen-fridge`](https://crates.io/crates/kitchen-fridge) | 0.4.0 (2022-01-19) | |
| iCalendar and recurrence | [`icalendar`](https://crates.io/crates/icalendar) | 0.17.13 (2026-07-28) | builder and parser |
| | [`calcard`](https://crates.io/crates/calcard) | 0.3.14 (2026-09-15) | iCalendar/JSCalendar (Stalwart) |
| | [`ical`](https://crates.io/crates/ical) | 0.11.0 (2024-03-13) | parser |
| | [`rrule`](https://crates.io/crates/rrule) | 0.14.0 (2025-04-20) | RFC 5545 recurrence rules |
| D-Bus (GOA/EDS) | [`zbus`](https://crates.io/crates/zbus) | 5.19.0 (2026-08-09) | |
| | [`gnome-online-accounts-rs`](https://crates.io/crates/gnome-online-accounts-rs) | 0.0.1 (2023-07-02) | |
| Akonadi, EDS, Android `CalendarContract` | — | — | no bindings found |

## Gaps and open questions

- **Google policy pages not read.** Every **[unverified: search excerpt]** claim should be checked on the live page before a decision depends on it: testing-mode expiry, the personal-use exception, the unverified-app cap, sensitive-scope rules, the CalDAV guide, push requirements, quotas and redirect rules.
- **How the granular Calendar scopes are classified.** If `calendar.freebusy`, `calendar.events.public.readonly` or `calendar.calendarlist.readonly` are non-sensitive, apps that use only them avoid the sensitive-scope rules. Not confirmed.
- **Play-free OAuth on Android.** Can a newly created client use AppAuth with a custom-scheme or App Link redirect, and which client type (Android, iOS or Desktop) would it be? DAVx5 and GOA show working patterns, but those clients may be older or of another type. Does a Desktop-type client with loopback work from Android, and is that allowed?
- **Google's sync adapter without full Play services.** Does `GoogleCalendarSyncAdapter` work with microG, or under GrapheneOS's sandboxed Google Play? Does the Google Calendar app sync on its own without it? How often does it sync?
- **Device flow and Calendar scopes.** If the limited-input device flow allows a Calendar scope, the server could hold its own token without any redirect URI.
- **Server redirect URIs.** Would a self-hosted server with a LAN-only or private hostname be accepted as a web-client redirect? Can a refresh token obtained by a device's installed-app client be moved to and used by the server?
- **Public calendars without OAuth.** Does an API key alone read a public Google calendar? What are the terms of the public iCal address, and the "secret address in iCal format" for private calendars ([Google Calendar Help](https://support.google.com/calendar/answer/37648), blocked)? Can Workspace admins disable those?
- **Service accounts.** Can a consumer user share a calendar with a service account's email, so the server reads it with no user refresh token? Search results were inconclusive.
- **CalDAV details.** Does Google's CalDAV accept a read-only scope? Does it support server-side recurrence expansion and `sync-collection`? Do app passwords still work on consumer accounts?
- **Quota numbers** for projects created after May 1, 2026, and whether a personal project could ever reach the paid tier.
- **Client secrets in an open-source app.** vdirsyncer says the ToS forbids hard-coding them, while KDE and GNOME do it anyway. The ToS text was not read.
- **Desktop store practicalities.** Is Akonadi set up with Google on the target Plasma machine? How much work is a C++/Qt bridge from Rust? The supported (non-"private") EDS client API was not confirmed.
- **Refresh-token count.** 100 per account per client: if each device and the server sign in separately, how many tokens would a household use?
