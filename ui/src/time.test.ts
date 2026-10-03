import { expect, test } from "vitest";
import { toUnixSeconds } from "./time";

test("combines a local date and time into unix seconds", () => {
  const expected = Math.floor(new Date(2026, 9, 3, 9, 30).getTime() / 1000);
  expect(toUnixSeconds("2026-10-03", "09:30")).toBe(expected);
});

test("rejects an empty or invalid date or time", () => {
  expect(toUnixSeconds("", "09:30")).toBeNull();
  expect(toUnixSeconds("2026-10-03", "")).toBeNull();
  expect(toUnixSeconds("not-a-date", "09:30")).toBeNull();
});
