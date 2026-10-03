import { expect, test } from "vitest";
import { patternFor } from "./repeat";

test("the common patterns", () => {
  expect(patternFor("once", "2026-10-03", [])).toBeNull();
  expect(patternFor("daily", "2026-10-03", [])).toEqual({ kind: "daily" });
  expect(patternFor("weekdays", "2026-10-03", [])).toEqual({ kind: "weekdays" });
  expect(patternFor("weekly", "2026-10-03", ["MO", "TH"])).toEqual({
    kind: "weekly",
    days: ["MO", "TH"],
  });
  expect(patternFor("weekly", "2026-10-03", [])).toBeNull();
});

test("monthly by date and by weekday", () => {
  expect(patternFor("monthly_date", "2026-10-28", [])).toEqual({
    kind: "monthly_by_date",
    day: 28,
  });
  // 2026-10-03 is the first Saturday; 2026-10-30 is the fifth Friday: the last.
  expect(patternFor("monthly_weekday", "2026-10-03", [])).toEqual({
    kind: "monthly_by_weekday",
    ordinal: 1,
    weekday: "SA",
  });
  expect(patternFor("monthly_weekday", "2026-10-30", [])).toEqual({
    kind: "monthly_by_weekday",
    ordinal: -1,
    weekday: "FR",
  });
});
