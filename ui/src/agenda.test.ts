import { describe, expect, test } from "vitest";
import type { Agenda, DueItem, EarlierItem, ExpectedItem } from "./api";
import { buildAgenda, closedWord, dayHeading, emptyDay } from "./agenda";
import { noFilters, setListShown, setPriorityShown } from "./filters";
import { isView, openingView, tipText, viewTitle, VIEWS } from "./views";

const H = 3600;
const D = 24 * H;
const D0 = 1_790_985_600; // a midnight
const days: Agenda["days"] = [
  [D0 - D, D0],
  [D0, D0 + D],
  [D0 + D, D0 + 2 * D],
];

const open = (id: string, list = "p", priority: DueItem["priority"] = "medium"): DueItem =>
  ({ occurrence_id: id, reminder_id: id, list_id: list, priority, title: id }) as DueItem;
const exp = (id: string, at: number, list = "p", priority: ExpectedItem["priority"] = "medium"): ExpectedItem =>
  ({ reminder_id: id, list_id: list, priority, title: id, scheduled_at: at }) as ExpectedItem;
const closed = (
  id: string,
  at: number,
  kind: EarlierItem["kind"] = "completed",
  list = "p",
): EarlierItem =>
  ({ occurrence_id: id, list_id: list, priority: "medium", title: id, scheduled_at: at, kind, paused: null }) as EarlierItem;

const data = (over: Partial<Agenda> = {}): Agenda => ({
  overdue: [],
  due: [],
  paused: [],
  days,
  expected: [],
  closed: [],
  ...over,
});

const now = D0 + 12 * H;

describe("the Now band", () => {
  test("lists overdue, then due, then paused, and stays out of the day list", () => {
    const v = buildAgenda(
      noFilters(),
      data({
        overdue: [open("o")],
        due: [open("d")],
        paused: [{ item: open("z"), pause: { until: null, list: false } }],
      }),
      now,
    );
    expect(v.now.map((r) => `${r.kind}:${r.item.occurrence_id}`)).toEqual(["overdue:o", "due:d", "paused:z"]);
    expect(v.days.every((d) => d.rows.length === 0)).toBe(true);
  });

  test("an empty band is empty", () => {
    expect(buildAgenda(noFilters(), data(), now).now).toEqual([]);
  });
});

describe("the day list", () => {
  test("is yesterday, today and tomorrow, even when empty", () => {
    const v = buildAgenda(noFilters(), data(), now);
    expect(v.days.map((d) => d.name)).toEqual(["yesterday", "today", "tomorrow"]);
    expect(v.days.map((d) => d.rows.length)).toEqual([0, 0, 0]);
    expect(emptyDay("yesterday")).not.toEqual(emptyDay("tomorrow"));
    expect(dayHeading("today")).toBe("Today");
  });

  test("puts each row in its day in time order, closed and expected together", () => {
    const v = buildAgenda(
      noFilters(),
      data({
        closed: [closed("y", D0 - 2 * H), closed("t1", D0 + 8 * H, "skipped")],
        expected: [exp("t2", D0 + 18 * H), exp("m", D0 + D + 9 * H), exp("m0", D0 + D)],
      }),
      now,
    );
    expect(v.days.map((d) => d.rows.map((r) => r.item.title))).toEqual([
      ["y"],
      ["t1", "t2"],
      ["m0", "m"],
    ]);
    expect(v.days[1]!.rows.map((r) => r.kind)).toEqual(["closed", "expected"]);
  });

  test("a day boundary belongs to the day it starts", () => {
    const v = buildAgenda(noFilters(), data({ expected: [exp("a", D0), exp("b", D0 + D)] }), now);
    expect(v.days[1]!.rows.map((r) => r.item.title)).toEqual(["a"]);
    expect(v.days[2]!.rows.map((r) => r.item.title)).toEqual(["b"]);
  });

  test("a row from before yesterday began is left out", () => {
    const v = buildAgenda(noFilters(), data({ closed: [closed("old", D0 - D - H)] }), now);
    expect(v.days.flatMap((d) => d.rows)).toEqual([]);
  });

  test("a 25-hour day is grouped by the core's boundaries, not by 24 hours", () => {
    const long: Agenda["days"] = [
      [D0 - D, D0],
      [D0, D0 + 25 * H],
      [D0 + 25 * H, D0 + 49 * H],
    ];
    const v = buildAgenda(noFilters(), data({ days: long, expected: [exp("late", D0 + 24 * H + 30 * 60)] }), now);
    expect(v.days[1]!.rows.map((r) => r.item.title)).toEqual(["late"]);
  });
});

