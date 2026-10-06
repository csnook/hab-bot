import { describe, expect, test } from "vitest";
import {
  describeCountdownWhen,
  describeSchedule,
  expiryLine,
  formatDate,
  lowerFirst,
  noteLine,
  overdueLine,
  summarize,
  type SummaryInput,
} from "./summary";

const base: SummaryInput = {
  title: "Take the bins out",
  list: "Household",
  priority: "medium",
  trigger: { kind: "schedule", pattern: { kind: "weekly", days: ["WE"] }, time: "18:00" },
  zone: null,
  overdue: null,
  expiries: [],
};

describe("the sentence", () => {
  test("a repeating reminder", () => {
    expect(summarize(base)).toBe(
      "In Household, remind me to take the bins out every Wednesday at 18:00, at Medium priority.",
    );
  });

  test("never mentions overdue or expiry unless overridden", () => {
    const s = summarize(base);
    expect(s).not.toMatch(/overdue|expire/i);
  });

  test("an overdue duration", () => {
    expect(summarize({ ...base, overdue: { kind: "after", seconds: 3 * 3600 } })).toBe(
      "In Household, remind me to take the bins out every Wednesday at 18:00, at Medium priority. " +
        "It goes overdue after 3 h.",
    );
  });

  test("an overdue at the next time a schedule matches", () => {
    const s = summarize({
      ...base,
      overdue: { kind: "next", pattern: { kind: "monthly_by_date", day: 1 }, time: "00:00" },
    });
    expect(s).toContain("It goes overdue the next 1st at 00:00.");
  });

  test("one expiry", () => {
    expect(summarize({ ...base, expiries: [{ kind: "after", seconds: 3600 }] })).toContain(
      "It expires after 1 h.",
    );
  });

  test("several expiries, whichever comes first", () => {
    const s = summarize({
      ...base,
      expiries: [
        { kind: "after", seconds: 3600 },
        { kind: "next", pattern: { kind: "daily" }, time: "23:59" },
      ],
    });
    expect(s).toContain("It expires after 1 h, or the next 23:59, whichever comes first.");
  });

  test("a one-off", () => {
    expect(
      summarize({
        ...base,
        title: "Call the plumber",
        list: "Personal",
        priority: "high",
        trigger: { kind: "once", date: "2026-10-03", time: "09:30" },
      }),
    ).toBe("In Personal, remind me to call the plumber on 3 Oct 2026 at 09:30, at High priority.");
  });

  test("a countdown", () => {
    expect(
      summarize({
        ...base,
        title: "Water the plants",
        trigger: { kind: "countdown", countdown: { amount: 3, unit: "days", at: "09:00" } },
      }),
    ).toContain("remind me to water the plants 3 days after it was last done, at 09:00,");
  });

  test("a pinned time zone", () => {
    expect(summarize({ ...base, zone: "Europe/London" })).toContain(
      "every Wednesday at 18:00 (in Europe/London time),",
    );
  });

  test("an empty title and an unfinished trigger still read as a sentence", () => {
    expect(summarize({ ...base, title: "  ", trigger: { kind: "incomplete" } })).toBe(
      "In Household, remind me to … at a time still to choose, at Medium priority.",
    );
  });

  test("a priority shows by name", () => {
    expect(summarize({ ...base, priority: "maximum" })).toContain("at Maximum priority.");
    expect(summarize({ ...base, priority: "minimum" })).toContain("at Minimum priority.");
  });
});

describe("pieces of it", () => {
  test("schedules", () => {
    expect(describeSchedule({ kind: "daily" }, "07:00")).toBe("every day at 07:00");
    expect(describeSchedule({ kind: "weekdays" }, "07:00")).toBe("every weekday at 07:00");
    expect(describeSchedule({ kind: "weekly", days: ["MO", "TH"] }, "07:00")).toBe(
      "every Monday and Thursday at 07:00",
    );
    expect(describeSchedule({ kind: "weekly", days: ["MO", "WE", "FR"] }, "07:00")).toBe(
      "every Monday, Wednesday and Friday at 07:00",
    );
    expect(describeSchedule({ kind: "monthly_by_date", day: 22 }, "07:00")).toBe(
      "on the 22nd of every month at 07:00",
    );
    expect(describeSchedule({ kind: "monthly_by_date", day: -1 }, "07:00")).toBe(
      "on the last day of every month at 07:00",
    );
    expect(describeSchedule({ kind: "monthly_by_weekday", ordinal: 2, weekday: "TU" }, "18:00")).toBe(
      "on the 2nd Tuesday of every month at 18:00",
    );
    expect(describeSchedule({ kind: "monthly_by_weekday", ordinal: -1, weekday: "FR" }, "18:00")).toBe(
      "on the last Friday of every month at 18:00",
    );
  });

  test("countdowns", () => {
    expect(describeCountdownWhen({ amount: 1, unit: "weeks", at: null })).toBe(
      "1 week after it was last done",
    );
    expect(describeCountdownWhen({ amount: 8, unit: "hours", at: null })).toBe(
      "8 hours after it was last done",
    );
  });

  test("dates", () => {
    expect(formatDate("2026-01-09")).toBe("9 Jan 2026");
    expect(formatDate("2026-12-31")).toBe("31 Dec 2026");
    expect(formatDate("")).toBe("");
    expect(formatDate("2026-13-01")).toBe("2026-13-01");
  });

  test("lowering the first letter keeps acronyms and names like iPhone", () => {
    expect(lowerFirst("Take the bins out")).toBe("take the bins out");
    expect(lowerFirst("NASA call")).toBe("NASA call");
    expect(lowerFirst("iPhone backup")).toBe("iPhone backup");
    expect(lowerFirst("A")).toBe("a");
    expect(lowerFirst("")).toBe("");
  });
});

describe("the one-line summaries of the folded sections", () => {
  test("overdue follows the priority until overridden", () => {
    expect(overdueLine(null, "after 1 h (Medium)")).toEqual({
      text: "after 1 h (Medium)",
      isDefault: true,
    });
    expect(overdueLine({ kind: "after", seconds: 7200 }, "after 1 h (Medium)")).toEqual({
      text: "after 2 h",
      isDefault: false,
    });
  });

  test("expiry shows the default, or what was added and that firing again still expires it", () => {
    expect(expiryLine([], "when it fires again")).toEqual({
      text: "when it fires again",
      isDefault: true,
    });
    expect(
      expiryLine([{ kind: "after", seconds: 5400 }], "when it fires again"),
    ).toEqual({ text: "after 90 min, or when it fires again", isDefault: false });
  });

  test("the note shows its first line, shortened", () => {
    expect(noteLine("")).toEqual({ text: "No note", isDefault: true });
    expect(noteLine("   \n ")).toEqual({ text: "No note", isDefault: true });
    expect(noteLine("Green bin too\nand the box")).toEqual({
      text: "Green bin too",
      isDefault: false,
    });
    const long = "x".repeat(80);
    expect(noteLine(long).text).toHaveLength(60);
    expect(noteLine(long).text.endsWith("…")).toBe(true);
  });
});
