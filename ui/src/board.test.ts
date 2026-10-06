import { describe, expect, test } from "vitest";
import type { BoardCard, DueItem, ExpectedItem } from "./api";
import {
  buildBoard,
  columnOf,
  dayTime,
  FINISHED_DAYS,
  holdingPause,
  pauseOffer,
  statusLine,
  fakingText,
  hiddenCards,
  COLUMNS,
} from "./board";
import { noFilters, setListShown, setPriorityShown } from "./filters";
import { isView, openingView, VIEWS } from "./views";

const H = 3600;
const D = 86_400;
const now = new Date(2026, 9, 3, 12, 0).getTime() / 1000;
const at = (days: number, h: number, m = 0) => new Date(2026, 9, 3 + days, h, m).getTime() / 1000;

const open = (over: Partial<DueItem> = {}): DueItem =>
  ({
    occurrence_id: "o",
    reminder_id: "r",
    list_id: "p",
    priority: "medium",
    title: "t",
    scheduled_at: at(0, 9),
    fired_at: at(0, 9),
    overdue_at: at(0, 10),
    ...over,
  }) as DueItem;
const next = (t: number): ExpectedItem =>
  ({ reminder_id: "r", list_id: "p", priority: "medium", title: "t", scheduled_at: t }) as ExpectedItem;

let n = 0;
const card = (over: Partial<BoardCard> = {}): BoardCard => ({
  reminder_id: `r${++n}`,
  list_id: "p",
  title: "Card",
  priority: "medium",
  repeats: true,
  open: null,
  next: null,
  pause: null,
  last_closed_at: null,
  faking: "none",
  ...over,
});

describe("which column a card is in", () => {
  test("an open occurrence is Overdue once past its overdue time, otherwise Due", () => {
    expect(columnOf(card({ open: open({ overdue_at: now - 1 }) }), now)).toBe("overdue");
    expect(columnOf(card({ open: open({ overdue_at: now }) }), now)).toBe("overdue");
    expect(columnOf(card({ open: open({ overdue_at: now + 1 }) }), now)).toBe("due");
  });

  test("a reminder with an open occurrence stays in Overdue or Due even when paused", () => {
    const pause = { until: null, list: false };
    expect(columnOf(card({ open: open({ overdue_at: now - 1 }), pause }), now)).toBe("overdue");
    expect(columnOf(card({ open: open({ overdue_at: now + 9 }), pause }), now)).toBe("due");
  });

  test("a reminder with a next expected occurrence is Expected", () => {
    expect(columnOf(card({ next: next(now + H) }), now)).toBe("expected");
  });

  test("a paused one with nothing open is Paused, not Expected", () => {
    const c = card({ next: next(now + H), pause: { until: now + D, list: false } });
    expect(columnOf(c, now)).toBe("paused");
    expect(columnOf(card({ next: next(now + H), pause: { until: null, list: true } }), now)).toBe(
      "paused",
    );
  });

  test("a pause that has run out since the data was read no longer holds", () => {
    const c = card({ next: next(now + H), pause: { until: now - 1, list: false } });
    expect(holdingPause(c.pause, now)).toBeNull();
    expect(columnOf(c, now)).toBe("expected");
  });

  test("a one-off that was closed is Finished, even under a pause", () => {
    expect(columnOf(card({ repeats: false, last_closed_at: now - H }), now)).toBe("finished");
    const c = card({ repeats: false, last_closed_at: now - H, pause: { until: null, list: false } });
    expect(columnOf(c, now)).toBe("finished");
  });

  test("a reminder with nothing open, expected or closed is Watching", () => {
    expect(columnOf(card(), now)).toBe("watching");
  });

  test("nothing is ever Waiting: the core has no waiting state", () => {
    const all = [
      card({ open: open() }),
      card({ next: next(now + H) }),
      card(),
      card({ last_closed_at: now }),
    ];
    expect(all.map((c) => columnOf(c, now))).not.toContain("waiting");
    expect(buildBoard(noFilters(), all, now).map((c) => c.id)).not.toContain("waiting");
  });
});

