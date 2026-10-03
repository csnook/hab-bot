import { expect, test } from "vitest";
import type { DeviceInfo } from "./api";
import { deviceLabel, lastSyncedText, removeQuestion } from "./devices";

const device = (over: Partial<DeviceInfo>): DeviceInfo => ({
  id: 1,
  name: "Desktop",
  portable: false,
  last_synced: 1000,
  this_device: false,
  ...over,
});

test("a device is listed by its name, or as unnamed until it arrives", () => {
  expect(deviceLabel(device({}))).toBe("Desktop");
  expect(deviceLabel(device({ name: null }))).toBe("Unnamed device");
  expect(deviceLabel(device({ name: "  " }))).toBe("Unnamed device");
});

test("when a device last synced", () => {
  expect(lastSyncedText(device({ last_synced: 1000 }), 1030)).toBe("just now");
  expect(lastSyncedText(device({ last_synced: 1000 }), 1000 + 5 * 60)).toBe("5 minutes ago");
  expect(lastSyncedText(device({ last_synced: null }), 1000)).toBe("not yet");
  expect(lastSyncedText(device({ this_device: true }), 9999)).toBe("this device");
});

test("the removal question names the device", () => {
  expect(removeQuestion(device({ name: "Laptop" }))).toContain("Remove Laptop");
});
