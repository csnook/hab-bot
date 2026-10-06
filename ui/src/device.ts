import type { AlertStyle, DeviceQuiet, LoudestAlert, ThisDevice } from "./api";
import { styleName } from "./priorities";
import { untilLabel } from "./quiet";

/** The loudest-alert choices, loudest first: an alarm caps nothing. */
export const LOUDEST_CHOICES: Array<[AlertStyle, string]> = [
  ["alarm", "Alarm (no limit)"],
  ["insistent", "Insistent"],
  ["gentle", "Gentle"],
  ["silent", "Silent"],
];

/** The default: no cap, and so nothing for Maximum to be exempt from. */
export const NO_CAP: LoudestAlert = { style: "alarm", caps_maximum: false };

/** Whether a cap changes anything on this device. */
export const isCapped = (l: LoudestAlert): boolean => l.style !== "alarm";

/** "No limit", "Insistent at most, Maximum still gets through", "Gentle at most, Maximum too". */
export function describeLoudest(l: LoudestAlert): string {
  if (!isCapped(l)) return "No limit";
  const maximum = l.caps_maximum ? "Maximum too" : "Maximum still gets through";
  return `${styleName(l.style)} at most, ${maximum}`;
}

/** "Quiet until 19:00, Maximum left out", or "Quiet until Tue 08:00, Maximum too". */
export function describeQuiet(q: DeviceQuiet, nowSeconds: number): string {
  const maximum = q.include_maximum ? "Maximum too" : "Maximum left out";
  return `Quiet until ${untilLabel(q.until, nowSeconds)}, ${maximum}`;
}

/** The quiet setting if it is still in force, as the core reads it. */
export function quietNow(d: Pick<ThisDevice, "quiet">, nowSeconds: number): DeviceQuiet | null {
  return d.quiet && d.quiet.until > nowSeconds ? d.quiet : null;
}

/** Why a device name can't be kept, or null. */
export function checkDeviceName(name: string): string | null {
  return name.trim() === "" ? "This device needs a name." : null;
}

/** "portable" or "stationary", with what each means for connections. */
export function portableNote(portable: boolean): string {
  return portable
    ? "Its Wi-Fi and Bluetooth connections say where you are."
    : "Its connections are ignored by default for Wi-Fi and Bluetooth sources.";
}