describe("the columns", () => {
  test("come in the spec's order and empty ones are left out", () => {
    expect(COLUMNS).toEqual([
      "overdue",
      "due",
      "waiting",
      "expected",
      "watching",
      "paused",
      "finished",
    ]);
    expect(buildBoard(noFilters(), [], now)).toEqual([]);
    const cols = buildBoard(
      noFilters(),
      [
        card({ last_closed_at: now - H }),
        card({ next: next(now + H) }),
        card({ open: open({ overdue_at: now - 1 }) }),
      ],
      now,
    );
    expect(cols.map((c) => c.title)).toEqual(["Overdue", "Expected", "Finished"]);
  });

  test("Overdue goes by priority, then the longest overdue", () => {
    const a = card({ title: "a", priority: "low", open: open({ overdue_at: now - 5 * H }) });
    const b = card({ title: "b", priority: "high", open: open({ overdue_at: now - H }) });
    const c = card({ title: "c", priority: "high", open: open({ overdue_at: now - 3 * H }) });
    const [col] = buildBoard(noFilters(), [a, b, c], now);
    expect(col.cards.map((x) => x.title)).toEqual(["c", "b", "a"]);
  });

  test("Due goes by when it fired, Expected by time then priority", () => {
    const d1 = card({ title: "late", open: open({ fired_at: now - H, overdue_at: now + H }) });
    const d2 = card({ title: "early", open: open({ fired_at: now - 2 * H, overdue_at: now + H }) });
    const e1 = card({ title: "x", priority: "low", next: next(now + H) });
    const e2 = card({ title: "y", priority: "high", next: next(now + H) });
    const e3 = card({ title: "z", priority: "maximum", next: next(now + 2 * H) });
    const cols = buildBoard(noFilters(), [d1, d2, e1, e2, e3], now);
    expect(cols[0].cards.map((x) => x.title)).toEqual(["early", "late"]);
    expect(cols[1].cards.map((x) => x.title)).toEqual(["y", "x", "z"]);
  });

  test("Paused goes by priority then title; Finished by latest first", () => {
    const pause = { until: null, list: false };
    const p = [
      card({ title: "b", next: next(now + H), pause }),
      card({ title: "a", next: next(now + H), pause }),
      card({ title: "c", priority: "high", next: next(now + H), pause }),
    ];
    expect(buildBoard(noFilters(), p, now)[0].cards.map((x) => x.title)).toEqual(["c", "a", "b"]);
    const f = [
      card({ title: "old", last_closed_at: now - 2 * D }),
      card({ title: "new", last_closed_at: now - H }),
    ];
    expect(buildBoard(noFilters(), f, now)[0].cards.map((x) => x.title)).toEqual(["new", "old"]);
  });

  test("Finished shows the last 7 days and counts the older ones for Show older", () => {
    const recent = card({ title: "recent", last_closed_at: now - (FINISHED_DAYS * D - 10) });
    const edge = card({ title: "edge", last_closed_at: now - FINISHED_DAYS * D });
    const old = card({ title: "old", last_closed_at: now - FINISHED_DAYS * D - 1 });
    const [col] = buildBoard(noFilters(), [recent, edge, old], now);
    expect(col.cards.map((c) => c.title)).toEqual(["recent", "edge"]);
    expect(col.olderHidden).toBe(1);
    const [all] = buildBoard(noFilters(), [recent, edge, old], now, true);
    expect(all.cards.map((c) => c.title)).toEqual(["recent", "edge", "old"]);
    expect(all.olderHidden).toBe(0);
  });

  test("Finished with only older cards still shows, for Show older", () => {
    const cols = buildBoard(noFilters(), [card({ last_closed_at: now - 30 * D })], now);
    expect(cols).toHaveLength(1);
    expect(cols[0].cards).toEqual([]);
    expect(cols[0].olderHidden).toBe(1);
  });
});

