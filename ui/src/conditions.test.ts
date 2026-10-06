import { describe, expect, test } from "vitest";
import type { Condition } from "./api";
import {
  buildConditions,
  buildSun,
  buildSuns,
  checkCondition,
  describeCondition,
  describeConditions,
  describeSun,
  describeSuns,
  needsHome,
  newCondition,
  sunFormOf,
} from "./conditions";

describe("checking conditions", () => {
  test("every kind starts out valid except an empty date range", () => {
    for (const kind of ["days", "window", "season", "daylight", "darkness"] as const) {
      expect(checkCondition(newCondition(kind)).ok).toBe(true);
    }
    expect(checkCondition(newCondition("dates")).ok).toBe(false);
  });

  test("days need at least one", () => {
    expect(checkCondition({ kind: "days", days: [] }).ok).toBe(false);
  });

  test("a window can cross midnight but not be empty", () => {
    expect(checkCondition({ kind: "window", from: "22:00", to: "06:00" }).ok).toBe(true);
    expect(checkCondition({ kind: "window", from: "10:00", to: "10:00" }).ok).toBe(false);
    expect(checkCondition({ kind: "window", from: "", to: "10:00" }).ok).toBe(false);
    expect(checkCondition({ kind: "window", from: "24:00", to: "10:00" }).ok).toBe(false);
  });

  test("a date range must be real dates in order", () => {
    const d = (from: string, to: string): Condition => ({ kind: "dates", from, to });
    expect(checkCondition(d("2026-12-20", "2027-01-05")).ok).toBe(true);
    expect(checkCondition(d("2026-12-20", "2026-12-20")).ok).toBe(true);
    expect(checkCondition(d("2026-12-20", "2026-12-19")).ok).toBe(false);
    expect(checkCondition(d("2026-02-30", "2026-03-05")).ok).toBe(false);
  });

  test("a season is two days of the year, and may wrap the new year", () => {
    const s = (from: string, to: string): Condition => ({ kind: "season", from, to });
    expect(checkCondition(s("11-01", "02-28")).ok).toBe(true);
    expect(checkCondition(s("02-29", "03-01")).ok).toBe(true);
    expect(checkCondition(s("02-30", "03-01")).ok).toBe(false);
    expect(checkCondition(s("6-1", "08-31")).ok).toBe(false);
  });

  test("the first wrong condition is the error", () => {
    const r = buildConditions([newCondition("daylight"), { kind: "days", days: [] }]);
    expect(r).toEqual({ ok: false, error: "Choose at least one day for the condition." });
    expect(buildConditions([])).toEqual({ ok: true, value: [] });
  });
});

describe("saying conditions", () => {
  test("days", () => {
    expect(describeCondition({ kind: "days", days: ["MO", "TU", "WE", "TH", "FR"] })).toBe(
      "on weekdays",
    );
    expect(describeCondition({ kind: "days", days: ["SU", "SA"] })).toBe("on weekends");
    expect(describeCondition({ kind: "days", days: ["MO"] })).toBe("on Mondays");
    expect(describeCondition({ kind: "days", days: ["SA", "MO", "TH"] })).toBe(
      "on Mondays, Thursdays and Saturdays",
    );
  });

  test("windows, dates, seasons and daylight", () => {
    expect(describeCondition({ kind: "window", from: "22:00", to: "06:00" })).toBe(
      "between 22:00 and 06:00",
    );
    expect(describeCondition({ kind: "dates", from: "2026-12-20", to: "2027-01-05" })).toBe(
      "from 20 Dec 2026 to 5 Jan 2027",
    );
    expect(describeCondition({ kind: "season", from: "06-01", to: "08-31" })).toBe(
      "from 1 Jun to 31 Aug each year",
    );
    expect(describeCondition({ kind: "daylight" })).toBe("in daylight");
    expect(describeCondition({ kind: "darkness" })).toBe("in darkness");
  });

  test("several are listed with and", () => {
    expect(describeConditions([])).toBe("");
    expect(describeConditions([{ kind: "daylight" }])).toBe("in daylight");
    expect(
      describeConditions([
        { kind: "days", days: ["SA", "SU"] },
        { kind: "window", from: "08:00", to: "12:00" },
        { kind: "daylight" },
      ]),
    ).toBe("on weekends, between 08:00 and 12:00 and in daylight");
  });
});

describe("sun events", () => {
  test("offsets are before, at or after", () => {
    expect(buildSun({ event: "sunset", direction: "before", minutes: 30 })).toEqual({
      ok: true,
      value: { event: "sunset", offset_minutes: -30 },
    });
    expect(buildSun({ event: "sunrise", direction: "after", minutes: 15 })).toEqual({
      ok: true,
      value: { event: "sunrise", offset_minutes: 15 },
    });
    // "at" ignores the minutes left in the form.
    expect(buildSun({ event: "civil_dawn", direction: "at", minutes: 99999 })).toEqual({
      ok: true,
      value: { event: "civil_dawn", offset_minutes: 0 },
    });
  });

  test("an offset is whole minutes, up to twelve hours", () => {
    for (const minutes of [0, -5, 1.5, 721, Number.NaN]) {
      expect(buildSun({ event: "sunset", direction: "before", minutes }).ok).toBe(false);
    }
    expect(buildSun({ event: "sunset", direction: "after", minutes: 720 }).ok).toBe(true);
    expect(buildSuns([{ event: "sunset", direction: "after", minutes: 0 }]).ok).toBe(false);
  });

  test("the form comes back from a trigger", () => {
    expect(sunFormOf({ event: "sunset", offset_minutes: -30 })).toEqual({
      event: "sunset",
      direction: "before",
      minutes: 30,
    });
    expect(sunFormOf({ event: "sunrise", offset_minutes: 0 }).direction).toBe("at");
    const t = { event: "civil_dusk", offset_minutes: 45 } as const;
    expect(buildSun(sunFormOf(t))).toEqual({ ok: true, value: t });
  });

  test("wording", () => {
    expect(describeSun({ event: "sunset", offset_minutes: -30 })).toBe("30 minutes before sunset");
    expect(describeSun({ event: "sunrise", offset_minutes: 0 })).toBe("at sunrise");
    expect(describeSun({ event: "civil_dusk", offset_minutes: 90 })).toBe(
      "1 hour 30 minutes after civil dusk",
    );
    expect(describeSun({ event: "civil_dawn", offset_minutes: -1 })).toBe(
      "1 minute before civil dawn",
    );
    expect(
      describeSuns([
        { event: "civil_dawn", offset_minutes: 0 },
        { event: "sunset", offset_minutes: -30 },
      ]),
    ).toBe("at civil dawn and 30 minutes before sunset");
  });
});

describe("needing the home location", () => {
  test("only sun events and daylight or darkness do", () => {
    expect(needsHome([{ kind: "days", days: ["MO"] }], [])).toBe(false);
    expect(needsHome([{ kind: "daylight" }], [])).toBe(true);
    expect(needsHome([{ kind: "darkness" }], [])).toBe(true);
    expect(needsHome([], [{ event: "sunset", offset_minutes: 0 }])).toBe(true);
  });
});
