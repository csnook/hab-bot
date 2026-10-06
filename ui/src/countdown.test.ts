import { expect, test } from "vitest";
import { countdownFor, describeCountdown, hasTimeOfDay, lastDoneSeconds } from "./countdown";

test("only days and weeks have a time of day", () => {
  expect(hasTimeOfDay("hours")).toBe(false);
  expect(hasTimeOfDay("minutes")).toBe(false);
  expect(hasTimeOfDay("days")).toBe(true);
  expect(countdownFor(3, "days", "09:00")).toEqual({
    ok: true,
    value: { amount: 3, unit: "days", at: "09:00" },
  });
  // A time of day left over from choosing days is dropped for hours.
  expect(countdownFor(8, "hours", "09:00")).toEqual({
    ok: true,
    value: { amount: 8, unit: "hours", at: null },
  });
  expect(countdownFor(3, "days", "")).toEqual({
    ok: true,
    value: { amount: 3, unit: "days", at: null },
  });
  expect(countdownFor(0, "days", "").ok).toBe(false);
  expect(countdownFor(1.5, "days", "").ok).toBe(false);
});

test("when it was last done: now by default, never, or a time", () => {
  const now = Math.floor(new Date("2026-10-03T12:00:00").getTime() / 1000);
  expect(lastDoneSeconds("now", now, "")).toEqual({ ok: true, value: now });
  expect(lastDoneSeconds("never", now, "")).toEqual({ ok: true, value: null });
  expect(lastDoneSeconds("at", now, "2026-10-01T09:40")).toEqual({
    ok: true,
    value: Math.floor(new Date("2026-10-01T09:40").getTime() / 1000),
  });
  expect(lastDoneSeconds("at", now, "").ok).toBe(false);
  expect(lastDoneSeconds("at", now, "2026-10-04T09:40").ok).toBe(false);
});

test("describing a countdown", () => {
  expect(describeCountdown({ amount: 3, unit: "days", at: "09:00" })).toBe("3 days, at 09:00");
  expect(describeCountdown({ amount: 1, unit: "hours", at: null })).toBe("1 hour");
});
