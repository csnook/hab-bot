import { expect, test } from "vitest";
import {
  LOUDEST_CHOICES,
  NO_CAP,
  checkDeviceName,
  describeLoudest,
  describeQuiet,
  isCapped,
  portableNote,
  quietNow,
} from "./device";

const at = (h: number, m: number, day = 3) => Math.floor(new Date(2026, 9, day, h, m).getTime() / 1000);

test("the loudest-alert choices run from no limit down to silent", () => {
  expect(LOUDEST_CHOICES.map(([s]) => s)).toEqual(["alarm", "insistent", "gentle", "silent"]);
  expect(isCapped(NO_CAP)).toBe(false);
  expect(isCapped({ style: "gentle", caps_maximum: false })).toBe(true);
});

test("a cap says whether Maximum still gets through", () => {
  expect(describeLoudest(NO_CAP)).toBe("No limit");
  // With no cap there is nothing for Maximum to be exempt from.
  expect(describeLoudest({ style: "alarm", caps_maximum: true })).toBe("No limit");
  expect(describeLoudest({ style: "insistent", caps_maximum: false })).toBe(
    "Insistent at most, Maximum still gets through",
  );
  expect(describeLoudest({ style: "gentle", caps_maximum: true })).toBe("Gentle at most, Maximum too");
});

test("quiet says until when and whether Maximum is included", () => {
  const now = at(17, 0);
  expect(describeQuiet({ until: at(19, 0), include_maximum: false }, now)).toBe(
    "Quiet until 19:00, Maximum left out",
  );
  expect(describeQuiet({ until: at(19, 0), include_maximum: true }, now)).toBe(
    "Quiet until 19:00, Maximum too",
  );
  expect(describeQuiet({ until: at(8, 0, 4), include_maximum: false }, now)).toMatch(
    /^Quiet until \w+ 08:00, Maximum left out$/,
  );
});

test("a quiet setting that has run out is not in force", () => {
  const q = { until: at(19, 0), include_maximum: false };
  expect(quietNow({ quiet: q }, at(18, 0))).toEqual(q);
  expect(quietNow({ quiet: q }, at(19, 0))).toBeNull();
  expect(quietNow({ quiet: null }, at(18, 0))).toBeNull();
});

test("a device needs a name", () => {
  expect(checkDeviceName("  ")).not.toBeNull();
  expect(checkDeviceName(" Study desktop ")).toBeNull();
});

test("portable and stationary say what they mean for connections", () => {
  expect(portableNote(true)).toMatch(/say where you are/);
  expect(portableNote(false)).toMatch(/ignored/);
});
