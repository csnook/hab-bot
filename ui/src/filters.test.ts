import { describe, expect, test } from "vitest";
import type { DueItem, Filters, Inbox, ListInfo, Snapshot } from "./api";
import {
  filterInbox,
  filterSnapshot,
  hiddenCount,
  isFiltering,
  noFilters,
  pruned,
  setListShown,
  setPriorityShown,
  shows,
  visible,
} from "./filters";

const due = (id: string, list: string, priority: DueItem["priority"]): DueItem =>
  ({ occurrence_id: id, reminder_id: id, list_id: list, priority, title: id }) as DueItem;

const inbox = (): Inbox => ({
  overdue: [due("a", "home", "high"), due("b", "work", "low")],
  due: [due("c", "home", "minimum"), due("d", "p", "medium")],
  later_today: [
    { reminder_id: "e", list_id: "work", priority: "medium" } as Inbox["later_today"][number],
  ],
  earlier_today: [
    { occurrence_id: "f", list_id: "home", priority: "low" } as Inbox["earlier_today"][number],
  ],
  paused: [
    { item: due("g", "home", "high"), pause: { until: null, list: false } },
    { item: due("h", "work", "low"), pause: { until: 9, list: true } },
  ],
});

describe("the sidebar's checkboxes", () => {
  test("nothing is hidden at first, and a new list or priority shows", () => {
    const f = noFilters();
    expect(isFiltering(f)).toBe(false);
    expect(shows(f, { list_id: "brand-new", priority: "maximum" })).toBe(true);
  });

  test("unticking a list hides its reminders in every view", () => {
    const f = setListShown(noFilters(), "home", false);
    const i = filterInbox(f, inbox());
    expect(i.overdue.map((d) => d.occurrence_id)).toEqual(["b"]);
    expect(i.due.map((d) => d.occurrence_id)).toEqual(["d"]);
    expect(i.later_today.map((e) => e.reminder_id)).toEqual(["e"]);
    expect(i.earlier_today).toEqual([]);
    // The paused ones are filtered like the rest, and stay out of the count of hidden open ones.
    expect(i.paused.map((p) => p.item.occurrence_id)).toEqual(["h"]);
    const snap = filterSnapshot(f, {
      due: [due("a", "home", "high"), due("b", "work", "low")],
      upcoming: [
        { reminder_id: "u1", list_id: "home", priority: "low" },
        { reminder_id: "u2", list_id: "p", priority: "low" },
      ],
      countdowns: [{ reminder_id: "k", list_id: "home", priority: "low" }],
      sign_in_notices: [{ id: "n" }],
    } as unknown as Snapshot);
    expect(snap.due.map((d) => d.reminder_id)).toEqual(["b"]);
    expect(snap.upcoming.map((u) => u.reminder_id)).toEqual(["u2"]);
    expect(snap.countdowns).toEqual([]);
    // Notices are about the account, not a list.
    expect(snap.sign_in_notices).toHaveLength(1);
  });

  test("unticking a priority hides it, and the two filters combine", () => {
    let f = setPriorityShown(noFilters(), "minimum", false);
    f = setPriorityShown(f, "low", false);
    expect(visible(f, inbox().due).map((d) => d.occurrence_id)).toEqual(["d"]);
    f = setListShown(f, "p", false);
    expect(filterInbox(f, inbox()).due).toEqual([]);
    expect(hiddenCount(f, inbox())).toBe(3);
  });

  test("ticking a box again shows it again, once", () => {
    let f = setListShown(noFilters(), "home", false);
    f = setListShown(f, "home", false);
    expect(f.hidden_lists).toEqual(["home"]);
    f = setListShown(f, "home", true);
    expect(f).toEqual(noFilters());
  });

  test("the stored filter forgets lists that are gone", () => {
    const f: Filters = { hidden_lists: ["home", "gone"], hidden_priorities: ["low"] };
    const lists = [{ id: "home" }, { id: "p" }] as ListInfo[];
    expect(pruned(f, lists)).toEqual({ hidden_lists: ["home"], hidden_priorities: ["low"] });
    expect(pruned(noFilters(), lists)).toEqual(noFilters());
  });
});
