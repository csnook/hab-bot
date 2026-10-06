# A device limits its own alerts, and records the style it used

Settings → This device (spec: Alerts → Settings per device) has two limits that live on the device and never sync: the loudest style it uses (a cap on Silent < Gentle < Insistent < Alarm, with Maximum exempt unless the cap says otherwise) and "Quiet this device until…" (everything silent until a time, Maximum left out unless included). They are device settings in the core (`loudest_style`, `loudest_caps_maximum`, `quiet_device`), read by the alerter each pass (`Core::device_limits`) and applied where Do Not Disturb is: after the style is worked out, before the server check and the history. A device's name and whether it is portable are the user's personal settings: `DeviceNamed` already existed, and `DevicePortable` is new in format 13 (the device that authored it is the one it describes).

Neither limit is a snooze. Nothing is written per occurrence and no occurrence's `snoozed_until` changes, so other devices alert in full, the occurrence goes overdue on schedule, and the tray and window list it as they do any open occurrence. The alerter looks again when the quiet setting ends, and then alerts at the current level.

The history records the style the device alerted in, as it already does for a Do Not Disturb downgrade: a capped or quieted alert is recorded as the capped or silent style, by that device. The history is of what each device did, and the intended style is always derivable from the priority.

The last-chance alert is limited like any other, so a quiet device shows it silently rather than not at all (a hold never silently turns into a miss; the notification is still there). The server check before an alert is made whatever the style: it exists to catch an occurrence closed elsewhere, which a silent notification would get wrong as much as an alarm.

## Considered Options

- **Record the intended style, and cap only the notification**: rejected. The history would say an alarm rang on a device that showed a silent notification.
- **Implement quiet as a snooze with a device scope**: rejected by the spec ("It isn't a snooze, so other devices still alert") and because snoozes are synced events.
- **Store the personal portable flag on the server's device record**: rejected. That record is signed when the device joins and the server reads it; the spec puts it in the personal list, unread by the server.

## Consequences

- An older app (format 12) holds `DevicePortable` without applying it; its Devices list shows the portable flag from when the device joined.
- A web client or Android device implements the same two limits against its own storage (spec: Web client, Android).
