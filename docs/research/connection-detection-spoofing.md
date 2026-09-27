# Detecting Bluetooth, Wi-Fi and USB connections, and how each can be faked

Research for [csnook/hab-bot#7](https://github.com/csnook/hab-bot/issues/7). The question: on Android and on Linux, what can the app detect about connections to specific devices and networks? For each kind of connection, how can it be faked, how hard is that, and what can a user do to guard against it? The answers feed the warnings shown when someone sets up a Bluetooth, Wi-Fi or USB **source**.

- **Read on:** 2026-09-27. The version or last-updated date of each source is given in [Access notes](#access-notes).
- **Terminology:** this document uses the terms defined in `CONTEXT.md`. A **source** is what the user sets up (for example "the car's Bluetooth"). A **source class** is the kind of source as the app implements it (Bluetooth, Wi-Fi, USB). A source supplies **triggers** (such as "connected to the car") and **conditions** (such as "connected to home Wi-Fi"). "Faking" means making the app see a trigger occur, or a condition hold, when the real device or network is not involved. "Suppression" means stopping a genuine trigger or condition from being seen.
- **Unverified claims:** this environment's proxy blocked many upstream hosts, including `source.android.com`, `android.googlesource.com`, `networkmanager.dev`, `docs.kernel.org`, `flatpak.github.io`, `w1.fi`, `bluetooth.com`, `usb.org`, IEEE, ACM and CERT (full list in [Access notes](#access-notes)). Where the same text is published in the project's own GitHub repository or in an Ubuntu package, it was read there, and that copy is cited. **[inference]** marks a conclusion drawn from the cited text but not stated in it. **[unverified]** marks a claim no reachable primary source confirmed.

## Summary

- **Bluetooth.** Android reports a paired device connecting or disconnecting through broadcasts that only the system can send: `ACTION_ACL_CONNECTED` and `ACTION_ACL_DISCONNECTED` (the low-level link), and the per-profile `ACTION_CONNECTION_STATE_CHANGED` for A2DP and headset. The app needs the runtime permission `BLUETOOTH_CONNECT` ("Nearby devices") and no location permission. These four broadcasts are exempt from Android 8's limits on background broadcasts, so a receiver declared in the manifest gets them even when the app is not running. On Linux, BlueZ exposes `org.bluez.Device1.Connected` with `PropertiesChanged` signals on the D-Bus system bus. Any local user can read it; only root can own the `org.bluez` name.
- **Faking Bluetooth.** A device's name is self-reported, so matching on the name is trivial to fake. The address can be rewritten on many common Bluetooth controllers. A pure address clone lacks the pairing's link key, but the published BIAS attack (CVE-2020-10135, Core spec ≤ 5.2) impersonates a previously paired device without the link key, using a development board and public tools. BLUFFS (CVE-2023-24023, Core 4.2–5.4) is a related attack. Difficulty is moderate: it needs specialist knowledge, cheap hardware, the target's address, and nearness to the phone. The Android docs don't say whether `ACTION_ACL_CONNECTED` fires before authentication completes. That needs testing on a device.
- **Wi-Fi.** On Android the app reads the SSID and BSSID from `WifiInfo`, delivered through `ConnectivityManager.NetworkCallback`. Since Android 10 this requires `ACCESS_FINE_LOCATION` and the device's location setting switched on. Since Android 12 the callback must also opt in with `FLAG_INCLUDE_LOCATION_INFO`. Without these, the app gets `<unknown ssid>` and `02:00:00:00:00:00`. `NEARBY_WIFI_DEVICES` (Android 13) is not documented as covering this. No broadcast delivers Wi-Fi changes to a manifest-declared receiver. A `PendingIntent` callback can report joining a network while the app is not running, but not leaving it. On Linux, NetworkManager exposes the active access point's `Ssid` and `HwAddress` (BSSID) on the system bus, readable by any local user with no permission prompt. xdg-desktop-portal offers no SSID.
- **Faking Wi-Fi.** Any access point can broadcast any SSID. A BSSID is the radio's MAC address: it can be changed in software and anyone nearby can see it in a scan. So SSID plus BSSID stops a casual lookalike network but not a deliberate one. The real barrier on a WPA2/WPA3-Personal network is the password: a fake access point has to know it to complete the connection. Anyone the password was shared with could therefore fake the network. Open networks give no protection at all. The "SSID Confusion" flaw (CVE-2023-52424) lets an attacker in the middle make a client join a different network that shares its credentials. It affects WPA3 SAE-loop, 802.1X/EAP and some other modes.
- **USB.** A USB device identifies itself with self-reported descriptors: vendor ID, product ID, version, and manufacturer, product and optional serial-number strings. Linux sees these through sysfs and udev (`ID_VENDOR_ID`, `ID_MODEL_ID`, `ID_SERIAL`, `ID_SERIAL_SHORT`). A Flatpak app can see them through the USB portal (xdg-desktop-portal 1.19.1 and later), which also sends hot-plug events. Android sees USB devices only when the phone acts as the USB host (OTG). When the phone is plugged into a car or a computer, the phone is the device. Android then reports only that USB power is connected. The one exception is an accessory using Android's accessory protocol, which sends manufacturer, model, version and serial strings.
- **Faking USB.** The Linux kernel's own documentation says it plainly: "Anybody with access to a device gadget kit can fake descriptors and device info. Don't trust that." Any Linux board with a USB device-mode controller can set arbitrary IDs and strings through configfs. So can Facedancer boards and reflashed USB-stick controllers (BadUSB-style). Faking is easy once the attacker knows the target's descriptors and can reach the port.
- **Forgery by other apps.** In every case the connection events come from the operating system. Android marks each of these broadcasts as system-only. On Linux, D-Bus policy lets only root own the `org.bluez` and NetworkManager names, and libudev accepts events only from root, from its own user ID, or from outside its user namespace. So another app can fake these events only on a rooted or compromised system, or, on Linux, from another process running as the same user. The practical attacks are over the air or at the USB port.
- **Google Play Services.** Everything above is Android framework API (`android.bluetooth`, `android.companion`, `android.net`, `android.hardware.usb`). None of it needs Google Play Services.

## Access notes

| Source | How it was read | Version / date |
|---|---|---|
| Android API reference and guides (`developer.android.com`) | Fetched directly | Each page's "Last updated" date is given in [Sources](#sources) (2024-01-03 to 2026-09-21) |
| AOSP `frameworks/base/core/res/AndroidManifest.xml` (list of broadcasts only the system may send) | GitHub mirror [aosp-mirror/platform_frameworks_base](https://github.com/aosp-mirror/platform_frameworks_base/blob/main/core/res/AndroidManifest.xml), `main` | read 2026-09-27. The mirror may lag AOSP; `android.googlesource.com` and `cs.android.com` were blocked. |
| AOSP Bluetooth stack (`packages/modules/Bluetooth`) and Wi-Fi permission code (`WifiPermissionsUtil`) | **Blocked**: googlesource was unreachable and no GitHub mirror was found | — |
| `source.android.com` (Wi-Fi network selection, Android Open Accessory protocol, security bulletins) | **Blocked** | — |
| BlueZ D-Bus docs, D-Bus policy, `main.conf`, `tools/bdaddr.c` | [bluez/bluez](https://github.com/bluez/bluez) on GitHub, `master` | BlueZ 5.87 (`configure.ac`); `org.bluez.Device.rst` header dated October 2023 |
| NetworkManager D-Bus introspection XML and D-Bus policy | [NetworkManager/NetworkManager](https://github.com/NetworkManager/NetworkManager) on GitHub, `main` | 1.59.2-dev (`meson.build`). `networkmanager.dev` was blocked. |
| xdg-desktop-portal interface XML, `usb.c`, NEWS | [flatpak/xdg-desktop-portal](https://github.com/flatpak/xdg-desktop-portal), `main` | 23.0 in development; USB portal added in 1.19.1 (NEWS). `flatpak.github.io` was blocked. |
| Flatpak docs and `flatpak-build-finish(1)` | [flatpak/flatpak-docs](https://github.com/flatpak/flatpak-docs/blob/master/docs/sandbox-permissions.rst) and [flatpak/flatpak](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-build-finish.xml) | read 2026-09-27. `docs.flatpak.org` was blocked. |
| Linux kernel docs and headers | [torvalds/linux](https://github.com/torvalds/linux) GitHub mirror, `master` | 7.3-rc4 (`Makefile`). `docs.kernel.org` and `git.kernel.org` were blocked. |
| systemd/udev source | [systemd/systemd](https://github.com/systemd/systemd), `main` | 263~devel (`meson.version`) |
| USBGuard rule language | [USBGuard/usbguard](https://github.com/USBGuard/usbguard/blob/main/doc/man/usbguard-rules.conf.5.adoc), `main` | 1.1.4 (`VERSION`) |
| hostapd example configuration | Ubuntu 24.04 package [`hostapd_2.10-21ubuntu0.4_amd64.deb`](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb), file `/usr/share/doc/hostapd/examples/hostapd.conf` | hostapd 2.10. The upstream `w1.fi` was blocked. |
| BIAS, BLUFFS, KNOB | Authors' repositories [francozappa/bias](https://github.com/francozappa/bias), [francozappa/bluffs](https://github.com/francozappa/bluffs), [francozappa/knob](https://github.com/francozappa/knob) | read 2026-09-27. The papers (`francozappa.github.io`, ACM, USENIX, IEEE) and the Bluetooth SIG statements were blocked. |
| CVE records | [CVEProject/cvelistV5](https://github.com/CVEProject/cvelistV5) JSON | as published (dates below). `cve.org` and `nvd.nist.gov` were blocked. |
| USB emulation and reflashing tools | [greatscottgadgets/facedancer](https://github.com/greatscottgadgets/facedancer), [adamcaudill/Psychson](https://github.com/adamcaudill/Psychson), [greatscottgadgets/ubertooth](https://github.com/greatscottgadgets/ubertooth/blob/master/host/doc/ubertooth-rx.md) | read 2026-09-27 |

## Bluetooth

### Detection on Android

**Connection events (triggers).**

- [`BluetoothDevice.ACTION_ACL_CONNECTED`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#ACTION_ACL_CONNECTED) "Indicates a low level (ACL) connection has been established with a remote device". [`ACTION_ACL_DISCONNECTED`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#ACTION_ACL_DISCONNECTED) indicates the matching disconnection. Both have existed since API 5.
  - Both always carry `EXTRA_DEVICE` and `EXTRA_TRANSPORT`. `EXTRA_TRANSPORT` is either `TRANSPORT_BREDR` (Classic) or `TRANSPORT_LE` (Low Energy). So LE links fire these broadcasts too ([`EXTRA_TRANSPORT`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#EXTRA_TRANSPORT)).
  - Apps targeting Android 12 (API 31) or later need `BLUETOOTH_CONNECT` to receive them. Apps targeting API 30 or lower need `BLUETOOTH` (same page).
- Profile-level events: [`BluetoothA2dp.ACTION_CONNECTION_STATE_CHANGED`](https://developer.android.com/reference/android/bluetooth/BluetoothA2dp#ACTION_CONNECTION_STATE_CHANGED) (media audio) and [`BluetoothHeadset.ACTION_CONNECTION_STATE_CHANGED`](https://developer.android.com/reference/android/bluetooth/BluetoothHeadset#ACTION_CONNECTION_STATE_CHANGED) (calls/HFP).
  - These carry `EXTRA_STATE` and `EXTRA_PREVIOUS_STATE` (`STATE_DISCONNECTED`, `STATE_CONNECTING`, `STATE_CONNECTED` or `STATE_DISCONNECTING`), plus `EXTRA_DEVICE`, and a disconnect reason where one applies.
  - A car head unit usually offers one or both of these profiles **[inference]**.
- [`ACTION_ENCRYPTION_CHANGE`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#ACTION_ENCRYPTION_CHANGE) (API 36) reports whether a link is encrypted, with the key size and algorithm. It is not on the background-exemption list below, so an app receives it only through a receiver registered while the app is running.

**Background delivery.**

- Android 8 stopped delivering most implicit broadcasts to receivers declared in the manifest. The [implicit broadcast exceptions list](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions) exempts `BluetoothHeadset.ACTION_CONNECTION_STATE_CHANGED`, `BluetoothA2dp.ACTION_CONNECTION_STATE_CHANGED`, `ACTION_ACL_CONNECTED` and `ACTION_ACL_DISCONNECTED`. So a manifest receiver still gets these four when the app is not running.
- **Companion Device Manager (optional alternative).** From Android 16 (API 36), [`CompanionDeviceManager.startObservingDevicePresence(ObservingDevicePresenceRequest)`](https://developer.android.com/reference/android/companion/CompanionDeviceManager#startObservingDevicePresence(android.companion.ObservingDevicePresenceRequest)) binds the app's `CompanionDeviceService` and delivers [`DevicePresenceEvent`](https://developer.android.com/reference/android/companion/DevicePresenceEvent) events, `EVENT_BT_CONNECTED` and `EVENT_BT_DISCONNECTED`.
  - "For Bluetooth classic devices this is triggered when the device connects/disconnects". "WiFi devices are not supported."
  - The app needs `REQUEST_OBSERVE_COMPANION_DEVICE_PRESENCE`, and the device must first be associated with the app. Association goes through a system consent dialog ([companion device pairing guide](https://developer.android.com/develop/connectivity/bluetooth/companion-device-pairing)).
  - On API 31–35 the older, now-deprecated `startObservingDevicePresence(String)` works the same way for Classic devices, with `onDeviceAppeared`/`onDeviceDisappeared`. With it, "The app doesn't need to remain running in order to receive its callbacks" (same CompanionDeviceManager page).
  - A CDM-associated app that holds `REQUEST_COMPANION_START_FOREGROUND_SERVICES_FROM_BACKGROUND` or `REQUEST_COMPANION_RUN_IN_BACKGROUND` may start foreground services from the background ([FGS background-start exemptions](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start)).

**Current state (for conditions).**

- [`BluetoothProfile.getConnectedDevices()`](https://developer.android.com/reference/android/bluetooth/BluetoothProfile#getConnectedDevices()) returns the devices connected on a given profile. The app gets the profile object through [`BluetoothAdapter.getProfileProxy`](https://developer.android.com/reference/android/bluetooth/BluetoothAdapter#getProfileProxy(android.content.Context,%20android.bluetooth.BluetoothProfile.ServiceListener,%20int)).
- [`BluetoothDevice.isConnected(int transport)`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#isConnected(int)) is public only from API 36.1. On earlier versions, an app that needs the link-level state has to track the ACL broadcasts itself **[inference]**.

**Choosing the device at setup.**

- [`BluetoothAdapter.getBondedDevices()`](https://developer.android.com/reference/android/bluetooth/BluetoothAdapter#getBondedDevices()) lists paired devices. It returns an empty set when Bluetooth is off.
- [`getAddress()`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#getAddress()) returns "the hardware address".
- [`getAddressType()`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#getAddressType()) (API 35) distinguishes `ADDRESS_TYPE_PUBLIC` ("Hardware MAC Address") from `ADDRESS_TYPE_RANDOM` ("resolvable, non-resolvable or static").
- The CDM docs note that an LE device using a rotating (Resolvable Private) address should be bonded "so that android OS is able to resolve the address" ([CompanionDeviceManager](https://developer.android.com/reference/android/companion/CompanionDeviceManager)).
- [`getName()`](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#getName()) returns the name the remote device reported, cached locally. `getAlias()` returns a locally editable name.

**Permissions.**

- On Android 12 and later, `BLUETOOTH_CONNECT` is a runtime permission. It sits in the "Nearby devices" group together with `NEARBY_WIFI_DEVICES` and the UWB permissions.
- No location permission is needed to talk to paired devices. Location matters only if scan results are used to work out location ([Bluetooth permissions](https://developer.android.com/develop/connectivity/bluetooth/bt-permissions)).
- Apps targeting Android 11 or lower declare `BLUETOOTH` instead (same page).

### Detection on Linux

- **BlueZ** exposes each known remote device as an object implementing [`org.bluez.Device1`](https://github.com/bluez/bluez/blob/master/doc/org.bluez.Device.rst). The service is `org.bluez` and the object path is `…/hci0/dev_{BDADDR}`.
  - Properties:
    - `Address`
    - `AddressType`: "public" for BR/EDR devices; for LE devices using privacy, the identity address after pairing
    - `Name`, and `Alias`, which the user can edit
    - `Paired`, `Bonded`, `Trusted`, `Blocked`
    - `UUIDs`
    - `Connected`: "A PropertiesChanged signal indicate changes to this status"
    - `ServicesResolved`
  - The `Disconnected(reason, message)` signal gives the reason for a disconnection.
- **Who can read it.** BlueZ's [D-Bus policy](https://github.com/bluez/bluez/blob/master/src/bluetooth.conf) lets only `root` own `org.bluez`, while the default context may send to it. So any local process can read device state, and only root can impersonate the service **[inference from the policy]**.
- **Sandboxing.** No xdg-desktop-portal interface exists for Bluetooth (the interfaces shipped are listed in [`data/meson.build`](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/meson.build)).
  - A Flatpak app's system-bus access is filtered by default ([Flatpak sandbox permissions](https://github.com/flatpak/flatpak-docs/blob/master/docs/sandbox-permissions.rst)). To reach `org.bluez` it needs `--system-talk-name=org.bluez` ([`flatpak-build-finish(1)`](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-build-finish.xml)).
  - `--allow=bluetooth` grants `AF_BLUETOOTH` sockets, which only observing through D-Bus should not need **[inference]**.
  - An app that is not sandboxed (deb, rpm, AppImage) has the default D-Bus access described above.
- **KDE Plasma.** Its Bluetooth UI is assumed to sit on BlueZ; this could not be checked because `invent.kde.org` was blocked **[unverified]**.

### Faking it, and how hard that is

| Attack | What it takes | Difficulty | Source |
|---|---|---|---|
| Another app forging the event | Android: the ACL broadcasts appear under "Special broadcasts that only the system can send". Linux: only root may own `org.bluez`. So this needs a rooted or compromised OS. | Needs a rooted or compromised OS | [AOSP AndroidManifest.xml](https://github.com/aosp-mirror/platform_frameworks_base/blob/main/core/res/AndroidManifest.xml), [bluetooth.conf](https://github.com/bluez/bluez/blob/master/src/bluetooth.conf) |
| Name spoofing | Any device can report any name. The name is a self-reported value that the phone caches. The BIAS attack instructions set `btname` to the victim's name. | Trivial. Matters only if the app matches by name, or shows several devices with the same name at setup. | [BluetoothDevice.getName](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#getName()), [BIAS instructions](https://github.com/francozappa/bias/blob/master/bias/README.md) |
| Address cloning | Many controllers accept vendor commands that rewrite the address. BlueZ's `bdaddr` tool ("Utility for changing the Bluetooth device address") supports Ericsson, CSR, TI, Broadcom, Zeevo, ST and Marvell chips. The attacker must know the target's address. Ubertooth recovers the lower 32 bits (LAP and UAP) over the air, "able to discover undiscoverable devices". The top 16 bits (NAP) are "not significant" to that tool and are not recovered. How an attacker gets them is not covered here **[unverified]**. | Low to moderate for the clone itself. Learning the full address needs nearness, and possibly a moment when the car or phone is discoverable. | [bdaddr.c](https://github.com/bluez/bluez/blob/master/tools/bdaddr.c), [ubertooth-rx](https://github.com/greatscottgadgets/ubertooth/blob/master/host/doc/ubertooth-rx.md) |
| Impersonating a paired device without its link key (BIAS) | CVE-2020-10135: legacy and Secure Connections authentication in Core spec ≤ 5.2 "may allow an unauthenticated user to complete authentication without pairing credentials via adjacent access… impersonate a Bluetooth BR/EDR master or slave… without knowing the link key". The published setup is a Linux laptop, a CYW920819 development board and InternalBlue. | Moderate: public tools, cheap hardware, specialist skill, and nearness. How widely phones and head units are patched was not verified. | [CVE-2020-10135](https://github.com/CVEProject/cvelistV5/blob/main/cves/2020/10xxx/CVE-2020-10135.json), [BIAS repo](https://github.com/francozappa/bias) |
| Session-key attacks (BLUFFS, KNOB) | BLUFFS (CVE-2023-24023, Core 4.2–5.4) enables "device impersonation and machine-in-the-middle across sessions by only compromising one session key". The authors say it is effective "regardless of the victim's hardware and software details". KNOB (CVE-2019-9506) forces a short encryption key. | Moderate to high | [BLUFFS repo](https://github.com/francozappa/bluffs), [CVE-2023-24023](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/24xxx/CVE-2023-24023.json), [CVE-2019-9506](https://github.com/CVEProject/cvelistV5/blob/main/cves/2019/9xxx/CVE-2019-9506.json) |
| A link-level event without authentication | `ACTION_ACL_CONNECTED` is documented as "a low level (ACL) connection". The docs do not say it waits for authentication or encryption. It may fire for a device that clones only the address and then fails authentication. The AOSP Bluetooth source that would settle this was unreachable. | Unknown; needs testing on a device | [BluetoothDevice](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#ACTION_ACL_CONNECTED) **[unverified]** |
| Suppression | A trigger cannot occur while Bluetooth is off; `getBondedDevices()` is empty unless the adapter is `STATE_ON`. Radio jamming and relay attacks were not researched. | — | [BluetoothAdapter](https://developer.android.com/reference/android/bluetooth/BluetoothAdapter#getBondedDevices()) |

### What a user can do

- **Identify the device by its paired address, not its name.** Pick it from the paired list (`getBondedDevices()` or BlueZ's `Paired`/`Bonded` devices) rather than matching a name. A lookalike name then does nothing. The address can still be cloned by someone with the tools above ([sources in the table](#faking-it-and-how-hard-that-is)).
- **Prefer an authenticated signal over a bare link.** A profile connection (A2DP/HFP `STATE_CONNECTED`), or an encrypted link reported by `ACTION_ENCRYPTION_CHANGE` with its key size, goes further through the protocol than a bare ACL link. Whether this defeats a clone that has only the address has not been verified ([A2DP](https://developer.android.com/reference/android/bluetooth/BluetoothA2dp#ACTION_CONNECTION_STATE_CHANGED), [ACTION_ENCRYPTION_CHANGE](https://developer.android.com/reference/android/bluetooth/BluetoothDevice#ACTION_ENCRYPTION_CHANGE)) **[unverified]**.
- **Keep the phone, the car head unit and the computer updated.** BIAS and BLUFFS are flaws in the specification, and fixes depend on each vendor ([CVE-2020-10135](https://github.com/CVEProject/cvelistV5/blob/main/cves/2020/10xxx/CVE-2020-10135.json), [CVE-2023-24023](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/24xxx/CVE-2023-24023.json)).
- **On Linux, tighten BlueZ.** In [`main.conf`](https://github.com/bluez/bluez/blob/master/src/main.conf), `SecureConnections = only` ("we allow only Secure Connections") refuses older pairing methods, and `JustWorksRepairing` defaults to `never`. Setting a device's `Blocked` property rejects every incoming connection from it ([Device1](https://github.com/bluez/bluez/blob/master/doc/org.bluez.Device.rst)).
- **Remove old pairings.** Removing pairings with devices no longer used shrinks the set of addresses that can raise a trigger. This follows from triggers being tied to paired devices **[inference]**.
- **Add an independent condition.** For anything where a false trigger matters, combine the Bluetooth trigger with a condition from a different source class (time, place). An attacker then has to fake both. This is a trade-off for the design to weigh, not something tested here.

## Wi-Fi

### Detection on Android

**API surface.**

- Current-network information comes from [`NetworkCapabilities.getTransportInfo()`](https://developer.android.com/reference/android/net/NetworkCapabilities#getTransportInfo()), which returns a `WifiInfo`. It is delivered through `ConnectivityManager.NetworkCallback.onCapabilitiesChanged` for a `NetworkRequest` with `TRANSPORT_WIFI`.
- `WifiManager.getConnectionInfo()` was deprecated in API 31 in favour of this ([WifiManager](https://developer.android.com/reference/android/net/wifi/WifiManager#getConnectionInfo())).

**SSID and BSSID, and what hides them.**

- [`WifiInfo.getSSID()`](https://developer.android.com/reference/android/net/wifi/WifiInfo#getSSID()) may return `WifiManager.UNKNOWN_SSID` "if the caller has insufficient permissions to access the SSID".
- [`getBSSID()`](https://developer.android.com/reference/android/net/wifi/WifiInfo#getBSSID()) returns `"02:00:00:00:00:00"` in that case. With Wi-Fi 7 multi-link operation, the BSSID is "the BSSID of the link used for association". `getApMldMacAddress()` gives the multi-link device address.
- Since Android 12, location-sensitive fields are stripped from callbacks "even if the app holds the necessary permissions" unless the callback is built with [`FLAG_INCLUDE_LOCATION_INFO`](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback#FLAG_INCLUDE_LOCATION_INFO). With the flag, "the system will check location permission and the location toggle state, and take note of location usage by the app".
- [`getCurrentSecurityType()`](https://developer.android.com/reference/android/net/wifi/WifiInfo#getCurrentSecurityType()) (API 31) reports whether the current network is `OPEN`, `PSK`, `SAE`, `EAP`, `OWE` and so on.

**Permission history.**

| Android version | Requirement to read the connected SSID/BSSID | Source |
|---|---|---|
| 9 (API 28) | `ACCESS_FINE_LOCATION` or `ACCESS_COARSE_LOCATION`, plus `ACCESS_WIFI_STATE`, plus location services on. `NETWORK_STATE_CHANGED_ACTION` no longer carries SSID/BSSID. | [Android 9 changes](https://developer.android.com/about/versions/pie/android-9.0-changes-all) |
| 10 (target API 29+) | `ACCESS_FINE_LOCATION` for `getConnectionInfo()` and other Wi-Fi methods | [Android 10 privacy changes](https://developer.android.com/about/versions/10/privacy/changes) |
| 12 (API 31) | As above, plus `FLAG_INCLUDE_LOCATION_INFO` on the callback | [NetworkCallback](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback#FLAG_INCLUDE_LOCATION_INFO) |
| 13+ (target API 33+) | `NEARBY_WIFI_DEVICES` covers hotspot, Wi-Fi Aware, P2P and RTT APIs. Connection info is not in its list. The page adds that "Several Wi-Fi APIs require the ACCESS_FINE_LOCATION permission, even when your app targets Android 13 or higher". So reading the connected SSID still needs fine location **[inference: absence from the list]**. | [Wi-Fi permissions](https://developer.android.com/develop/connectivity/wifi/wifi-permissions) |

**Background.**

- `CONNECTIVITY_ACTION` was deprecated in API 28 and is not delivered to manifest receivers for apps targeting API 24+ ([ConnectivityManager](https://developer.android.com/reference/android/net/ConnectivityManager#CONNECTIVITY_ACTION)). `WifiManager.NETWORK_STATE_CHANGED_ACTION` "is not delivered to manifest receivers in applications that target API version 26 or later" ([WifiManager](https://developer.android.com/reference/android/net/wifi/WifiManager#NETWORK_STATE_CHANGED_ACTION)).
- Neither broadcast is on the [exceptions list](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions).
- [`registerNetworkCallback(NetworkRequest, PendingIntent)`](https://developer.android.com/reference/android/net/ConnectivityManager#registerNetworkCallback(android.net.NetworkRequest,%20android.app.PendingIntent)) "may outlive the calling application". But its intent corresponds only to `onAvailable`, which is joining a network. It is not sent on leaving. Detecting departure needs a callback in a running process, for example inside a foreground service **[inference]**.
- **Background location.** The system counts location as used in the background whenever it is used without a visible activity or a foreground service ([location permissions](https://developer.android.com/develop/sensors-and-location/location/permissions)).
  - Background location needs `ACCESS_BACKGROUND_LOCATION`. On Android 11 and later the user grants it on a settings page ("Allow all the time"), not in the dialog ([background location](https://developer.android.com/develop/sensors-and-location/location/permissions/background)).
  - A `location`-type foreground service cannot be started from the background without that permission ([FGS types](https://developer.android.com/develop/background-work/services/fgs/service-types)).
  - Whether reading the SSID through `FLAG_INCLUDE_LOCATION_INFO` in the background counts as background location is not stated. It is likely, given that "the system will… take note of location usage" **[unverified; test on a device]**.
- The `connectedDevice` foreground-service type is available to an app that declares `CHANGE_NETWORK_STATE` or `CHANGE_WIFI_STATE` ([FGS types](https://developer.android.com/develop/background-work/services/fgs/service-types)).
- Companion-device presence does not support Wi-Fi ([CompanionDeviceManager](https://developer.android.com/reference/android/companion/CompanionDeviceManager)).

### Detection on Linux

- **NetworkManager API.**
  - `org.freedesktop.NetworkManager.Device.Wireless.ActiveAccessPoint` points to the access point in use ([Device.Wireless XML](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.Device.Wireless.xml)).
  - That [`AccessPoint`](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.AccessPoint.xml) object has `Ssid` (bytes), `HwAddress` ("The hardware address (BSSID) of the access point"), `Flags`/`WpaFlags`/`RsnFlags` (security capabilities), `Frequency`, `Strength` and `LastSeen`.
  - The top-level object has `ActiveConnections` and `PrimaryConnection` ([NetworkManager XML](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.xml)). Each active connection has `Id`, `Uuid` (the saved profile) and `State` ([Connection.Active XML](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.Connection.Active.xml)).
  - Changes arrive as `PropertiesChanged` and `StateChanged` signals.
- **Who can read it.** NetworkManager's [D-Bus policy](https://github.com/NetworkManager/NetworkManager/blob/main/src/core/org.freedesktop.NetworkManager.conf) lets only root own the name. The default context may call `Properties`, `ObjectManager` and the read-only `AccessPoint` interface. There is no equivalent of Android's location gate on Linux: any local process can read the SSID and BSSID **[inference from the policy]**.
- **Sandboxing.**
  - The [`NetworkMonitor` portal](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.NetworkMonitor.xml) reports only whether the network is available, whether it is metered, the connectivity level and host reachability. It gives no SSID or BSSID.
  - A Flatpak app would need `--system-talk-name=org.freedesktop.NetworkManager` ([`flatpak-build-finish(1)`](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-build-finish.xml)).
- **Not covered.** Linux systems managed by iwd, systemd-networkd or ConnMan instead of NetworkManager were not researched. That KDE Plasma's network applet uses NetworkManager could not be checked (`invent.kde.org` was blocked) **[unverified]**.

### Faking it, and how hard that is

| Attack | What it takes | Difficulty | Source |
|---|---|---|---|
| Another app forging the event | `android.net.wifi.STATE_CHANGE` is a system-only broadcast. `NetworkCallback` data comes from the system. On Linux only root may own the NetworkManager name. | Needs a rooted or compromised OS | [AOSP AndroidManifest.xml](https://github.com/aosp-mirror/platform_frameworks_base/blob/main/core/res/AndroidManifest.xml), [NM policy](https://github.com/NetworkManager/NetworkManager/blob/main/src/core/org.freedesktop.NetworkManager.conf) |
| Same SSID, open network | An access point advertises whatever SSID it is configured with (hostapd `ssid=`). An open network has no secret. | Trivial: a phone hotspot or a cheap router | [hostapd.conf](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb) |
| Same BSSID | A BSSID is a MAC address. hostapd's docs say the radio's MAC "must be changed before starting hostapd (`ifconfig wlan0 hw ether <MAC addr>`)" when it doesn't fit, and that extra BSSIDs can be set explicitly (`bssid=`). BSSIDs are not secret: NetworkManager creates an `AccessPoint` object with a `HwAddress` for every access point found in a scan. | Low for someone with a Linux-capable Wi-Fi adapter who has been near the home network once | [hostapd.conf](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb) (Multiple BSSID section), [AccessPoint XML](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.AccessPoint.xml) |
| Same SSID on WPA2/WPA3-Personal | The attacker must know the password. For WPA2-PSK the key "uses SSID so the PSK changes when ASCII passphrase is used and the SSID is changed". A lookalike access point without the passphrase cannot complete the handshake. That last point is standard 802.11 behaviour; the IEEE text was not reachable. So anyone the password was ever shared with (guests, neighbours, a travel router set up with the same SSID and password) can make a network that passes an SSID check. | Low for anyone who knows the password; high otherwise | [hostapd.conf](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb) (`wpa_passphrase` comment); IEEE 802.11 **[unverified]** |
| Auto-join of a lookalike | [`WifiConfiguration.BSSID`](https://developer.android.com/reference/android/net/wifi/WifiConfiguration#BSSID): "When set, this network configuration entry should only be used when associating with the AP having the specified BSSID". By default, then, a saved network is not tied to one BSSID, and the phone may join any access point with a matching SSID and security type **[inference]**. `source.android.com`'s network-selection page was blocked. | — | [WifiConfiguration](https://developer.android.com/reference/android/net/wifi/WifiConfiguration#BSSID) |
| SSID Confusion (CVE-2023-52424) | "Trick a victim into connecting to an unintended or untrusted network with Home WEP, Home WPA3 SAE-loop, Enterprise 802.1X/EAP, Mesh AMPE, or FILS… because the SSID is not always used to derive the pairwise master key or session keys". This matters when the home network shares credentials with another network. | Moderate: needs a machine-in-the-middle position. Patch status on Android, Linux and routers was not verified. | [CVE-2023-52424](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/52xxx/CVE-2023-52424.json) |
| Suppression | Knocking a phone off its network makes a "connected to home Wi-Fi" condition false. hostapd documents Protected Management Frames (`ieee80211w`, "PMF required" for WPA3-Personal-only mode). Whether PMF stops forged disconnection frames is standard 802.11w behaviour and was not verified here. | — | [hostapd.conf](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb) **[unverified]** |

### What a user can do

- **Use WPA2/WPA3-Personal with a password that isn't widely shared.** Anyone who knows the password can build a network that passes the check (see the table above). Open networks offer no assurance. On Android, `WifiInfo.getCurrentSecurityType()` reveals when a network is open ([WifiInfo](https://developer.android.com/reference/android/net/wifi/WifiInfo#getCurrentSecurityType())). On Linux, the `WpaFlags`/`RsnFlags` properties do ([AccessPoint XML](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.AccessPoint.xml)).
- **Put guests on a separate network, with a separate password.** Changing the main password after sharing it has the same effect. Don't reuse one set of credentials across SSIDs, because of SSID Confusion ([CVE-2023-52424](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/52xxx/CVE-2023-52424.json)).
- **Record the BSSID as well as the SSID.** This defeats a casual lookalike, such as a phone hotspot renamed to the home SSID. It does not stop a deliberate attacker, since the BSSID can be both learned and set ([hostapd.conf](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb)).
  - The trade-off is false negatives. A network with several access points, several bands or Wi-Fi 7 multi-link operation shows several BSSIDs, and Android reports the BSSID "of the link used for association" ([WifiInfo.getBSSID](https://developer.android.com/reference/android/net/wifi/WifiInfo#getBSSID())). Every BSSID would need recording.
- **Prefer WPA3-Personal with PMF where the router supports it.** hostapd documents a WPA3-only mode (`ieee80211w=2`, `wpa_key_mgmt=SAE`), transition-disable flags, and SAE-PK ([hostapd.conf](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb)). How far SAE-PK protects against someone who knows the password, and whether Android and Linux clients support it, was not verified **[unverified]**.

## USB

### Detection on Android

**Phone as USB host (OTG): a USB device is plugged into the phone.**

- **Attach.** [`UsbManager.ACTION_USB_DEVICE_ATTACHED`](https://developer.android.com/reference/android/hardware/usb/UsbManager#ACTION_USB_DEVICE_ATTACHED) is an "Activity intent sent when user attaches a USB device… when in host mode".
  - An app declares an intent filter on an activity, with a `device_filter.xml` matching `vendor-id`, `product-id`, `class`, `subclass` and `protocol`. The app must declare the `android.hardware.usb.host` feature.
  - When a matching device is connected, "the system presents them with a dialog that asks if they want to start your application". If the user accepts, the app has permission for that device until it is disconnected ([USB host guide](https://developer.android.com/develop/connectivity/usb/host), last updated 2024-05-23).
- **Detach.** [`ACTION_USB_DEVICE_DETACHED`](https://developer.android.com/reference/android/hardware/usb/UsbManager#ACTION_USB_DEVICE_DETACHED) is a broadcast.
- **Background and forgery.** `ACTION_USB_DEVICE_ATTACHED` and `ACTION_USB_DEVICE_DETACHED` are both on the [exceptions list](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions). Both are system-only ([AOSP AndroidManifest.xml](https://github.com/aosp-mirror/platform_frameworks_base/blob/main/core/res/AndroidManifest.xml)).
- **Identity.** [`UsbDevice`](https://developer.android.com/reference/android/hardware/usb/UsbDevice) provides `getVendorId()`, `getProductId()`, `getManufacturerName()`, `getProductName()` and `getVersion()`. [`getSerialNumber()`](https://developer.android.com/reference/android/hardware/usb/UsbDevice#getSerialNumber()) throws `SecurityException` when the app targets Android 10+ "and the app does not have permission to read from the device".

**Phone plugged into a host (car head unit, computer, charger).**

- The phone is then the USB device, and the host is not reported as a `UsbDevice`: `ACTION_USB_DEVICE_ATTACHED` is sent only "in host mode" ([UsbManager](https://developer.android.com/reference/android/hardware/usb/UsbManager#ACTION_USB_DEVICE_ATTACHED)).
- What is available:
  - [`Intent.ACTION_POWER_CONNECTED`](https://developer.android.com/reference/android/content/Intent#ACTION_POWER_CONNECTED), which "can only be sent by the system". It is not on the exceptions list, so apps targeting API 26+ receive it only through a receiver registered while running ([exceptions list](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions)).
  - [`BatteryManager.EXTRA_PLUGGED`](https://developer.android.com/reference/android/os/BatteryManager#EXTRA_PLUGGED), whose value `BATTERY_PLUGGED_USB` means "Power source is a USB port".
- Neither identifies which host. `android.hardware.usb.action.USB_STATE` is a system-only broadcast but is not in the public `UsbManager` API.
- **Accessory mode.** A host that implements the Android accessory protocol identifies itself with `manufacturer`, `model` and `version` (the fields filtered on) and an optional serial ([USB accessory guide](https://developer.android.com/develop/connectivity/usb/accessory), 2024-01-03). [`UsbAccessory.getSerial()`](https://developer.android.com/reference/android/hardware/usb/UsbAccessory#getSerial()) also needs per-accessory permission on Android 10+.
- Whether car head units expose anything to third-party apps this way (for example alongside Android Auto) was not researched. The protocol spec on `source.android.com` was blocked **[unverified]**.

### Detection on Linux

- **Descriptors.**
  - The kernel caches each device's descriptors. The [device descriptor](https://github.com/torvalds/linux/blob/master/include/uapi/linux/usb/ch9.h) holds `idVendor`, `idProduct`, `bcdDevice`, the class, subclass and protocol, and string indices `iManufacturer`, `iProduct` and `iSerialNumber`.
  - They are readable from sysfs at `/sys/bus/usb/devices/.../descriptors`, and devices are named `<busnum>-<port[.port]>`, which encodes the physical port ([sysfs-bus-usb ABI](https://github.com/torvalds/linux/blob/master/Documentation/ABI/stable/sysfs-bus-usb)).
- **udev.** udev's [`usb_id` builtin](https://github.com/systemd/systemd/blob/main/src/udev/udev-builtin-usb_id.c) sets:
  - `ID_VENDOR_ID`, `ID_MODEL_ID` and `ID_REVISION`
  - `ID_SERIAL`, built as vendor `_` model, plus `_` serial "If the device supplies a serial number"
  - `ID_SERIAL_SHORT`, only when the serial consists of printable characters; any other serial is dropped
- **Hot-plug events** reach libudev/sd-device monitors over netlink. A monitor trusts messages from uid 0, from its own uid ("Currently, such situation happens only for unicast messages"), or from outside its user namespace, and refuses others ([`device-monitor.c`](https://github.com/systemd/systemd/blob/main/src/libsystemd/sd-device/device-monitor.c)).
- **Sandboxing.**
  - Flatpak's `--device=usb` (Flatpak 1.15.11+) exposes raw `/dev/bus/usb` ([sandbox permissions](https://github.com/flatpak/flatpak-docs/blob/master/docs/sandbox-permissions.rst)).
  - The finer-grained [USB portal](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.Usb.xml) was added in xdg-desktop-portal 1.19.1 ([NEWS](https://github.com/flatpak/xdg-desktop-portal/blob/main/NEWS.md)). `EnumerateDevices()` gives a snapshot. `CreateSession()` then sends `DeviceEvents` signals when devices are "added, changed, or removed".
  - Only devices matching the app's `--usb=` queries (`vnd:`, `dev:`, `cls:`, `all`) are shown ([`flatpak-build-finish(1)`](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-build-finish.xml)). The user holds a per-app "blanket USB permission" that governs all use of the portal.
  - Only a fixed set of udev properties is passed to the app: `ID_VENDOR_ID`, `ID_MODEL_ID`, `ID_SERIAL`, `ID_SERIAL_SHORT`, `ID_REVISION`, `ID_MODEL_ENC`, `ID_VENDOR_ENC`, the `*_FROM_DATABASE` names, `ID_TYPE` and `ID_INPUT_JOYSTICK` ([`usb.c`](https://github.com/flatpak/xdg-desktop-portal/blob/main/desktop-portal/usb.c)).
  - Monitoring presence does not require acquiring (opening) the device **[inference from the interface]**.
  - Whether KDE's portal backend implements the USB permission dialog was not checked **[unverified]**.

### Faking it, and how hard that is

| Attack | What it takes | Difficulty | Source |
|---|---|---|---|
| Another app forging the event | Android: USB attach and detach broadcasts are system-only. Linux: the udev monitor rejects messages from other non-root users in its user namespace. A same-user process on the desktop is not excluded by that check. | Needs root or a compromised OS (or, on Linux, code already running as the same user) | [AOSP AndroidManifest.xml](https://github.com/aosp-mirror/platform_frameworks_base/blob/main/core/res/AndroidManifest.xml), [device-monitor.c](https://github.com/systemd/systemd/blob/main/src/libsystemd/sd-device/device-monitor.c) |
| Cloning descriptors with a gadget board | Descriptors are whatever the device's firmware says. The Linux kernel docs: "Anybody with access to a device gadget kit can fake descriptors and device info. Don't trust that." With configfs a Linux board sets `idVendor`, `idProduct`, `strings/0x409/serialnumber`, `manufacturer` and `product` by writing files. | Low: a cheap single-board computer or microcontroller with USB device mode, the target's IDs, and physical access to the port | [authorization.rst](https://github.com/torvalds/linux/blob/master/Documentation/usb/authorization.rst), [gadget_configfs.rst](https://github.com/torvalds/linux/blob/master/Documentation/usb/gadget_configfs.rst) |
| USB emulation tools | Facedancer boards "emulate USB devices", with emulations written in Python, and can sit in the middle of a connection | Low to moderate | [facedancer](https://github.com/greatscottgadgets/facedancer) |
| Reflashing a commodity device (BadUSB) | Psychson publishes custom firmware and patches for Phison 2251-03 USB-stick controllers | Moderate; depends on the controller | [Psychson](https://github.com/adamcaudill/Psychson) (SRLabs' original BadUSB material was blocked) |
| Learning the target's IDs | They are read from sysfs on any Linux machine the genuine device is plugged into ([sysfs-bus-usb](https://github.com/torvalds/linux/blob/master/Documentation/ABI/stable/sysfs-bus-usb)). A serial number is optional, so many devices can be told apart only by vendor and model **[inference from `usb_id`]**. | Low with brief access to the genuine device | [udev-builtin-usb_id.c](https://github.com/systemd/systemd/blob/main/src/udev/udev-builtin-usb_id.c) |
| Hash-based device matching | USBGuard's `hash` is "computed from the device attribute values and the USB descriptor data", which the device reports about itself. A full clone would therefore reproduce it **[inference]**. | As for cloning | [usbguard-rules.conf(5)](https://github.com/USBGuard/usbguard/blob/main/doc/man/usbguard-rules.conf.5.adoc) |

### What a user can do

- **Choose a device that reports a serial number, and match on vendor, product and serial.** On Linux, also match the port (`via-port` in USBGuard terms; the sysfs name encodes the port). This separates the chosen device from others of the same model. It does not stop a clone ([usb_id](https://github.com/systemd/systemd/blob/main/src/udev/udev-builtin-usb_id.c), [USBGuard](https://github.com/USBGuard/usbguard/blob/main/doc/man/usbguard-rules.conf.5.adoc)).
- **Keep the device, and the port, under the user's control.** Cloning needs the device's IDs and access to the port (see the table above).
- **Where real assurance is needed, use cryptography.** The kernel docs advise: "If you need something secure, use crypto and Certificate Authentication or stuff like that" ([authorization.rst](https://github.com/torvalds/linux/blob/master/Documentation/usb/authorization.rst)). That means a device that can answer a challenge, which goes beyond reading descriptors.
- **On Linux, block unknown USB devices.** Blocking with kernel `authorized_default` or USBGuard protects the computer against malicious USB devices ([authorization.rst](https://github.com/torvalds/linux/blob/master/Documentation/usb/authorization.rst)). It does not verify a device's identity.
- **On Android, the phone cannot identify a car or computer it is plugged into.** Only "USB power connected" is available, except for Android-accessory hosts ([BatteryManager](https://developer.android.com/reference/android/os/BatteryManager#BATTERY_PLUGGED_USB)). Any USB charger produces the same signal **[inference]**.

## Cross-cutting summary

| | Android: what the app sees | Linux: what the app sees | Can another app fake it? | Over-the-air or physical fake | Main user guard |
|---|---|---|---|---|---|
| Bluetooth | Paired device's address, name and transport. Link and profile connect/disconnect. Works in the background. `BLUETOOTH_CONNECT`, no location. | BlueZ `Device1` address, `Connected`, `Paired`. Readable by any local user. | No (system-only broadcasts; root-owned D-Bus name) | Name: trivial. Address clone: low to moderate. Impersonation without the link key (BIAS): moderate. | Match on the paired address; keep firmware updated; add a second condition |
| Wi-Fi | SSID, BSSID and security type, with fine location and location on. Joining can be caught in the background; leaving needs a running process. | NetworkManager `Ssid`, `HwAddress` (BSSID) and security flags. Readable by any local user. No portal. | No | SSID: trivial. BSSID: low. A secured network also needs the password. | WPA2/3-Personal with a password not widely shared; record the BSSID(s); separate guest network |
| USB | Only in host (OTG) mode: vendor/product IDs, strings, serial (with permission). When the phone is plugged into a host: only "USB power". | udev vendor/model/serial and port; Flatpak USB portal with hot-plug events | No (root-only event sources, bar same-user on Linux) | Low with a gadget board and the target's IDs | Serial plus port; physical control; cryptographic devices for real assurance |

## Gaps and open questions

1. **Does `ACTION_ACL_CONNECTED` fire before authentication?** If a device clones only the address and then fails authentication, does the broadcast still fire? The same question applies to BlueZ's `Connected`. The AOSP Bluetooth source was unreachable, and the docs say only "low level (ACL) connection". This decides whether the Bluetooth warning should cover address cloning alone, or only BIAS-class attacks. **Test on a device.**
2. **Background SSID reads.** Does a `NetworkCallback` with `FLAG_INCLUDE_LOCATION_INFO` return the SSID while the app is in the background when the user granted location only "while in use"? Or does it need `ACCESS_BACKGROUND_LOCATION`, or a `location` foreground service? The AOSP `WifiPermissionsUtil` source was unreachable. **Test on Android 13–16.**
3. **Android's rules for auto-joining a saved network** (matching on SSID and security type, and when a BSSID is used): `source.android.com` was blocked. The claim above is inferred from `WifiConfiguration.BSSID`.
4. **Patch status.** How far BIAS, BLUFFS, KNOB and SSID Confusion are fixed in current Android, BlueZ/Linux, car head units and routers is unknown. The Bluetooth SIG statements, Android security bulletins, CERT notes and the papers were all blocked.
5. **IEEE 802.11 text.** That a lookalike access point cannot complete a WPA2/WPA3-Personal handshake without the password, and what PMF prevents, rests on standard behaviour and on hostapd's comments, not on the standard itself.
6. **Learning a Bluetooth address.** How an attacker learns the full address, including the 16 bits Ubertooth does not recover, is not established here. Neither is whether car head units stay discoverable.
7. **Relay attacks.** Relaying Bluetooth or Wi-Fi to fake nearness, as opposed to cloning, was not researched.
8. **USB authentication.** The USB Type-C Authentication specification (usb.org) was blocked. Whether any consumer device the app might use supports it is unknown.
9. **Android Auto and accessory-mode head units.** Whether a third-party app can identify a car it is plugged into over USB was not researched.
10. **KDE Plasma specifics.** bluedevil, plasma-nm and xdg-desktop-portal-kde's support for the USB portal dialog could not be checked (`invent.kde.org`, `api.kde.org` blocked).
11. **Linux without NetworkManager.** iwd, systemd-networkd and ConnMan expose different APIs; not covered.
12. **Packaging.** How the Tauri app is packaged on Linux (Flatpak or not) decides whether the `--system-talk-name` and USB portal rules above apply. `tauri.app` was blocked.
13. **Freshness of the AOSP mirror.** `aosp-mirror/platform_frameworks_base` on GitHub may lag AOSP. The system-only broadcast entries cited are long-standing, but were not checked against current AOSP.

## Sources

Android (developer.android.com; "Last updated" date as shown on each page):

- [BluetoothDevice](https://developer.android.com/reference/android/bluetooth/BluetoothDevice), 2026-08-28
- [BluetoothA2dp](https://developer.android.com/reference/android/bluetooth/BluetoothA2dp), 2026-08-14
- [BluetoothHeadset](https://developer.android.com/reference/android/bluetooth/BluetoothHeadset), 2026-08-28
- [BluetoothAdapter](https://developer.android.com/reference/android/bluetooth/BluetoothAdapter), 2026-08-14
- [BluetoothProfile](https://developer.android.com/reference/android/bluetooth/BluetoothProfile), 2026-08-03
- [Bluetooth permissions](https://developer.android.com/develop/connectivity/bluetooth/bt-permissions), 2026-09-16
- [Companion device pairing](https://developer.android.com/develop/connectivity/bluetooth/companion-device-pairing), 2026-09-16
- [CompanionDeviceManager](https://developer.android.com/reference/android/companion/CompanionDeviceManager), 2026-09-16
- [DevicePresenceEvent](https://developer.android.com/reference/android/companion/DevicePresenceEvent), 2026-08-03
- [ObservingDevicePresenceRequest](https://developer.android.com/reference/android/companion/ObservingDevicePresenceRequest), 2026-08-03
- [Implicit broadcast exceptions](https://developer.android.com/develop/background-work/background-tasks/broadcasts/broadcast-exceptions), 2026-02-26
- [Foreground service types](https://developer.android.com/develop/background-work/services/fgs/service-types), 2026-09-21
- [Restrictions on starting FGS from the background](https://developer.android.com/develop/background-work/services/fgs/restrictions-bg-start), 2026-09-16
- [WifiInfo](https://developer.android.com/reference/android/net/wifi/WifiInfo), 2026-08-03
- [WifiManager](https://developer.android.com/reference/android/net/wifi/WifiManager), 2026-08-03
- [WifiConfiguration](https://developer.android.com/reference/android/net/wifi/WifiConfiguration), 2026-08-03
- [ConnectivityManager](https://developer.android.com/reference/android/net/ConnectivityManager), 2026-08-03
- [ConnectivityManager.NetworkCallback](https://developer.android.com/reference/android/net/ConnectivityManager.NetworkCallback), 2026-08-03
- [NetworkCapabilities](https://developer.android.com/reference/android/net/NetworkCapabilities), 2026-08-03
- [Wi-Fi permissions](https://developer.android.com/develop/connectivity/wifi/wifi-permissions), 2026-09-16
- [Android 9 behaviour changes](https://developer.android.com/about/versions/pie/android-9.0-changes-all), 2026-03-03
- [Android 10 privacy changes](https://developer.android.com/about/versions/10/privacy/changes), 2026-09-16
- [Location permissions](https://developer.android.com/develop/sensors-and-location/location/permissions), 2026-09-16
- [Background location](https://developer.android.com/develop/sensors-and-location/location/permissions/background), 2026-09-16
- [USB host overview](https://developer.android.com/develop/connectivity/usb/host), 2024-05-23
- [USB accessory overview](https://developer.android.com/develop/connectivity/usb/accessory), 2024-01-03
- [UsbManager](https://developer.android.com/reference/android/hardware/usb/UsbManager), 2026-08-03
- [UsbDevice](https://developer.android.com/reference/android/hardware/usb/UsbDevice), 2026-08-03
- [UsbAccessory](https://developer.android.com/reference/android/hardware/usb/UsbAccessory), 2026-08-03
- [BatteryManager](https://developer.android.com/reference/android/os/BatteryManager), 2026-08-03
- [Intent](https://developer.android.com/reference/android/content/Intent), 2026-09-16
- [AOSP `core/res/AndroidManifest.xml`](https://github.com/aosp-mirror/platform_frameworks_base/blob/main/core/res/AndroidManifest.xml) (GitHub mirror, `main`), read 2026-09-27

Linux and freedesktop (GitHub default branches, read 2026-09-27):

- BlueZ 5.87: [`doc/org.bluez.Device.rst`](https://github.com/bluez/bluez/blob/master/doc/org.bluez.Device.rst), [`src/bluetooth.conf`](https://github.com/bluez/bluez/blob/master/src/bluetooth.conf), [`src/main.conf`](https://github.com/bluez/bluez/blob/master/src/main.conf), [`tools/bdaddr.c`](https://github.com/bluez/bluez/blob/master/tools/bdaddr.c)
- NetworkManager 1.59.2-dev:
  - [`AccessPoint.xml`](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.AccessPoint.xml)
  - [`Device.Wireless.xml`](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.Device.Wireless.xml)
  - [`NetworkManager.xml`](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.xml)
  - [`Connection.Active.xml`](https://github.com/NetworkManager/NetworkManager/blob/main/introspection/org.freedesktop.NetworkManager.Connection.Active.xml)
  - [D-Bus policy](https://github.com/NetworkManager/NetworkManager/blob/main/src/core/org.freedesktop.NetworkManager.conf)
- xdg-desktop-portal (23.0 in development):
  - [`org.freedesktop.portal.Usb.xml`](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.Usb.xml)
  - [`org.freedesktop.portal.NetworkMonitor.xml`](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/org.freedesktop.portal.NetworkMonitor.xml)
  - [`desktop-portal/usb.c`](https://github.com/flatpak/xdg-desktop-portal/blob/main/desktop-portal/usb.c)
  - [`data/meson.build`](https://github.com/flatpak/xdg-desktop-portal/blob/main/data/meson.build)
  - [`NEWS.md`](https://github.com/flatpak/xdg-desktop-portal/blob/main/NEWS.md)
- Flatpak: [`sandbox-permissions.rst`](https://github.com/flatpak/flatpak-docs/blob/master/docs/sandbox-permissions.rst), [`flatpak-build-finish.xml`](https://github.com/flatpak/flatpak/blob/main/doc/flatpak-build-finish.xml)
- Linux 7.3-rc4:
  - [`include/uapi/linux/usb/ch9.h`](https://github.com/torvalds/linux/blob/master/include/uapi/linux/usb/ch9.h)
  - [`Documentation/ABI/stable/sysfs-bus-usb`](https://github.com/torvalds/linux/blob/master/Documentation/ABI/stable/sysfs-bus-usb)
  - [`Documentation/usb/authorization.rst`](https://github.com/torvalds/linux/blob/master/Documentation/usb/authorization.rst)
  - [`Documentation/usb/gadget_configfs.rst`](https://github.com/torvalds/linux/blob/master/Documentation/usb/gadget_configfs.rst)
- systemd 263~devel: [`src/udev/udev-builtin-usb_id.c`](https://github.com/systemd/systemd/blob/main/src/udev/udev-builtin-usb_id.c), [`src/libsystemd/sd-device/device-monitor.c`](https://github.com/systemd/systemd/blob/main/src/libsystemd/sd-device/device-monitor.c)
- USBGuard 1.1.4: [`usbguard-rules.conf.5.adoc`](https://github.com/USBGuard/usbguard/blob/main/doc/man/usbguard-rules.conf.5.adoc)
- hostapd 2.10 (Ubuntu 24.04 `2:2.10-21ubuntu0.4`): `/usr/share/doc/hostapd/examples/hostapd.conf` in [the package](http://archive.ubuntu.com/ubuntu/pool/universe/w/wpa/hostapd_2.10-21ubuntu0.4_amd64.deb). Upstream is `https://w1.fi/cgit/hostap/plain/hostapd/hostapd.conf` (blocked).

Security research and CVE records:

- [CVE-2020-10135 (BIAS)](https://github.com/CVEProject/cvelistV5/blob/main/cves/2020/10xxx/CVE-2020-10135.json), published 2020-05-19
- [CVE-2023-24023 (BLUFFS)](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/24xxx/CVE-2023-24023.json), published 2023-11-28
- [CVE-2019-9506 (KNOB)](https://github.com/CVEProject/cvelistV5/blob/main/cves/2019/9xxx/CVE-2019-9506.json), published 2019-08-14
- [CVE-2023-52424 (SSID Confusion)](https://github.com/CVEProject/cvelistV5/blob/main/cves/2023/52xxx/CVE-2023-52424.json), published 2024-05-17
- [francozappa/bias](https://github.com/francozappa/bias) and its [attack instructions](https://github.com/francozappa/bias/blob/master/bias/README.md)
- [francozappa/bluffs](https://github.com/francozappa/bluffs)
- [francozappa/knob](https://github.com/francozappa/knob)
- [greatscottgadgets/facedancer](https://github.com/greatscottgadgets/facedancer)
- [greatscottgadgets/ubertooth `ubertooth-rx`](https://github.com/greatscottgadgets/ubertooth/blob/master/host/doc/ubertooth-rx.md)
- [adamcaudill/Psychson](https://github.com/adamcaudill/Psychson)

Blocked by this environment's proxy (not read):

- Android and AOSP: `source.android.com`, `android.googlesource.com`, `cs.android.com`
- Linux, freedesktop and KDE: `networkmanager.dev`, `gitlab.freedesktop.org`, `www.freedesktop.org`, `specifications.freedesktop.org`, `flatpak.github.io`, `docs.flatpak.org`, `docs.kernel.org`, `git.kernel.org`, `www.kernel.org`, `invent.kde.org`, `api.kde.org`
- Standards bodies: `w1.fi`, `www.bluetooth.com`, `www.usb.org`, `standards.ieee.org`, `ieeexplore.ieee.org`, `www.wi-fi.org`
- Research and papers: `francozappa.github.io`, `knobattack.com`, `dl.acm.org`, `www.usenix.org`, `arxiv.org`, `eprint.iacr.org`, `mathyvanhoef.com`, `www.top10vpn.com`, `srlabs.de`, `www.blackhat.com`, `www.nccgroup.com`
- Vulnerability databases: `kb.cert.org`, `nvd.nist.gov`, `www.cve.org`
- Other: `tauri.app`, `en.wikipedia.org`
- The GitHub REST API was reachable but not enabled for these repositories, so commit SHAs could not be recorded. Versions come from each repository's version file instead.
