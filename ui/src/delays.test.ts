import { describe, expect, test } from "vitest";
import type { Pattern, PriorityInfo } from "./api";
import {
  defaultOverdueText,
  describeDelay,
  describeExpiries,
  describeNext,
  durationSeconds,
  emptyNext,
  listDays,
  nextFormOf,
  nextSpec,
  ordinalWord,
  patternOf,
  splitDuration,
} from "./delays";

const info = (priority: PriorityInfo["priority"], due_interval: number): PriorityInfo => ({
  priority,
  name: priority,
  settings: {
    due_style: "gentle",
    due_interval,
    overdue_steps: [],
    overdue_interval: 600,
    ring_duration: null,
    server_wait: null,
    swipeable: true,
    breaks_do_not_disturb: false,
  },
});

describe("durations", () => {
  test("whole units only", () => {
    expect(durationSeconds(90, "minutes")).toBe(5400);
    expect(durationSeconds(2, "hours")).toBe(7200);
    expect(durationSeconds(1, "days")).toBe(86400);
    expect(durationSeconds(0, "hours")).toBeNull();
    expect(durationSeconds(-1, "hours")).toBeNull();
    expect(durationSeconds(1.5, "hours")).toBeNull();
    expect(durationSeconds(Number.NaN, "hours")).toBeNull();
  });

  test("filling the form picks the largest exact unit", () => {
    expect(splitDuration(86400)).toEqual({ amount: 1, unit: "days" });
    expect(splitDuration(2 * 86400)).toEqual({ amount: 2, unit: "days" });
    expect(splitDuration(3 * 3600)).toEqual({ amount: 3, unit: "hours" });
    expect(splitDuration(5400)).toEqual({ amount: 90, unit: "minutes" });
    expect(splitDuration(45)).toEqual({ amount: 1, unit: "minutes" });
    expect(splitDuration(0)).toEqual({ amount: 1, unit: "minutes" });
  });
});

describe("the next time a schedule matches", () => {
  test("each way of saying it", () => {
    expect(describeNext({ kind: "daily" }, "23:59")).toBe("the next 23:59");
    expect(describeNext({ kind: "weekdays" }, "09:00")).toBe("the next weekday at 09:00");
    expect(describeNext({ kind: "weekly", days: ["MO", "TH"] }, "09:00")).toBe(
      "the next Monday or Thursday at 09:00",
    );
    expect(describeNext({ kind: "monthly_by_date", day: 1 }, "00:00")).toBe(
      "the next 1st at 00:00",
    );
    expect(describeNext({ kind: "monthly_by_date", day: -1 }, "00:00")).toBe(
      "the next last day of the month at 00:00",
    );
    expect(describeNext({ kind: "monthly_by_weekday", ordinal: 2, weekday: "TU" }, "18:00")).toBe(
      "the next 2nd Tuesday at 18:00",
    );
    expect(describeNext({ kind: "monthly_by_weekday", ordinal: -1, weekday: "FR" }, "18:00")).toBe(
      "the next last Friday of the month at 18:00",
    );
  });

  test("ordinals", () => {
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 31].map(ordinalWord)).toEqual([
      "1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "31st",
    ]);
  });

  test("lists of days", () => {
    expect(listDays([])).toBe("");
    expect(listDays(["SA"])).toBe("Saturday");
    expect(listDays(["SA", "SU"])).toBe("Saturday and Sunday");
  });

  test("the form builds a spec, and a spec fills the form back", () => {
    const form = { ...emptyNext(), repeat: "monthly_date" as const, day: 1, time: "00:00" };
    expect(nextSpec(form)).toEqual({
      ok: true,
      value: { kind: "next", pattern: { kind: "monthly_by_date", day: 1 }, time: "00:00" },
    });
    const patterns: Pattern[] = [
      { kind: "daily" },
      { kind: "weekdays" },
      { kind: "weekly", days: ["MO", "SA"] },
      { kind: "monthly_by_date", day: -1 },
      { kind: "monthly_by_weekday", ordinal: 3, weekday: "WE" },
    ];
    for (const pattern of patterns) {
      const back = nextSpec(nextFormOf(pattern, "07:15"));
      expect(back).toEqual({ ok: true, value: { kind: "next", pattern, time: "07:15" } });
    }
  });

  test("mistakes are named", () => {
    expect(patternOf({ ...emptyNext(), repeat: "weekly", days: [] }).ok).toBe(false);
    expect(patternOf({ ...emptyNext(), repeat: "monthly_date", day: 0 }).ok).toBe(false);
    expect(patternOf({ ...emptyNext(), repeat: "monthly_date", day: 32 }).ok).toBe(false);
    expect(patternOf({ ...emptyNext(), repeat: "monthly_date", day: -1 }).ok).toBe(true);
    expect(patternOf({ ...emptyNext(), repeat: "monthly_weekday", ordinal: 5 }).ok).toBe(false);
    expect(patternOf({ ...emptyNext(), repeat: "monthly_weekday", weekday: "XX" }).ok).toBe(false);
    expect(nextSpec({ ...emptyNext(), time: "" }).ok).toBe(false);
    expect(nextSpec({ ...emptyNext(), time: "9:00" }).ok).toBe(false);
  });
});

describe("saying a delay", () => {
  test("durations and schedules", () => {
    expect(describeDelay({ kind: "after", seconds: 3600 })).toBe("after 1 h");
    expect(describeDelay({ kind: "after", seconds: 5400 })).toBe("after 90 min");
    expect(describeDelay({ kind: "after", seconds: 86400 })).toBe("after 1 day");
    expect(describeDelay({ kind: "after", seconds: 0 })).toBe("at once");
    expect(describeDelay({ kind: "next", pattern: { kind: "daily" }, time: "23:59" })).toBe(
      "the next 23:59",
    );
    expect(describeDelay({ kind: "other", rule: "FREQ=YEARLY" })).toContain("FREQ=YEARLY");
  });

  test("several expiries", () => {
    expect(
      describeExpiries([
        { kind: "after", seconds: 3600 },
        { kind: "next", pattern: { kind: "daily" }, time: "23:59" },
      ]),
    ).toBe("after 1 h, or the next 23:59");
  });
});

describe("the default overdue time, greyed with its source", () => {
  const infos = [info("medium", 3600), info("minimum", 0), info("high", 600)];
  test("names the priority", () => {
    expect(defaultOverdueText("medium", infos)).toBe("after 1 h (Medium)");
    expect(defaultOverdueText("high", infos)).toBe("after 10 min (High)");
    expect(defaultOverdueText("minimum", infos)).toBe("at once (Minimum)");
  });
  test("still names it before the table has loaded", () => {
    expect(defaultOverdueText("low", [])).toBe("follows the priority (Low)");
  });
});
