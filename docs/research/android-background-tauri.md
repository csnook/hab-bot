# Running a Tauri 2 app in the background on Android

Research for [csnook/hab-bot#2](https://github.com/csnook/hab-bot/issues/2): what a Tauri 2 app can do on Android while it is closed or in the background, and how much of that comes from existing plugins versus Kotlin we would write.

- **Read on:** 2026-09-27. Versions and "last updated" dates of what was read are given in [Access notes](#access-notes).
- **Terminology:** this document uses the terms in `CONTEXT.md`. Android's own words clash with some of them, so:
  - **AlarmManager alarm** means Android's time-scheduling mechanism (`AlarmManager`). It is not an *alert*. An **alarm-style alert** means the loud, alarm-clock kind of *alert*.
  - **Android notification** means the OS object posted through `NotificationManager`, which is one way to deliver an *alert*.
  - Android's geofencing API talks about "triggers" (for example `INITIAL_TRIGGER_DWELL`). This document calls those **geofence transitions**, keeping *trigger* for the domain meaning.
- **Unverified claims:** this environment's proxy blocked `tauri.app` / `v2.tauri.app`, `github.com` (web pages and API), `docs.rs`, `support.google.com`, `play.google.com`, `developers.google.com`, `firebase.google.com`, `source.android.com` and `android.googlesource.com`. Tauri code was read from the published crates on crates.io, and the Tauri docs from their Markdown source on `raw.githubusercontent.com`. Android framework source was read from the `aosp-mirror` GitHub mirror, also through `raw.githubusercontent.com`. Claims that rest only on a search-engine excerpt are marked **[unverified: search excerpt]**. Claims that are inferences from reading source code, with no documentation behind them, are marked **[inference from source]**.

## Summary

- **Tauri's Rust side only starts when an Activity starts.** The app's Rust entry point (`tauri::Builder … run()`) is started from the first `Activity.onCreate`, and every Kotlin plugin object is constructed with that `Activity`. When Android wakes the app for an AlarmManager alarm, a broadcast, a job or a geofence, it starts the process and runs a Kotlin `BroadcastReceiver`, `Service` or `Worker`. Tauri does not start its runtime, webview or plugins for those. Tauri's docs say the way to run Rust from plugin code "even when the application WebView is suspended" is plain JNI that we write ourselves. So every background path on Android begins in Kotlin: a manifest-declared receiver, service or worker. That Kotlin either does the work itself or calls into our Rust library through JNI exports that we write.
- **Existing plugins cover little of this.**
  - The official **notification** plugin can schedule Android notifications through AlarmManager, and it restores them after a reboot. It does not declare an exact-alarm permission, so on Android 12+ it falls back to inexact, mostly non-wakeup alarms unless the app declares one. It has no `setAlarmClock`, no full-screen intent, no alarm category, no Do Not Disturb options, and no app code runs when an Android notification fires. Its action buttons always open the app.
  - The official **geolocation** plugin works only in the foreground, requires Google Play Services (GMS) to get positions, and has no geofencing.
  - No official plugin exists for Bluetooth, Wi-Fi, WorkManager or foreground services.
  - Community plugins cover some pieces: a foreground-service lifecycle with a JNI "headless core" pattern, a BLE client that works in the foreground, a battery-optimization prompt, and broadcast listening while the Activity lives. None covers alarm-style alerts, geofences, or watching Bluetooth or Wi-Fi in the background.
- **Exact-time alarms after close or reboot** are possible on the platform.
  - They need `SCHEDULE_EXACT_ALARM`, which the user must grant and which is denied by default on new installs (Android 14+, apps targeting 33+). The alternative is `USE_EXACT_ALARM`, which is granted automatically but which Google Play limits to alarm-clock and calendar apps. The Play policy page itself could not be read here.
  - `setAlarmClock()` fires on time through Doze. `setExactAndAllowWhileIdle()` fires in Doze but is rate-limited.
  - All AlarmManager alarms are cleared by a reboot, and on Android 15 also by a force-stop. They must be registered again from `BOOT_COMPLETED` or `LOCKED_BOOT_COMPLETED`.
- **Alarm-style alerts:**
  - Full-screen intents need `USE_FULL_SCREEN_INTENT`. For apps targeting Android 14+, Play grants it by default only to calling and alarm apps, and users can turn it off.
  - In AOSP's default Do Not Disturb policy, alarms are allowed and "reminders" are not. Android notifications with `CATEGORY_ALARM` or `USAGE_ALARM` audio count as alarms. "Total silence" blocks everything except calls.
  - From Android 17, audio played by the app while it is in the background needs a foreground service. Apps targeting API 37 also need either while-in-use capability or the exact-alarm permission combined with `USAGE_ALARM` audio.
- **Location triggers:**
  - With GMS: `GeofencingClient`, limited to 100 geofences per app. It needs fine plus background location. In the background, Android 8+ responds to transitions "every couple of minutes". Geofences must be registered again after a reboot.
  - Without GMS: the framework's `LocationManager.addProximityAlert` (API 1). In AOSP 15 its own location polling defaults to no more often than every 30 minutes. Otherwise the app would sample location itself, which Android 8+ limits to "a few times each hour" in the background.
- **Bluetooth:** `ACTION_ACL_CONNECTED` / `ACTION_ACL_DISCONNECTED` and the A2DP/headset connection-state broadcasts are exempt from Android 8's limits on manifest receivers. So a manifest receiver can see the car's Bluetooth connect while the app is closed. It needs `BLUETOOTH_CONNECT` on Android 12+. Companion Device Manager presence observation (API 31, reworked in 36) is another framework route.
- **Wi-Fi:** no Wi-Fi or connectivity broadcast reaches manifest receivers. `registerNetworkCallback(NetworkRequest, PendingIntent)` can wake the app when a matching network becomes available, but it reports only "available", not "lost". Reading the SSID needs location permission and location turned on.
- **Periodic work** such as a weather check every 30 minutes: WorkManager's minimum period is 15 minutes, and WorkManager is built on the framework `JobScheduler`, with no GMS dependency. Doze defers jobs to maintenance windows, and standby-bucket quotas cap them; the "rare" bucket has network access disabled. A fixed 30-minute cadence is not guaranteed.
- **Without GMS**, the things that change are geofencing (fall back to framework proximity alerts or our own sampling), FCM push, and the official geolocation plugin (which returns errors). AlarmManager alarms, Android notifications, WorkManager, Bluetooth and Wi-Fi callbacks, foreground services and Companion Device Manager are all framework or AndroidX APIs.

### At a glance

| Capability | Android mechanism | Existing Tauri plugin? | Needs GMS? | Key permissions / versions |
|---|---|---|---|---|
| Exact-time alert after close/reboot | `AlarmManager.setAlarmClock` / `setExactAndAllowWhileIdle` + boot receiver | Partly (official notification plugin: fixed content, inexact unless the app adds the permission) | No | `SCHEDULE_EXACT_ALARM` (12+) or `USE_EXACT_ALARM` (13+, Play-restricted); `RECEIVE_BOOT_COMPLETED`; `POST_NOTIFICATIONS` (13+) |
| Alarm-style alert | Full-screen intent, `CATEGORY_ALARM`, `USAGE_ALARM` audio, often a foreground service | No | No | `USE_FULL_SCREEN_INTENT` (restricted on 14+); foreground-service type; Android 17 background-audio rules |
| Arrive/leave a place | GMS `GeofencingClient`, or framework `addProximityAlert`, or own sampling | No (geolocation plugin is foreground-only and GMS-only) | Geofencing API yes; fallbacks no | `ACCESS_FINE_LOCATION` + `ACCESS_BACKGROUND_LOCATION` (10+; granted in Settings on 11+) |
| Bluetooth connect/disconnect | Manifest receiver for `ACL_CONNECTED`/`ACL_DISCONNECTED`; CDM presence | No (blec is a foreground BLE GATT client) | No | `BLUETOOTH_CONNECT` (12+) |
| Wi-Fi connect | `registerNetworkCallback(…, PendingIntent)`; in-process `NetworkCallback` | No | No | `ACCESS_NETWORK_STATE`; SSID needs location permission + location on |
| Periodic work | WorkManager `PeriodicWorkRequest` (≥15 min) | No | No | none extra; `INTERNET` for fetching |

## 1. How Tauri code runs on Android

This section is the background the other sections depend on. It determines where evaluation can run when the app is not open.

### What starts the Rust side

- The generated Android project is an ordinary Android Studio project. Tauri's docs say "Tauri uses an Android Studio project under the hood, so any official practice for building and publishing Android apps also apply to your app" ([tauri-docs `distribute/google-play.mdx`](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/distribute/google-play.mdx)). The minimum supported Android version is "Android 7.0 (codename Nougat, SDK 24)" (same page).
- **The Rust library is loaded, and the app's Rust `main` started, from the Activity:**
  - The Kotlin `Rust` object calls `System.loadLibrary("{{library}}")` in its initializer ([wry `src/android/kotlin/Rust.kt` L14-L17](https://github.com/tauri-apps/wry/blob/792d0359ba6501a4fc360ece17de2ae42329a47c/src/android/kotlin/Rust.kt#L14-L17)).
  - `WryActivity.onCreate` calls `Rust.onCreate(this)` and registers a `ProcessLifecycleOwner` observer. That observer calls `Rust.onFirstActivityCreate()` once per process ([wry `WryActivity.kt` L23-L30, L109-L129](https://github.com/tauri-apps/wry/blob/792d0359ba6501a4fc360ece17de2ae42329a47c/src/android/kotlin/WryActivity.kt#L23-L30)).
  - In tao, `onFirstActivityCreate` spawns a thread that runs the app's `main()` ([tao `ndk_glue.rs` L331-L387](https://github.com/tauri-apps/tao/blob/37b7e8bc90a050e93be988df636f322c3ef147b6/src/platform_impl/android/ndk_glue.rs#L331-L387)). tao's macro docs describe `main` as "the main entry point of your android application that runs in `onFirstActivityCreate` after `onCreate`" ([same file, L55-L74](https://github.com/tauri-apps/tao/blob/37b7e8bc90a050e93be988df636f322c3ef147b6/src/platform_impl/android/ndk_glue.rs#L55-L74)).
  - `#[tauri::mobile_entry_point]` wires the app's `run()` in as that `main` through `tauri::android_binding!` ([tauri-macros `mobile.rs` L80](https://github.com/tauri-apps/tauri/blob/e60834fc67d87c10e2f44b2568052295cb61c325/crates/tauri-macros/src/mobile.rs#L80); [tauri `lib.rs` L141-L151](https://github.com/tauri-apps/tauri/blob/447fa9f3f993fe77724189e355078b38ce20baea/crates/tauri/src/lib.rs#L141-L151)).
- **Kotlin plugins are bound to the Activity:**
  - `abstract class Plugin(private val activity: Activity)` ([tauri `Plugin.kt` L45](https://github.com/tauri-apps/tauri/blob/447fa9f3f993fe77724189e355078b38ce20baea/crates/tauri/mobile/android/src/main/java/app/tauri/plugin/Plugin.kt#L45)).
  - Rust instantiates each plugin with the constructor signature `(Landroid/app/Activity;)V` and loads it through the Activity's `getPluginManager()` ([tauri `plugin/mobile.rs` L208-L248](https://github.com/tauri-apps/tauri/blob/447fa9f3f993fe77724189e355078b38ce20baea/crates/tauri/src/plugin/mobile.rs#L208-L248)).
  - The docs' plugin template has the same shape ([tauri-docs `develop/Plugins/develop-mobile.mdx`](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/develop/Plugins/develop-mobile.mdx)).
- **Result [inference from source]:** Android can create the app process without an Activity, for example to deliver an AlarmManager `PendingIntent` to a receiver, run a job, or deliver a geofence transition. In that case none of the code paths above run: no Tauri `App`, no plugins, no webview, no Rust state.

### How Rust can still run in the background

- Tauri's docs, in "Calling Rust From Mobile Plugins": "While Tauri doesn't directly provide a mechanism to call Rust from your plugin code, using JNI on Android and FFI on iOS allows plugins to call shared code, even when the application WebView is suspended." The example calls `System.loadLibrary("app_lib")` in Kotlin and exports `Java_<package>_<class>_<method>` from Rust with the `jni` crate ([tauri-docs `develop-mobile.mdx`, "Calling Rust From Mobile Plugins"](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/develop/Plugins/develop-mobile.mdx)).
- The community plugin `tauri-plugin-background-service` 1.0.1 (2026-07-30) uses this pattern for its "headless" mode. Its Kotlin `HeadlessBridge` `dlopen`s a host-app library (`nativeLibName = "app_core"`) whose Rust exports `Java_app_tauri_backgroundservice_HeadlessBridge_*`. The plugin itself "ships NO native library" ([HeadlessBridge.kt L62-L78](https://github.com/dardourimohamed/tauri-background-service/blob/197f08ab1efb35df7524c1371383471c9118e481/tauri-plugin-background-service/android/src/main/kotlin/app/tauri/backgroundservice/HeadlessBridge.kt#L62-L78)). Its README warns: "When the service is restarted by the OS, the Rust process is new. Persist any state you need to restore" ([README L311](https://github.com/dardourimohamed/tauri-background-service/blob/197f08ab1efb35df7524c1371383471c9118e481/tauri-plugin-background-service/README.md)).
- **Trade-off for "where evaluation happens":** when Android wakes the app for a trigger, there are three options.
  1. Evaluate in Kotlin, duplicating logic that also exists in Rust or TypeScript.
  2. Call a Rust evaluation function through JNI exports that run without the Tauri runtime. That function would have to open storage itself, because Tauri's managed state isn't there.
  3. Only record the trigger and evaluate the next time the Activity runs, which is late.

  Evaluating in the TypeScript UI is possible only while the Activity and webview are alive.

### Versions in play

- tauri 2.12.0 and tauri-plugin-notification / tauri-plugin-geolocation 2.5.0 / 2.4.0 were all released 2026-09-26 ([crates.io tauri](https://crates.io/crates/tauri), [notification](https://crates.io/crates/tauri-plugin-notification), [geolocation](https://crates.io/crates/tauri-plugin-geolocation)).
- Tauri 3.0 alphas exist (tauri 3.0.0-alpha.3 on 2026-09-26; plugins 3.0.0-alpha.1 on 2026-09-21). Their Android notification and geolocation Kotlin matches 2.x apart from build settings and a few bug fixes not yet carried over (diff of the crate tarballs).

## 2. Exact-time alarms that fire after the app is closed or the phone restarts

### Platform facts

- **AlarmManager runs outside the app.** Alarms "operate outside of your application, so you can use them to trigger events or actions even when your app is not running, and even if the device itself is asleep" ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule)).
- **Inexact alarms are late by design.**
  - On Android 12+, `set()`, `setInexactRepeating()` and `setAndAllowWhileIdle()` fire "within one hour of the supplied trigger time, unless any battery-saving restrictions are in effect".
  - `setWindow()` windows under 10 minutes are "typically clipped" to 10 minutes.
  - "on Android 4.4 (API level 19) and higher, all repeating alarms are inexact alarms."

  (All from [Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule).)
- **Non-wakeup types wait for the device.** `RTC` and `ELAPSED_REALTIME` do not wake the device. `RTC_WAKEUP` and `ELAPSED_REALTIME_WAKEUP` do (same page).
- **The exact-alarm APIs** ([same page](https://developer.android.com/develop/background-work/services/alarms/schedule); [AlarmManager reference](https://developer.android.com/reference/android/app/AlarmManager)):
  - `setExact()`: "nearly precise … as long as other battery-saving measures aren't in effect". Doze defers it to the next maintenance window ([Doze](https://developer.android.com/training/monitoring-device-state/doze-standby)).
  - `setExactAndAllowWhileIdle()`: fires in Doze. The reference says the system "will not dispatch these alarms more than about every minute … when in low-power idle modes this duration may be significantly longer, such as 15 minutes". The Doze page says "Neither setAndAllowWhileIdle() nor setExactAndAllowWhileIdle() can fire alarms more than once per nine minutes, per app". The power-limits table says "While-idle alarms: Limited to 7 per hour" during Doze ([Power management resource limits](https://developer.android.com/topic/performance/power/power-details)). These three first-party figures do not agree with each other.
  - `setAlarmClock()`: "the system never adjusts their delivery time … leaves low-power modes if necessary". Doze: "Alarms set with setAlarmClock() continue to fire normally. The system exits Doze shortly before those alarms fire." The system may also show the user the upcoming alarm ([AlarmManager reference](https://developer.android.com/reference/android/app/AlarmManager)).
- **Standby-bucket quotas do not apply to alarm clocks.** The power-limits table caps alarms by bucket (working set 10/hour, frequent 2/hour, rare 1/hour, restricted 1/day) ([Power management resource limits](https://developer.android.com/topic/performance/power/power-details)). In AOSP 15, `isExemptFromAppStandby` returns true for alarm clocks and for allow-while-idle alarms ([AlarmManagerService.java L4330-L4333, android15-release](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/apex/jobscheduler/service/java/com/android/server/alarm/AlarmManagerService.java)) **[inference from source; not stated in the docs]**.
- **Exact alarms are exempt from foreground-service launch restrictions**: "exact alarms aren't affected by foreground service launch restrictions" ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule)). "Your app invokes an exact alarm to complete an action that the user requests" is a listed exemption ([Restrictions on starting a foreground service from the background](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).

### Permissions and Play policy

- **API 31+ needs a permission.** Apps targeting Android 12+ "must declare one of the 'Alarms & reminders' permissions", or `setExact()`, `setExactAndAllowWhileIdle()` and `setAlarmClock()` throw `SecurityException` ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule); [Android 14 exact-alarm change](https://developer.android.com/about/versions/14/changes/schedule-exact-alarms)). There is one exception: "If the exact alarm is set using an OnAlarmListener object … the SCHEDULE_EXACT_ALARM permission isn't required". A listener only works while the process is alive (same pages). API 37 adds a listener/`Executor` overload of `setExactAndAllowWhileIdle` ([AlarmManager reference](https://developer.android.com/reference/android/app/AlarmManager)).
- **`SCHEDULE_EXACT_ALARM`** (API 31+):
  - "Granted by the user", from the "Alarms & reminders" special-access screen (intent `ACTION_REQUEST_SCHEDULE_EXACT_ALARM`).
  - It is "not pre-granted to fresh installs of apps targeting Android 13 (API level 33) and higher" on Android 14+, and a backup/restore to an Android 14 device leaves it denied ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule); [Android 14 change](https://developer.android.com/about/versions/14/changes/schedule-exact-alarms)).
  - "When the SCHEDULE_EXACT_ALARM permission is revoked for your app, your app stops, and all future exact alarms are canceled." When it is granted, the app gets `ACTION_SCHEDULE_EXACT_ALARM_PERMISSION_STATE_CHANGED` and should reschedule ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule)).
- **`USE_EXACT_ALARM`** (API 33+):
  - "Granted automatically", "Cannot be revoked by the user", "Limited use cases" ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule)).
  - "Calendar or alarm clock apps … can request the USE_EXACT_ALARM normal permission … Apps will not be able to publish a version of their app with this permission in the manifest unless they qualify based on the policy language" ([Android 14 change](https://developer.android.com/about/versions/14/changes/schedule-exact-alarms)).
  - The linked Play policy ([Play Console Help 12253906](https://support.google.com/googleplay/android-developer/answer/12253906#exact_alarm_preview)) was **blocked**. Search excerpts describe it as limited to apps whose core function is "an alarm or timer app" or "a calendar app that shows event notifications" **[unverified: search excerpt of https://support.google.com/googleplay/android-developer/answer/16558241]**. Whether a reminder app qualifies could not be checked.
- **Battery-optimization exemption.** The reference says `setExactAndAllowWhileIdle` needs `SCHEDULE_EXACT_ALARM` "unless the app is exempt from battery restrictions" ([AlarmManager reference](https://developer.android.com/reference/android/app/AlarmManager)). In AOSP 15, `canScheduleExactAlarms()` returns true when the app is on the device-idle allowlist ([AlarmManagerService.java L2689-L2697, L2858-L2872](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/apex/jobscheduler/service/java/com/android/server/alarm/AlarmManagerService.java)) **[inference from source]**. Asking for that exemption directly (`ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`) is restricted by Play: "Google Play policies prohibit apps from requesting direct exemption … unless the core function of the app is adversely affected" ([Doze](https://developer.android.com/training/monitoring-device-state/doze-standby)).
- **Standby bucket side effect.** In AOSP 15, holding `USE_EXACT_ALARM` pins the app's standby bucket to working set or better. `SCHEDULE_EXACT_ALARM` does so only for apps targeting below Android 14 (compat change `SCHEDULE_EXACT_ALARM_DOES_NOT_ELEVATE_BUCKET`, enabled from target 34) ([AlarmManagerService.java `shouldGetBucketElevation`](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/apex/jobscheduler/service/java/com/android/server/alarm/AlarmManagerService.java); [AlarmManager.java L303-L312](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/apex/jobscheduler/framework/java/android/app/AlarmManager.java)) **[inference from source]**.
- **Target API for Play.** From August 31, 2026, new apps and updates must target Android 16 (API 36) ([Target API level requirements](https://developer.android.com/google/play/requirements/target-sdk)). All the "targets 31/33/34" rules above therefore apply to a Play build.

### Reboot, force-stop and time changes

- **Reboot clears alarms.** "By default, all alarms are canceled when a device shuts down." Apps must re-register them from a `BOOT_COMPLETED` receiver with `RECEIVE_BOOT_COMPLETED`, which "only works if the app has already been launched by the user at least once" ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule)).
- **Direct Boot.** Before the first unlock after a reboot, apps don't run unless their components are `directBootAware`. Such components get `ACTION_LOCKED_BOOT_COMPLETED` and can use only device-encrypted storage. The docs list "Apps that have scheduled notifications, such as alarm clock apps" as a use case ([Direct Boot](https://developer.android.com/privacy-and-security/direct-boot)).
- **Force-stop (Android 15).** "the system also cancels all pending intents when the app enters the stopped state". When the user leaves the stopped state, "the ACTION_BOOT_COMPLETED broadcast is delivered to the app providing an opportunity to re-register any pending intents" ([Android 15 behavior changes, all apps](https://developer.android.com/about/versions/15/behavior-changes-all)).
- **Clock changes.** `TIME_SET`, `ACTION_TIMEZONE_CHANGED` and `ACTION_NEXT_ALARM_CLOCK_CHANGED` can still be received by manifest receivers: "Clock apps might need to receive these broadcasts to update alarms" ([Implicit broadcast exceptions](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions)).

### What the official notification plugin does (tauri-plugin-notification 2.5.0)

Source read from the crate tarball. Links are pinned to plugins-workspace commit [`a2364a5`](https://github.com/tauri-apps/plugins-workspace/tree/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification).

- **Scheduling happens only on mobile.** "Scheduling is only implemented on mobile; the desktop implementation delivers the notification immediately and ignores the schedule" ([`src/models.rs` L132-L137](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/src/models.rs#L132-L137)). The schedule kinds are `At { date, repeating, allow_while_idle }`, `Interval { … }` (calendar-field match) and `Every { interval, count, … }`. On Android a "month" is approximated as 30 days and a "year" as 52 weeks ([`models.rs` L59-L66](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/src/models.rs#L59-L66)).
- **The manifest declares no exact-alarm permission.** The plugin manifest declares `RECEIVE_BOOT_COMPLETED`, `WAKE_LOCK` and `POST_NOTIFICATIONS`, but neither `SCHEDULE_EXACT_ALARM` nor `USE_EXACT_ALARM` ([AndroidManifest.xml](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/AndroidManifest.xml)).
- **Which AlarmManager call it makes** ([`TauriNotificationManager.kt` L314-L384](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L314-L384)):
  - One-shot `At`, and the first `Interval` firing, go through `setExactIfPossible`:
    - if `canScheduleExactAlarms()` is false on API 31+: `setAndAllowWhileIdle(RTC_WAKEUP)` when `allowWhileIdle`, otherwise `set(RTC)`, which is inexact and non-wakeup;
    - if exact alarms are allowed: `setExactAndAllowWhileIdle(RTC_WAKEUP)` when `allowWhileIdle`, otherwise `setExact(RTC)`, which is non-wakeup.
  - `At` with `repeating`, and `Every`, use `setRepeating(AlarmManager.RTC, …)`: non-wakeup, and inexact on API 19+.
  - Each later `Interval` firing is rescheduled from the receiver with `setExact(RTC)` or `set(RTC)`, without allow-while-idle, even if the first one had it ([L508-L533](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L508-L533)).
  - `setAlarmClock` is never used.
- **No app code runs at fire time.** The Android notification is built completely when it is scheduled, parceled into the `PendingIntent`, and posted by `TimedNotificationPublisher` when it fires ([L314-L329, L469-L500](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L469-L500)). The JS `notification` event is emitted only if a plugin instance exists, meaning the Activity is running ([`NotificationPlugin.kt` L87-L93](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/NotificationPlugin.kt#L87-L93)). So conditions cannot be checked at the moment it fires.
- **Reboot restore.** `LocalNotificationRestoreReceiver` listens for `LOCKED_BOOT_COMPLETED`, `BOOT_COMPLETED` and `QUICKBOOT_POWERON`, and is `directBootAware`. But it returns early if the user is not yet unlocked, so in practice it reschedules at `BOOT_COMPLETED`, after unlock. Any `At` notification whose time passed while the phone was off is moved to 15 seconds after boot ([L541-L581](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L541-L581)). Pending notifications are stored in regular `SharedPreferences` ([`NotificationStorage.kt` L77-L78](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/NotificationStorage.kt#L77-L78)), which is credential-encrypted storage per [Direct Boot](https://developer.android.com/privacy-and-security/direct-boot).
- **Actions always open the app.** Action buttons are `PendingIntent.getActivity(...)`, so every action, such as a "Done" or "Snooze", opens the app UI ([L227-L264](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L227-L264)). The Android `NotificationAction` model has only `id`, `title` and `input` ([`NotificationPlugin.kt` L46-L51](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/NotificationPlugin.kt#L46-L51)), even though the docs list a `foreground` property ([tauri-docs `plugin/notification.mdx`](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/plugin/notification.mdx)).
- **Implication [inference from source]:** declaring `SCHEDULE_EXACT_ALARM` or `USE_EXACT_ALARM` in the app's own manifest, which Android merges with the plugin's, would make `canScheduleExactAlarms()` true and turn on the exact paths. It would still not give wakeup alarms for `allowWhileIdle: false`, or `setAlarmClock` behavior.
- **Community fork.** `tauri-plugin-notifications` 0.5.0-rc.14 (Choochmeque, 2026-09-16) is a fork with the same scheduling code. It adds FCM and APNs push, and its Gradle file depends on `firebase-messaging` ([build.gradle.kts L74-L75](https://github.com/Choochmeque/tauri-plugin-notifications/blob/55be77f158b7b0ab0238861db079bd9f3f995c95/android/build.gradle.kts#L74-L75)). Its README says Firebase is left out when the Cargo feature is off; that was not checked.

## 3. Alerts that behave like an alarm clock

### Full-screen display

- **Android 10+ blocks background Activity starts.** Starting an Activity from the background was the pre-Android 10 approach; the docs now show the notification-based way for Android 10+ ([Display time-sensitive notifications](https://developer.android.com/develop/ui/views/notifications/time-sensitive)). "If your notification includes a full-screen intent to display an activity when the device is locked, the system UI displays a heads-up notification instead while the user actively uses the device" (same page).
- **Android 14 restricts `USE_FULL_SCREEN_INTENT`.** "For apps targeting Android 14 (API level 34) or higher, apps that are allowed to use this permission are limited to those that provide calling and alarms only. The Google Play Store revokes default USE_FULL_SCREEN_INTENT permissions for any apps that don't fit this profile." Users can toggle it. Apps can check `NotificationManager.canUseFullScreenIntent()` and send users to `ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT` ([Android 14 behavior changes](https://developer.android.com/about/versions/14/behavior-changes-14)). Without it, the "notification will show up as an expanded heads up notification on lockscreen" ([NotificationManager reference, `canUseFullScreenIntent`](https://developer.android.com/reference/android/app/NotificationManager)). The Play declaration process ("Understanding foreground service and full-screen intent requirements") is on support.google.com, which was **blocked**.
- **No existing plugin sets a full-screen intent.** The official notification plugin never calls `setFullScreenIntent` or `setCategory` and always uses `PRIORITY_DEFAULT` ([`TauriNotificationManager.kt` L149-L224](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L149-L224)). `tauri-plugin-background-service` declares `USE_FULL_SCREEN_INTENT`, but for incoming-call UI ([its manifest](https://github.com/dardourimohamed/tauri-background-service/blob/197f08ab1efb35df7524c1371383471c9118e481/tauri-plugin-background-service/android/src/main/AndroidManifest.xml)).

### Sound through Do Not Disturb

- **The DND modes:**
  - `INTERRUPTION_FILTER_PRIORITY` suppresses all but notifications matching the priority criteria.
  - `INTERRUPTION_FILTER_ALARMS` suppresses everything "except those of category Notification.CATEGORY_ALARM".
  - `INTERRUPTION_FILTER_NONE` suppresses everything and mutes "all audio streams (except those used for phone calls)".

  ([NotificationManager reference](https://developer.android.com/reference/android/app/NotificationManager))
- **What counts as an alarm in AOSP.** In AOSP 15, `ZenModeFiltering.isAlarm()` is true for `CATEGORY_ALARM` notifications or those whose audio attributes usage is `USAGE_ALARM`. In priority mode, alarms pass when `policy.allowAlarms()`. In `ZEN_MODE_NO_INTERRUPTIONS` everything is intercepted ("#notevenalarms") ([ZenModeFiltering.java L165-L210, L338-L341](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/services/core/java/com/android/server/notification/ZenModeFiltering.java)). A notification's audio attributes come from its channel on Android 8+ ([NotificationRecord.java](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/services/core/java/com/android/server/notification/NotificationRecord.java)).
- **AOSP's default DND policy** has `alarms="true" … reminders="false" events="false"` ([default_zen_mode_config.xml, android15-release](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/core/res/res/xml/default_zen_mode_config.xml)). So with stock defaults an Android notification categorized as a reminder is blocked by DND and one categorized as an alarm is not. OEM builds may change these defaults; that was not checked. Android 9 added `PRIORITY_CATEGORY_ALARMS` ([Android 9 features](https://developer.android.com/about/versions/pie/android-9.0)).
- **Channel-level DND bypass.** `NotificationChannel.setBypassDnd()` can be set only by "Apps with Do Not Disturb policy access (see NotificationManager.isNotificationPolicyAccessGranted()) … but only if the channel hasn't been updated by the user since its creation" ([NotificationChannel reference](https://developer.android.com/reference/android/app/NotificationChannel)). Policy access is granted by the user in Settings (`ACTION_NOTIFICATION_POLICY_ACCESS_SETTINGS`) ([NotificationManager reference](https://developer.android.com/reference/android/app/NotificationManager)).
- **Android 15 changed global DND control.** Apps targeting 35+ "can no longer change the global state or policy of Do Not Disturb … Instead, apps must contribute an AutomaticZenRule" ([Android 15 behavior changes](https://developer.android.com/about/versions/15/behavior-changes-15)).
- **What the official plugin already sets:**
  - Its default channel uses `USAGE_ALARM` audio attributes, but only when the plugin config sets a default `sound` ([`TauriNotificationManager.kt` L92-L116](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/TauriNotificationManager.kt#L92-L116)). Under the AOSP logic above, such notifications would count as alarms for DND **[inference from source]**.
  - Channels made with its `createChannel` use `USAGE_NOTIFICATION`, and expose no DND bypass, category or audio-usage option ([`ChannelManager.kt` L34-L45, L93-L105](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/ChannelManager.kt#L34-L105)).
  - The default channel is created in `load()`, so only after the Activity has run once ([`NotificationPlugin.kt` L95-L118](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/java/NotificationPlugin.kt#L95-L118)).

### Playing sound and keeping it going

- **Android 17 background audio hardening.** On all apps running on Android 17, background audio calls (playback, focus, volume) need "a visible activity or … a foreground service that is not of type SHORT_SERVICE". Apps targeting API 37 also need a foreground service with while-in-use capabilities, "However, the requirement … is waived if the app has been granted the exact alarm permission, and it is making changes to audio streams that have the USAGE_ALARM attribute". Otherwise the calls "fail silently". Example: "if your app starts a foreground service in response to BOOT_COMPLETE and attempts to interact with audio, it will be suppressed" ([Background audio hardening](https://developer.android.com/about/versions/17/changes/bg-audio)). The page covers audio the app plays itself; whether it affects a channel sound played by the system for an Android notification is not stated.
- **Foreground-service types for alarm apps.**
  - `systemExempted` is allowed for "Apps holding SCHEDULE_EXACT_ALARM or USE_EXACT_ALARM permission" ([Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types)).
  - `mediaPlayback` and `specialUse` are the other candidates. `specialUse` requires a free-form justification reviewed in Play Console.
  - Apps targeting 34+ must declare their FGS types in Play Console (same page).
  - Starting a foreground service from the background is allowed when an exact alarm fires ([Restrictions on starting a FGS from the background](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).
- **Existing plugins.** None plays looping alarm audio. `tauri-plugin-timer` 0.1.0 shows the Kotlin-FGS pattern for an ongoing notification (`specialUse`), with buttons that reach the app through a `BroadcastReceiver` ([manifest](https://github.com/WhoStoleMySleep/tauri-plugin-timer/blob/09f2f17acba147d62300bf11d6a4c09831b47dee/android/src/main/AndroidManifest.xml)).

## 4. Location triggers (arriving at or leaving a place)

### With Google Play Services: the Geofencing API

- **It is a GMS API.** It uses `LocationServices.getGeofencingClient(...)`, which is in `com.google.android.gms`. Registered geofences "are kept in the com.google.process.location process owned by the com.google.android.gms package" ([Create and monitor geofences](https://developer.android.com/develop/sensors-and-location/location/geofencing)).
- **Limits and behavior** (same page):
  - "a limit of 100 per app, per device user".
  - Transitions can be enter, exit or dwell (loitering delay).
  - Recommended radius "between 100 - 150 meters".
  - Responsiveness is configurable, but lower values don't guarantee faster alerts.
  - "On Android 8.0 (API level 26) and higher, if an app is running in the background while monitoring a geofence, then the device responds to geofencing events every couple of minutes."
  - "On most devices, the geofence service uses only network location for geofence triggering."
- **Permissions:** `ACCESS_FINE_LOCATION`, plus `ACCESS_BACKGROUND_LOCATION` when targeting Android 10+ (same page). On Android 11+ the permission dialog has no "Allow all the time" option; users must turn it on in Settings ([Request background location](https://developer.android.com/develop/sensors-and-location/location/permissions/background)).
- **Re-registration.** GMS restores geofences itself after a GMS upgrade or restart. The app must register them again after a device reboot, a reinstall, data clearing, GMS data clearing, or `GEOFENCE_NOT_AVAILABLE` ([geofencing, "Re-register geofences only when required"](https://developer.android.com/develop/sensors-and-location/location/geofencing)).
- **Starting work from a transition.** A geofence transition is an allowed reason to start a foreground service from the background ([FGS background-start exemptions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).
- **Play policy.** "The Google Play store has updated its policy concerning device location, restricting background location access to apps that need it for their core functionality and meet related policy requirements" ([Access location in the background](https://developer.android.com/develop/sensors-and-location/location/background)). The policy page itself is on support.google.com and was **blocked**.

### Without Google Play Services

- **Framework proximity alerts.** `LocationManager.addProximityAlert(lat, lon, radius, expiration, PendingIntent)` has existed since API level 1. It fires the `PendingIntent` with `KEY_PROXIMITY_ENTERING` true on entry and false on exit. From API 17 it needs `ACCESS_FINE_LOCATION`. "if the device passes through the given area briefly, it is possible that no Intent will be fired" ([LocationManager reference](https://developer.android.com/reference/android/location/LocationManager)).
- **How AOSP 15 implements proximity alerts** **[inference from source]** ([GeofenceManager.java](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/services/core/java/com/android/server/location/geofence/GeofenceManager.java); [SystemSettingsHelper.java](https://github.com/aosp-mirror/platform_frameworks_base/blob/android15-release/services/core/java/com/android/server/location/injector/SystemSettingsHelper.java)):
  - System server's `GeofenceManager` requests `FUSED_PROVIDER` locations at an interval of `min(2 h, max(proximity-alert throttle, distance-to-nearest-fence ÷ 100 m/s))`.
  - The throttle `LOCATION_BACKGROUND_THROTTLE_PROXIMITY_ALERT_INTERVAL_MS` defaults to 30 minutes.
  - It sets `setMinUpdateIntervalMillis(0)`, so it also receives fixes that other apps' requests cause.
  - Registration checks only `PERMISSION_FINE`; no background-location check was found in this path.
  - Registrations are in-memory in system server; nothing persists them across reboot. OEM or GMS-less ROM behavior may differ.
- **`FUSED_PROVIDER`** ("fused", API 31) is a framework constant: "If present, this provider may combine inputs from several other location providers" ([LocationManager reference](https://developer.android.com/reference/android/location/LocationManager)).
- **Sampling location ourselves:**
  - "Android 8.0 (API level 26) limits how frequently an app can retrieve the user's current location while the app is running in the background … apps can receive location updates only a few times each hour" ([Background Location Limits](https://developer.android.com/about/versions/oreo/background-location-limits)).
  - A `location`-type foreground service lifts this, but "you cannot create a location foreground service while your app is in the background, unless you've been granted the ACCESS_BACKGROUND_LOCATION runtime permission" ([FGS types](https://developer.android.com/develop/background-work/services/fgs/service-types)).
  - The same page suggests the geofence API as the alternative for "when the user reaches specific locations".
- **Not researched:** microG's geofencing on de-Googled ROMs was not examined (see Gaps).

### Existing Tauri plugins

- **The official geolocation plugin** (2.4.0) has no geofencing and no background location:
  - It depends on `com.google.android.gms:play-services-location:21.3.0` ([build.gradle.kts L36](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/geolocation/android/build.gradle.kts#L36)).
  - `getCurrentPosition` and `watchPosition` return "Google Play Services unavailable." without GMS. Only the internal `getLastLocation` uses the framework `LocationManager` ([Geolocation.kt L35-L143](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/geolocation/android/src/main/java/Geolocation.kt#L35-L143)).
  - It clears all location updates in `onPause` "to avoid possible background location calls" ([GeolocationPlugin.kt L71-L75](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/geolocation/android/src/main/java/GeolocationPlugin.kt#L71-L75)).
  - It declares only coarse and fine location ([manifest](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/geolocation/android/src/main/AndroidManifest.xml); [tauri-docs `plugin/geolocation.mdx`](https://github.com/tauri-apps/tauri-docs/blob/v2/src/content/docs/plugin/geolocation.mdx)).
- **No geofencing plugin was found** on crates.io (searches for "tauri geofence", "tauri geofencing", "tauri location", 2026-09-27).

## 5. Watching Bluetooth and Wi-Fi connections in the background

### Bluetooth

- **Manifest receivers work for connection changes.** Since Android 8, apps targeting 26+ can't declare manifest receivers for implicit broadcasts. The exceptions include "BluetoothHeadset.ACTION_CONNECTION_STATE_CHANGED, BluetoothA2dp.ACTION_CONNECTION_STATE_CHANGED, ACTION_ACL_CONNECTED, ACTION_ACL_DISCONNECTED" ([Implicit broadcast exceptions](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions)). A manifest receiver therefore starts the app process when a paired device such as a car stereo connects or disconnects, even if the app is closed.
- **Permission.** `ACTION_ACL_CONNECTED` "For apps targeting Build.VERSION_CODES.S or higher … requires the Manifest.permission.BLUETOOTH_CONNECT permission" ([BluetoothDevice reference](https://developer.android.com/reference/android/bluetooth/BluetoothDevice)). `BLUETOOTH_CONNECT`, `BLUETOOTH_SCAN` and `BLUETOOTH_ADVERTISE` are runtime permissions in the "Nearby devices" group on Android 12+ ([Bluetooth permissions](https://developer.android.com/develop/connectivity/bluetooth/bt-permissions)).
- **Companion Device Manager (CDM).** CDM is a framework API.
  - API 31: `startObservingDevicePresence(String)`. The app "doesn't need to remain running in order to receive its callbacks". It needs `REQUEST_OBSERVE_COMPANION_DEVICE_PRESENCE`, a prior CDM association (a system pairing dialog), and `FEATURE_COMPANION_DEVICE_SETUP`.
  - API 36 deprecates that in favor of `startObservingDevicePresence(ObservingDevicePresenceRequest)`. For classic Bluetooth this "is triggered when the device connects/disconnects". For BLE the system scans. "WiFi devices are not supported."

  ([CompanionDeviceManager reference](https://developer.android.com/reference/android/companion/CompanionDeviceManager); [Companion device pairing](https://developer.android.com/develop/connectivity/bluetooth/companion-device-pairing)). CDM with the right permission is also an exemption for starting foreground services from the background ([FGS restrictions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).
- **BLE scanning in the background.** `BluetoothLeScanner.startScan(filters, settings, PendingIntent)` is meant for when "your process is not always running and it should be started when scan results are available". It needs `BLUETOOTH_SCAN`, and location permission unless the app declares `neverForLocation` ([BluetoothLeScanner reference](https://developer.android.com/reference/android/bluetooth/le/BluetoothLeScanner); [Bluetooth permissions](https://developer.android.com/develop/connectivity/bluetooth/bt-permissions)).
- **Existing plugins cover none of this:**
  - `tauri-plugin-blec` 0.17.0 is a BLE GATT client. It scans with a callback (not a `PendingIntent`), has no service or receiver in its manifest, and does not watch classic-Bluetooth connections (`android/src/main/java/com/plugin/blec/BleClient.kt` in the [published crate 0.17.0](https://crates.io/crates/tauri-plugin-blec/0.17.0); repo [MnlPhlp/tauri-plugin-blec at `8a05809`](https://github.com/MnlPhlp/tauri-plugin-blec/tree/8a058090c366cd1f6883a7d665f9d76fccd3ab70). The crate's Android module is a symlink in the repo, and its GitHub path could not be resolved here).
  - `tauri-plugin-broadcast` 0.4.1 registers receivers dynamically on the `Activity` ([BroadcastPlugin.kt L71](https://github.com/dduutt/tauri-plugin-broadcast/blob/8c5b229027d55bf7d5971bea52d4b9025954fd63/android/src/main/java/BroadcastPlugin.kt#L71)), so it hears broadcasts only while the Activity lives.
  - Watching Bluetooth while closed needs our own manifest receiver in Kotlin.

### Wi-Fi

- **No broadcast starts the app.** "Apps targeting Android 7.0 (API level 24) do not receive CONNECTIVITY_ACTION broadcasts if they register to receive them in their manifest, and processes that depend on this broadcast will not start" ([Background optimizations](https://developer.android.com/topic/performance/background-optimization)). No Wi-Fi or connectivity broadcast is on the implicit-broadcast exception list ([exceptions](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions)).
- **`registerNetworkCallback(NetworkRequest, PendingIntent)`** (API 23) sends a broadcast "when a network is available which satisfies the given NetworkRequest … the request may outlive the calling application". The `PendingIntent` corresponds to `onAvailable` only. It needs `ACCESS_NETWORK_STATE`, and there is a limit of 100 outstanding requests per app ([ConnectivityManager reference](https://developer.android.com/reference/android/net/ConnectivityManager)). Disconnects ("lost") arrive only through the in-process `NetworkCallback` variant, which lasts "until either the application exits or unregisterNetworkCallback" (same reference).
- **Reading the SSID:**
  - Needs `ACCESS_FINE_LOCATION` or `ACCESS_COARSE_LOCATION`, plus `ACCESS_WIFI_STATE`, and location services turned on (Android 9: [Android 9 changes, "Restricted access to Wi-Fi location and connection information"](https://developer.android.com/about/versions/pie/android-9.0-changes-all)).
  - From target 29, `getConnectionInfo()` is in the list needing `ACCESS_FINE_LOCATION` ([Android 10 privacy changes](https://developer.android.com/about/versions/10/privacy/changes)).
  - From API 31, `WifiInfo` comes through `NetworkCapabilities.getTransportInfo()`. Location-sensitive fields are redacted unless the callback uses `FLAG_INCLUDE_LOCATION_INFO`, in which case "the system will check location permission and the location toggle state" ([WifiManager `getConnectionInfo`](https://developer.android.com/reference/android/net/wifi/WifiManager); [NetworkCallback reference](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback)).
  - Whether reading the SSID from a background wake also needs `ACCESS_BACKGROUND_LOCATION` was not found in the docs (see Gaps).
- **Other Wi-Fi permissions.** `NEARBY_WIFI_DEVICES` (Android 13+) covers managing Wi-Fi connections and nearby devices. `getScanResults()` and `startScan()` still need `ACCESS_FINE_LOCATION` ([Wi-Fi permissions](https://developer.android.com/develop/connectivity/wifi/wifi-permissions)). Doze "Doesn't perform Wi-Fi scans" ([Doze](https://developer.android.com/training/monitoring-device-state/doze-standby)).
- **Existing plugins.** No Tauri plugin watches Wi-Fi on Android. The crates.io search found only desktop Wi-Fi and network-manager plugins.

## 6. Periodic background work, such as checking the weather every 30 minutes

- **WorkManager** "lets tasks persist across app restarts and device reboots" ([WorkManager overview](https://developer.android.com/develop/background-work/background-tasks/persistent)). "The minimum repeat interval that can be defined is 15 minutes (same as the JobScheduler API)". A flex interval narrows when in each period the work may run ([Define work requests](https://developer.android.com/develop/background-work/background-tasks/persistent/getting-started/define-work)).
- **WorkManager needs no GMS.** Current WorkManager always creates a `SystemJobScheduler`, which is built on the framework `JobScheduler` ([androidx `Schedulers.java`, `createBestAvailableBackgroundScheduler`](https://github.com/androidx/androidx/blob/androidx-main/work/work-runtime/src/main/java/androidx/work/impl/Schedulers.java)). WorkManager is the documented replacement for the GMS-based `GcmNetworkManager` ([overview](https://developer.android.com/develop/background-work/background-tasks/persistent)).
- **Doze** "Doesn't let JobScheduler run. WorkManager uses JobScheduler internally, so WorkManager tasks don't run", and it suspends network access outside maintenance windows. Maintenance windows become less frequent the longer the device is idle ([Doze](https://developer.android.com/training/monitoring-device-state/doze-standby)).
- **Standby-bucket job quotas** ([Power management resource limits](https://developer.android.com/topic/performance/power/power-details); [App Standby Buckets](https://developer.android.com/topic/performance/appstandby)):
  - Regular jobs get up to 20 minutes per 60 in "active", 10 minutes per 4 h in "working set", 10 minutes per 12 h in "frequent", and 10 minutes per 24 h in "rare". "Restricted" gets once per day.
  - Network access is "Disabled" in rare and restricted.
  - Manufacturers can set their own bucket criteria.
  - A user tapping the app's Android notification puts the app in "active"; swiping it away does not.
  - "Apps that are on the Doze exemption list are exempted from the App Standby Bucket-based restrictions."
- **Android 16 job quotas.** Android 16 applies runtime quotas to jobs started while the app is visible and to jobs running alongside a foreground service ([Android 16 behavior changes](https://developer.android.com/about/versions/16/behavior-changes-all)).
- **Android 15 network rule.** "apps that start a network request outside of a valid process lifecycle receive an exception". The advice is WorkManager or a foreground service ([Android 15 behavior changes, all apps](https://developer.android.com/about/versions/15/behavior-changes-all)).
- **Alternatives:**
  - An inexact `setAndAllowWhileIdle` AlarmManager alarm can wake the app in Doze, under the while-idle limits in §2, and start a job. The docs suggest "To perform work while the device is in Doze, create an inexact alarm using setAndAllowWhileIdle(), and start a job from the alarm" ([Schedule alarms](https://developer.android.com/develop/background-work/services/alarms/schedule)).
  - A `dataSync` foreground service can run only 6 hours per 24 on Android 15+ (target 35+). From Android 15 it cannot be started from `BOOT_COMPLETED` ([Android 15 behavior changes](https://developer.android.com/about/versions/15/behavior-changes-15)).
  - FCM high-priority push is the documented way to wake an app in Doze ([Doze](https://developer.android.com/training/monitoring-device-state/doze-standby)). It needs GMS and a server, and is not an offline mechanism.
- **Local network (Android 17).** Apps targeting Android 17 need the new `ACCESS_LOCAL_NETWORK` runtime permission (Nearby devices group) to reach LAN hosts ([Android 17 behavior changes](https://developer.android.com/about/versions/17/behavior-changes-17)). This is relevant if a self-hosted sync server is reached on the LAN.
- **Existing plugins.** No Tauri plugin wraps WorkManager; the crates.io search "tauri workmanager" returned none. `tauri-plugin-background-service` offers a long-running foreground service instead. Its README lists "Android 15 `dataSync` type has a 6-hour cumulative timeout. OEM battery optimization may kill services" ([README L26](https://github.com/dardourimohamed/tauri-background-service/blob/197f08ab1efb35df7524c1371383471c9118e481/tauri-plugin-background-service/README.md)).

## 7. Android versions and permissions by capability

Tauri's minimum is API 24; Play currently requires targeting API 36 (§2).

| Need | Min API of mechanism | Permissions / user steps | Notes |
|---|---|---|---|
| Post an Android notification | 24 (channels from 26) | `POST_NOTIFICATIONS` runtime (33+) | [Time-sensitive notifications](https://developer.android.com/develop/ui/views/notifications/time-sensitive) |
| Exact AlarmManager alarm | 23 (`setExactAndAllowWhileIdle`), 21 (`setAlarmClock`) | 31+: `SCHEDULE_EXACT_ALARM` (special access, denied by default on 14+ for target 33+) or 33+: `USE_EXACT_ALARM` (Play-restricted) | Revocation stops the app and cancels exact alarms |
| Re-arm after reboot | 1 | `RECEIVE_BOOT_COMPLETED`; `directBootAware` + device-encrypted storage to act before unlock | App must have been launched once |
| Full-screen intent | 29 behavior | `USE_FULL_SCREEN_INTENT`; 34+: limited to calling/alarm apps, user-toggleable | Falls back to a heads-up notification |
| DND channel bypass | 26 | Notification-policy access granted in Settings | Alarms already pass default priority DND in AOSP |
| Alarm audio from background | Android 17 rules | FGS; target 37: WIU FGS, or exact-alarm permission + `USAGE_ALARM` | FGS type in Play Console (34+) |
| Geofence (GMS) | GMS | `ACCESS_FINE_LOCATION` + `ACCESS_BACKGROUND_LOCATION` (29+; Settings-only on 30+) | 100 per app; Play background-location policy |
| Proximity alert (framework) | 1 | `ACCESS_FINE_LOCATION` (17+) | AOSP 15 polls at ≥30 min by default |
| Bluetooth ACL / A2DP / headset broadcasts | 5 | `BLUETOOTH_CONNECT` runtime (31+) | Exempt from manifest-receiver limits |
| CDM presence | 31 (new API 36) | CDM association (system dialog) + `REQUEST_OBSERVE_COMPANION_DEVICE_PRESENCE` | Not for Wi-Fi |
| Wi-Fi available callback | 23 | `ACCESS_NETWORK_STATE`; SSID: fine location + location on | `PendingIntent` form reports "available" only |
| Periodic work | WorkManager on JobScheduler | none | ≥15 min; Doze and bucket quotas |
| Local network sync | Android 17 target | `ACCESS_LOCAL_NETWORK` | Only if the server is on the LAN |

## 8. What degrades without Google Play Services

| Feature | With GMS | Without GMS |
|---|---|---|
| Time triggers (AlarmManager), Android notifications, full-screen intents, DND | Same | Same (framework APIs) |
| Periodic work | WorkManager | Same (WorkManager uses framework JobScheduler) |
| Bluetooth connect/disconnect, CDM, BLE `PendingIntent` scans | Same | Same (framework APIs) |
| Wi-Fi callbacks | Same | Same (framework `ConnectivityManager`) |
| Arrive/leave a place | `GeofencingClient`: low power, "every couple of minutes" in background, 100 fences | `addProximityAlert` (≥30 min polling by default in AOSP 15, in-memory) or own sampling (a few times per hour in background unless a location FGS runs) |
| Current position | Official geolocation plugin works (foreground only) | Official plugin returns "Google Play Services unavailable."; framework `LocationManager` must be called from our own Kotlin |
| Waking the app with a server push in Doze | FCM high-priority messages | Not available through FCM; the UnifiedPush alternative was not researched here |

Sources: see §4–§6. The "without GMS" location behavior on real de-Googled devices (GrapheneOS, LineageOS, microG) was not tested or researched.

## 9. Plugins versus Kotlin we would write

| Piece | Existing plugin | Gap that needs our Kotlin |
|---|---|---|
| Post an Android notification now | Official notification plugin | none (while the Activity runs; or through our own Kotlin when headless) |
| Scheduled time trigger with evaluation at fire time | none | Receiver for our AlarmManager `PendingIntent`s; boot / time-change / permission-change receivers; JNI into Rust or Kotlin evaluation |
| Scheduled fixed Android notification | Official notification plugin (inexact unless the app declares an exact-alarm permission; RTC non-wakeup in most paths) | Wakeup / `setAlarmClock` behavior |
| Alarm-style alert | none | Full-screen Activity, `CATEGORY_ALARM` channel, alarm audio, likely a foreground service |
| Action buttons that complete or snooze without opening the UI | none (official plugin opens the Activity) | `BroadcastReceiver` for actions |
| Geofence or proximity triggers | none | GMS `GeofencingClient` wrapper with framework fallback; boot re-registration |
| Bluetooth triggers | none for background (blec is foreground BLE) | Manifest receiver for ACL / A2DP / headset broadcasts, or CDM service |
| Wi-Fi triggers | none | `registerNetworkCallback(…, PendingIntent)` receiver |
| Periodic source polling | none (background-service offers an FGS, not WorkManager) | WorkManager `Worker` |
| Running Rust headless | pattern shown by `tauri-plugin-background-service` `HeadlessBridge`; documented JNI approach in Tauri docs | JNI exports in our Rust library |
| Battery-optimization prompt | `tauri-plugin-android-battery-optimization` 0.1.4 (declares `REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`) ([manifest](https://github.com/NeoHuncho/tauri-plugin-android-battery-optimization/blob/9fdc0ea9e0379ce1477380b59a93397392abed17/android/src/main/AndroidManifest.xml)) | Play restricts direct requests (§2) |

The official plugins-workspace has no background, alarm, Bluetooth, Wi-Fi or geofence plugin ([plugins-workspace README](https://github.com/tauri-apps/plugins-workspace/blob/v2/README.md)). Tauri's plugin template lets a plugin ship its own `AndroidManifest.xml` entries (receivers, services, permissions), which Android merges into the app. The official notification plugin does this for its receivers ([manifest](https://github.com/tauri-apps/plugins-workspace/blob/a2364a5f216324439feedeb25b2db74e7b1eba90/plugins/notification/android/src/main/AndroidManifest.xml)).

## Gaps and open questions

- **Play policy pages were unreachable.** Not read: the exact `USE_EXACT_ALARM` qualifying list, the full-screen-intent and foreground-service declarations, the background-location policy, and whether a reminder app with alarm-style alerts qualifies as an "alarm clock" or "calendar" app. The only indication is a search excerpt (§2).
- **Tauri docs site and GitHub issues were unreachable.** Docs were read from Markdown source. No Tauri issue or discussion about Android background execution was read; a search showed [tauri-apps/tauri discussion #8423 "Background Notifications"](https://github.com/tauri-apps/tauri/discussions/8423), not read. Nothing found says Tauri plans a headless or background runtime for Android.
- **Behavior on real devices is untested.** Several claims come from AOSP 15 source, not docs, and OEM builds and Android 16/17 may differ:
  - DND defaults and `isAlarm`;
  - proximity-alert polling interval and persistence;
  - alarm-clock exemption from standby quotas;
  - exact-alarm exemption for battery-optimization-exempt apps;
  - `USE_EXACT_ALARM` bucket elevation.

  OEM battery killers (for example on Xiaomi, Samsung and Huawei) were not researched.
- **Headless JNI is untested.** Loading the app's `cdylib` from a receiver while the Activity is not running, then later starting the Activity, was not tried. Unknowns: whether `System.loadLibrary` twice (headless path plus `Rust` object) and Tauri's own `onFirstActivityCreate` coexist safely, what they cost, and whether any global Rust state would be duplicated.
- **Background SSID access.** It was not established whether reading the SSID from a receiver woken by a Wi-Fi `PendingIntent` needs `ACCESS_BACKGROUND_LOCATION`.
- **Android 17 audio vs notification sounds.** It is unclear whether the background-audio hardening affects notification-channel sounds played by the system, as opposed to audio the app plays.
- **Conflicting while-idle limits.** Three first-party figures disagree: about every minute (15 min in idle) per the reference, once per 9 minutes per app per the Doze page, and 7 per hour per the power-limits table.
- **De-Googled devices.** microG's geofencing, GrapheneOS sandboxed Play Services, and whether the framework `FUSED_PROVIDER` is present on GMS-less ROMs were not researched. UnifiedPush as an FCM alternative was not researched here.
- **Desktop Linux** is out of scope for this ticket. The official notification plugin ignores schedules on desktop (§2), so time triggers on Linux would need a separate mechanism.

## Access notes

| Source | How it was read | Version / date |
|---|---|---|
| tauri (core, Android Kotlin in `mobile/`) | crates.io tarball of `tauri` 2.12.0; GitHub links pinned to commit `447fa9f` from its `.cargo_vcs_info.json` | released 2026-09-26 |
| tauri-macros | crates.io tarball 2.6.0, commit `e60834f` | 2026-09 |
| wry (WryActivity, Rust.kt) | crates.io tarball 0.57.0, commit `792d035` | 2026-09-08 |
| tao (Android ndk_glue) | crates.io tarball 0.37.1, commit `37b7e8b` | 2026-09-26 |
| tauri-plugin-notification | crates.io tarball 2.5.0 (and 3.0.0-alpha.1 for diff), plugins-workspace commit `a2364a5` | 2026-09-26 |
| tauri-plugin-geolocation | crates.io tarball 2.4.0 (and 3.0.0-alpha.1), commit `a2364a5` | 2026-09-26 |
| plugins-workspace README | `raw.githubusercontent.com/tauri-apps/plugins-workspace/v2/README.md` | read 2026-09-27 |
| Tauri docs (`plugin/notification.mdx`, `plugin/geolocation.mdx`, `develop/Plugins/develop-mobile.mdx`, `distribute/google-play.mdx`) | `raw.githubusercontent.com/tauri-apps/tauri-docs/v2/...` (v2.tauri.app blocked) | `v2` branch, read 2026-09-27; files undated |
| tauri-plugin-background-service | crates.io tarball 1.0.1, commit `197f08a` | 2026-07-30 |
| tauri-plugin-notifications (Choochmeque) | crates.io tarball 0.5.0-rc.14, commit `55be77f` | 2026-09-16 |
| tauri-plugin-blec | crates.io tarball 0.17.0, commit `8a05809` | 2026-09-22 |
| tauri-plugin-android-battery-optimization | crates.io tarball 0.1.4, commit `9fdc0ea` | 2026-01-19 |
| tauri-plugin-broadcast | crates.io tarball 0.4.1, commit `8c5b229` | 2026-04-14 |
| tauri-plugin-timer | crates.io tarball 0.1.0, commit `09f2f17` | 2026-08-14 |
| Plugin discovery | crates.io search API, queries for alarm, geofence, bluetooth, ble, wifi, background, notification, location, workmanager, android | 2026-09-27 |
| developer.android.com guides | fetched directly; "Last updated" per page: Schedule alarms 2026-09-16; Android 14 exact alarms 2026-03-03; Doze 2026-08-18; Power limits 2026-05-19; App Standby 2026-09-16; Time-sensitive notifications 2026-09-23; Android 14 changes 2026-09-16; Android 15 changes 2026-09-16; Android 16 changes 2026-09-16; Android 17 changes and bg-audio 2026-09-16; FGS types 2026-09-21; FGS background start 2026-09-16; Direct Boot 2026-09-16; Geofencing 2026-09-16; Background location limits 2024-04-29; Request background location 2026-09-16; Broadcast exceptions 2026-02-26; Background optimizations 2026-09-21; Bluetooth permissions 2026-09-16; CDM pairing 2026-09-16; Wi-Fi permissions 2026-09-16; WorkManager define work 2026-09-16; Target API requirements 2026-09-16 | read 2026-09-27 |
| developer.android.com API reference | AlarmManager, NotificationManager, NotificationManager.Policy, NotificationChannel, Notification, AudioAttributes (2026-08-03); LocationManager, ConnectivityManager, NetworkCallback, WifiInfo, WifiManager, BluetoothLeScanner (2026-08-03); BluetoothDevice (2026-08-28); CompanionDeviceManager (2026-09-16) | read 2026-09-27 |
| AOSP framework source | `raw.githubusercontent.com/aosp-mirror/platform_frameworks_base/android15-release/...` (android.googlesource.com and cs.android.com blocked): `default_zen_mode_config.xml`, `ZenModeFiltering.java`, `NotificationRecord.java`, `GeofenceManager.java`, `LocationManagerService.java`, `SystemSettingsHelper.java`, `AlarmManagerService.java`, `AlarmManager.java` | android15-release branch; Android 16/17 source not checked |
| androidx WorkManager source | `raw.githubusercontent.com/androidx/androidx/androidx-main/.../Schedulers.java` | androidx-main, read 2026-09-27 |

**Blocked hosts:** `tauri.app`, `v2.tauri.app`, `github.com` (pages and `api.github.com`, `codeload.github.com`), `docs.rs`, `cdn.jsdelivr.net` / `data.jsdelivr.com`, `sourcegraph.com`, `support.google.com`, `play.google.com`, `developers.google.com`, `firebase.google.com`, `source.android.com`, `android.googlesource.com`, `www.npmjs.com`. GitHub links above point to files that were read through `raw.githubusercontent.com` or from crate tarballs, at the commits shown.
