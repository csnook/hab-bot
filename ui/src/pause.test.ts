import { describe, expect, test } from "vitest";
import {
  activePause,
  causeText,
  dayOf,
  dayStart,
  formatPauseDate,
  listPauseLine,
  nextWeekOf,
  pauseAction,
  pauseFormOf,
  pauseLine,
  pauseSummary,
  pauseUntil,
  tomorrowOf,
  untilText,
} from "./pause";

const local = (s: string) => Math.floor(new Date(s).getTime() / 1000);
const now = local("2026-10-03T12:00:00");
const march = local("2027-03-03T00:00:00");

describe("when a pause counts", () => {
  test("only from when it began until it ends, or until it is resumed", () => {
    const p = { from: 100, until: 200 };
    expect(activePause(p, 99)).toBeNull();
    expect(activePause(p, 100)).toBe(p);
    expect(activePause(p, 199)).toBe(p);
    expect(activePause(p, 200)).toBeNull();
    const forever = { from: 100, until: null };
    expect(activePause(forever, 10 ** 12)).toBe(forever);
    expect(activePause(null, 100)).toBeNull();
  });
});

describe("saying a day", () => {
  test("a day and month, and the year only when it isn't this one", () => {
    expect(formatPauseDate(local("2026-10-23T09:00:00"), now)).toBe("23 October");
    expect(formatPauseDate(march, now)).toBe("3 March 2027");
    expect(formatPauseDate(local("2026-03-03T00:00:00"), now)).toBe("3 March");
  });

  test("until a day, or until resumed", () => {
    expect(untilText(local("2026-12-01T00:00:00"), now)).toBe("until 1 December");
    expect(untilText(null, now)).toBe("until resumed");
  });
});

describe("the folded Pause section's one line", () => {
  const own = (until: number | null) => ({ from: now - 10, until });
  test("says Paused until a date, as the criteria give it", () => {
    expect(pauseLine(own(local("2026-10-20T00:00:00")), null, now)).toEqual({
      text: "Paused until 20 October",
      isDefault: false,
    });
  });
  test("says until resumed when there is no end", () => {
    expect(pauseLine(own(null), null, now).text).toBe("Paused until resumed");
  });
  test("says so when only the list is paused", () => {
    expect(pauseLine(null, own(local("2026-10-20T00:00:00")), now).text).toBe(
      "Paused with its list until 20 October",
    );
  });
  test("its own pause is the one shown when both apply", () => {
    expect(pauseLine(own(null), own(local("2026-10-20T00:00:00")), now).text).toBe(
      "Paused until resumed",
    );
  });
  test("a pause that has run out is not paused, and is greyed as the default", () => {
    expect(pauseLine({ from: 1, until: now - 1 }, null, now)).toEqual({
      text: "Not paused",
      isDefault: true,
    });
    expect(pauseLine(null, null, now).text).toBe("Not paused");
  });
  test("a list's pause in the sidebar", () => {
    expect(listPauseLine({ from: 1, until: march }, now)).toBe("Paused until 3 March 2027");
    expect(listPauseLine({ from: 1, until: now }, now)).toBeNull();
    expect(listPauseLine(null, now)).toBeNull();
  });
});

describe("what skipped an occurrence", () => {
  test("names the pause, or the list's", () => {
    expect(causeText({ until: march, list: false }, now)).toBe("the pause until 3 March 2027");
    expect(causeText({ until: null, list: true }, now)).toBe("the list's pause until resumed");
  });
});

describe("days", () => {
  test("a day starts at midnight, local time", () => {
    expect(dayStart("2026-10-04")).toBe(local("2026-10-04T00:00:00"));
    expect(dayStart("")).toBeNull();
    expect(dayStart("4 October")).toBeNull();
    expect(dayOf(local("2026-10-04T23:59:00"))).toBe("2026-10-04");
  });
  test("tomorrow and next week", () => {
    expect(tomorrowOf(now)).toBe("2026-10-04");
    expect(nextWeekOf(now)).toBe("2026-10-10");
    expect(tomorrowOf(local("2026-12-31T23:00:00"))).toBe("2027-01-01");
  });
});

describe("the form", () => {
  test("opens as it stands: off, until a day, or until resumed", () => {
    expect(pauseFormOf(null, now).mode).toBe("off");
    expect(pauseFormOf({ from: 1, until: now - 5 }, now).mode).toBe("off");
    expect(pauseFormOf({ from: 1, until: local("2026-10-20T00:00:00") }, now)).toEqual({
      mode: "until",
      date: "2026-10-20",
    });
    expect(pauseFormOf({ from: 1, until: null }, now).mode).toBe("resumed");
  });

  test("pausing until a day pauses until the start of it", () => {
    const a = pauseAction({ mode: "until", date: "2026-10-20" }, null, now);
    expect(a).toEqual({ ok: true, value: { kind: "pause", until: local("2026-10-20T00:00:00") } });
  });

  test("a day that isn't after today is refused", () => {
    for (const date of ["2026-10-03", "2026-09-01"]) {
      const a = pauseAction({ mode: "until", date }, null, now);
      expect(a.ok).toBe(false);
    }
    expect(pauseAction({ mode: "until", date: "" }, null, now).ok).toBe(false);
  });

  test("turning a pause off resumes it, and does nothing if it isn't paused", () => {
    const paused = { from: 1, until: null };
    expect(pauseAction({ mode: "off", date: "" }, paused, now)).toEqual({
      ok: true,
      value: { kind: "resume" },
    });
    expect(pauseAction({ mode: "off", date: "" }, null, now)).toEqual({
      ok: true,
      value: { kind: "none" },
    });
    // One that ran out is already over.
    expect(pauseAction({ mode: "off", date: "" }, { from: 1, until: 5 }, now).ok).toBe(true);
    expect(pauseAction({ mode: "off", date: "" }, { from: 1, until: 5 }, now)).toEqual({
      ok: true,
      value: { kind: "none" },
    });
  });

  test("leaving a pause as it is sends nothing; moving its end pauses again", () => {
    const until = local("2026-10-20T00:00:00");
    const paused = { from: 1, until };
    expect(pauseAction({ mode: "until", date: "2026-10-20" }, paused, now)).toEqual({
      ok: true,
      value: { kind: "none" },
    });
    expect(pauseAction({ mode: "until", date: "2026-10-25" }, paused, now)).toEqual({
      ok: true,
      value: { kind: "pause", until: local("2026-10-25T00:00:00") },
    });
    expect(pauseAction({ mode: "resumed", date: "" }, paused, now)).toEqual({
      ok: true,
      value: { kind: "pause", until: null },
    });
    expect(pauseAction({ mode: "resumed", date: "" }, { from: 1, until: null }, now)).toEqual({
      ok: true,
      value: { kind: "none" },
    });
  });

  test("the quick pause from the More menu", () => {
    expect(pauseUntil(true, "", now)).toEqual({ ok: true, value: null });
    expect(pauseUntil(false, "2026-10-20", now)).toEqual({
      ok: true,
      value: local("2026-10-20T00:00:00"),
    });
    expect(pauseUntil(false, "2026-10-03", now).ok).toBe(false);
  });

  test("the editor's sentence mentions a pause only while the form pauses", () => {
    expect(pauseSummary({ mode: "off", date: "2026-10-20" }, now)).toBeNull();
    expect(pauseSummary({ mode: "until", date: "2026-10-20" }, now)).toBe("until 20 October");
    expect(pauseSummary({ mode: "until", date: "" }, now)).toBeNull();
    expect(pauseSummary({ mode: "resumed", date: "" }, now)).toBe("until resumed");
  });
});