describe("the sidebar's filters", () => {
  const cards = [
    card({ title: "mine", list_id: "p", priority: "high", next: next(now + H) }),
    card({ title: "shared", list_id: "s", priority: "low", next: next(now + H) }),
    card({ title: "quiet", list_id: "p", priority: "minimum", open: open() }),
  ];

  test("a hidden list or priority takes its cards off the Board", () => {
    let f = setListShown(noFilters(), "s", false);
    expect(buildBoard(f, cards, now).flatMap((c) => c.cards.map((x) => x.title)).sort()).toEqual([
      "mine",
      "quiet",
    ]);
    f = setPriorityShown(noFilters(), "minimum", false);
    const cols = buildBoard(f, cards, now);
    expect(cols.map((c) => c.id)).toEqual(["expected"]);
    expect(hiddenCards(f, cards)).toBe(1);
  });

  test("hiding everything leaves no columns", () => {
    let f = setListShown(noFilters(), "p", false);
    f = setListShown(f, "s", false);
    expect(buildBoard(f, cards, now)).toEqual([]);
  });
});

describe("the status line", () => {
  test("Expected says when it is next", () => {
    expect(statusLine(card({ next: next(at(0, 19)) }), now)).toBe("Next: today 19:00");
    expect(statusLine(card({ next: next(at(1, 7)) }), now)).toBe("Next: tomorrow 07:00");
  });

  test("Overdue and Due say when the occurrence was due", () => {
    const o = card({ open: open({ scheduled_at: at(-1, 21, 30), overdue_at: at(-1, 22) }) });
    expect(statusLine(o, now)).toBe("Overdue: was due yesterday 21:30");
    const d = card({ open: open({ scheduled_at: at(0, 11, 5), overdue_at: at(0, 13) }) });
    expect(statusLine(d, now)).toBe("Due today 11:05");
  });

  test("an open occurrence under a pause says so", () => {
    const c = card({
      open: open({ overdue_at: at(0, 13) }),
      pause: { until: null, list: true },
    });
    expect(statusLine(c, now)).toContain("paused with its list until resumed");
  });

  test("Paused says until when, and whether its list pauses it", () => {
    const own = card({ next: next(at(5, 7)), pause: { until: at(3, 0), list: false } });
    expect(statusLine(own, now)).toBe("Paused until 6 October");
    const list = card({ next: next(at(5, 7)), pause: { until: null, list: true } });
    expect(statusLine(list, now)).toBe("Paused with its list until resumed");
  });

  test("Finished says when it closed", () => {
    expect(statusLine(card({ last_closed_at: at(0, 10) }), now)).toBe("Finished today 10:00");
  });

  test("days a week or more away give the date", () => {
    expect(dayTime(at(10, 9), now)).toBe("13 Oct 09:00");
    expect(dayTime(at(3, 9), now)).toMatch(/^[A-Z][a-z]{2} 09:00$/);
    expect(dayTime(new Date(2027, 0, 2, 8, 5).getTime() / 1000, now)).toBe("2 Jan 2027 08:05");
  });
});

describe("pausing from a card", () => {
  test("offers Pause, Resume, or Resume list when the list pauses it", () => {
    expect(pauseOffer(card(), now)).toEqual({ kind: "pause" });
    expect(pauseOffer(card({ pause: { until: null, list: false } }), now)).toEqual({
      kind: "resume",
    });
    expect(pauseOffer(card({ list_id: "L", pause: { until: now + 5, list: true } }), now)).toEqual({
      kind: "resume-list",
      listId: "L",
    });
  });

  test("a pause that has run out offers Pause again", () => {
    expect(pauseOffer(card({ pause: { until: now - 1, list: false } }), now)).toEqual({
      kind: "pause",
    });
  });
});

test("the faking badge", () => {
  expect(fakingText("none")).toBe("can't be faked");
  expect(fakingText("easy")).toBe("easy to fake");
});

describe("the view switcher has the Board", () => {
  test("board is a view and is remembered", () => {
    expect(VIEWS.map((v) => v.id)).toContain("board");
    expect(isView("board")).toBe(true);
    expect(openingView("board")).toBe("board");
  });
});
