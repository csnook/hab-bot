import { expect, test } from "vitest";
import {
  clock,
  expiryNote,
  expiryWarning,
  futureDateTime,
  lastChanceAt,
  nextTimeOfDay,
  optionLabel,
} from "./snooze";

const at = (h: number, m: number, day = 3) => Math.floor(new Date(2026, 9, day, h, m).getTime() / 1000);

test("the options are labelled by their length", () => {
  expect(optionLabel({ kind: "interval", seconds: 86_400 })).toBe("1 day");
  expect(optionLabel({ kind: "interval", seconds: 600 })).toBe("10 minutes");
  expect(optionLabel({ kind: "hour", seconds: 3_600 })).toBe("1 hour");
  expect(optionLabel({ kind: "interval", seconds: 7_200 })).toBe("2 hours");
  expect(optionLabel({ kind: "tomorrow_morning", seconds: null })).toBe("Tomorrow morning");
});

test("until a time means the next time the clock reads it", () => {
  expect(nextTimeOfDay("19:30", at(9, 0))).toBe(at(19, 30));
  expect(nextTimeOfDay("07:00", at(9, 0))).toBe(at(7, 0, 4));
  expect(nextTimeOfDay("09:00", at(9, 0))).toBe(at(9, 0, 4));
  expect(nextTimeOfDay("", at(9, 0))).toBeNull();
  expect(nextTimeOfDay("25:00", at(9, 0))).toBeNull();
});

test("a picked date and time has to be ahead", () => {
  expect(futureDateTime("2026-10-03T19:30", at(9, 0))).toBe(at(19, 30));
  expect(futureDateTime("2026-10-03T08:00", at(9, 0))).toBeNull();
  expect(futureDateTime("", at(9, 0))).toBeNull();
});

test("a last-chance alert comes 10 minutes before an expiry inside the snooze", () => {
  const expiry = at(23, 59);
  expect(clock(expiry)).toBe("23:59");
  expect(lastChanceAt(expiry, at(9, 0), at(23, 50))).toBe(at(23, 49));
  expect(lastChanceAt(expiry, at(9, 0), at(23, 49))).toBeNull();
  expect(lastChanceAt(expiry, at(23, 55), at(23, 58))).toBeNull();
  expect(lastChanceAt(null, at(9, 0), at(23, 58))).toBeNull();
  expect(expiryWarning(expiry, at(9, 0), at(23, 58))).toContain("Expires at 23:59");
  expect(expiryWarning(expiry, at(9, 0), at(10, 0))).toBeNull();
  expect(expiryNote(expiry)).toContain("last-chance alert comes at 23:49");
  expect(expiryNote(null)).toBeNull();
});