describe("the rule marking now", () => {
  test("sits between what has passed and what is coming, in today only", () => {
    const v = buildAgenda(
      noFilters(),
      data({
        closed: [closed("c1", D0 + 8 * H), closed("c2", D0 + 11 * H)],
        expected: [exp("e1", D0 + 13 * H)],
      }),
      now,
    );
    expect(v.days[1]!.ruleBefore).toBe(2);
    expect(v.days[0]!.ruleBefore).toBeNull();
    expect(v.days[2]!.ruleBefore).toBeNull();
  });

  test("comes first when nothing has passed, and last when nothing is coming", () => {
    expect(buildAgenda(noFilters(), data({ expected: [exp("e", D0 + 13 * H)] }), now).days[1]!.ruleBefore).toBe(0);
    expect(buildAgenda(noFilters(), data({ closed: [closed("c", D0 + H)] }), now).days[1]!.ruleBefore).toBe(1);
    expect(buildAgenda(noFilters(), data(), now).days[1]!.ruleBefore).toBe(0);
  });

  test("is absent if now isn't in any day (stale data)", () => {
    const v = buildAgenda(noFilters(), data(), D0 + 5 * D);
    expect(v.days.map((d) => d.ruleBefore)).toEqual([null, null, null]);
  });
});

describe("the sidebar's filters", () => {
  const all = data({
    overdue: [open("o1", "home"), open("o2", "work")],
    due: [open("d1", "work", "low")],
    paused: [{ item: open("z1", "home"), pause: { until: null, list: false } }],
    closed: [closed("c1", D0 + H, "completed", "home"), closed("c2", D0 + 2 * H, "completed", "work")],
    expected: [exp("e1", D0 + 15 * H, "home"), exp("e2", D0 + 16 * H, "work", "high")],
  });

  test("hiding a list hides it from the band and every day", () => {
    const v = buildAgenda(setListShown(noFilters(), "home", false), all, now);
    expect(v.now.map((r) => r.item.occurrence_id)).toEqual(["o2", "d1"]);
    expect(v.days[1]!.rows.map((r) => r.item.title)).toEqual(["c2", "e2"]);
  });

  test("hiding a priority hides it too", () => {
    const f = setPriorityShown(setPriorityShown(noFilters(), "low", false), "high", false);
    const v = buildAgenda(f, all, now);
    expect(v.now.map((r) => r.item.occurrence_id)).toEqual(["o1", "o2", "z1"]);
    expect(v.days[1]!.rows.map((r) => r.item.title)).toEqual(["c1", "c2", "e1"]);
  });

  test("the rule still sits right when filtering leaves a day empty", () => {
    const f = setListShown(setListShown(noFilters(), "home", false), "work", false);
    const v = buildAgenda(f, all, now);
    expect(v.days[1]!.rows).toEqual([]);
    expect(v.days[1]!.ruleBefore).toBe(0);
  });

  test("the data is not changed", () => {
    buildAgenda(setListShown(noFilters(), "home", false), all, now);
    expect(all.overdue).toHaveLength(2);
  });
});

test("closed rows read as what happened", () => {
  expect(closedWord(closed("a", 1))).toBe("Done");
  expect(closedWord(closed("a", 1, "skipped"))).toBe("Skipped");
  expect(closedWord(closed("a", 1, "missed"))).toBe("Missed");
  expect(closedWord({ ...closed("a", 1, "skipped"), paused: { until: null, list: false } })).toBe("Skipped (paused)");
});

describe("the view switcher", () => {
  test("opens on the remembered view, and on Inbox when there is none", () => {
    expect(openingView("agenda")).toBe("agenda");
    expect(openingView("inbox")).toBe("inbox");
    expect(openingView(null)).toBe("inbox");
    expect(openingView(undefined)).toBe("inbox");
    expect(openingView("")).toBe("inbox");
  });

  test("a view this window doesn't have yet opens as Inbox", () => {
    for (const v of ["board", "calendar", "history", "nonsense"]) {
      expect(isView(v)).toBe(false);
      expect(openingView(v)).toBe("inbox");
    }
  });

  test("lists Inbox then Agenda, with titles", () => {
    expect(VIEWS.map((v) => v.label)).toEqual(["Inbox", "Agenda"]);
    expect(viewTitle("agenda")).toBe("Agenda");
  });

  test("the tip explains the views and that the last one is remembered", () => {
    expect(tipText()).toMatch(/Inbox/);
    expect(tipText()).toMatch(/Agenda/);
    expect(tipText()).toMatch(/last/);
  });
});
