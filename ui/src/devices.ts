import type { DeviceInfo } from "./api";
import { ago } from "./time";

/** What the device list calls a device whose name hasn't reached this one yet. */
export function deviceLabel(d: DeviceInfo): string {
  return d.name?.trim() ? d.name : "Unnamed device";
}

/** "this device", "just now", "5 minutes ago", or "not yet". */
export function lastSyncedText(d: DeviceInfo, nowSeconds: number): string {
  if (d.this_device) return "this device";
  return d.last_synced === null ? "not yet" : ago(d.last_synced, nowSeconds);
}

/** The question asked before a device is removed. */
export function removeQuestion(d: DeviceInfo): string {
  return (
    `Remove ${deviceLabel(d)} from your account? It will stop syncing, and the keys of your ` +
    `lists will be changed so it can't read anything new.`
  );
}
