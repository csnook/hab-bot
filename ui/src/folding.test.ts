import { describe, expect, test } from "vitest";
import type { DueItem, Inbox } from "./api";
import { filterInbox, noFilters, setListShown, setPriorityShown } from "./filters";
import {
  foldLabel,
  foldMark,
  skipAllButton,
  skipAllQuestion,
  skippedText,
  splitOverdue,
  undoneText,
} from "./folding";

const item = (id: string, list = "p", priority: DueItem["priority"] = "low"): DueItem =>
  ({ occurrence_id: id, reminder_id: id, list_id: list, priority, title: id }) as DueItem;

const inbox = (overdue: DueItem[], folded: string[]): Inbox => ({
  overdue,
  due: [],
  later_today: [],
  earlier_today: [],
  paused: [],
  folded,
});

describe("splitting the Overdue section", () => {
  test("folded ones leave the list and keep its order", () => {
    const o = [item("a", "p", "high"), item("b"), item("c", "p", "medium"), item("d", "p", "minimum")];
    const s = splitOverdue(o, ["b", "d"]);
    expect(s.shown.map((d) => d.occurrence_id)).toEqual(["a", "c"]);
    expect(s.folded.map((d) => d.occurrence_id)).toEqual(["b", "d"]);
  });

  test("nothing folds when the core folded nothing, so medium and above stay put", () => {
    const o = [item("a", "p", "maximum"), item("b", "p", "medium")];
    const s = splitOverdue(o, []);
    expect(s.shown).toHaveLength(2);
    expect(s.folded).toEqual([]);
  });

  test("an id that is no longer overdue is ignored", () => {
    expect(splitOverdue([item("a")], ["gone"]).shown).toHaveLength(1);
  });
});

describe("the sidebar's filters and the row", () => {
  const all = inbox([item("a", "p", "high"), item("b", "p"), item("c", "work", "minimum")], ["b", "c"]);

  test("the row counts only what the filters leave", () => {
    const f = setListShown(noFilters(), "work", false);
    const s = splitOverdue(filterInbox(f, all).overdue, filterInbox(f, all).folded);
    expect(s.folded.map((d) => d.occurrence_id)).toEqual(["b"]);
  });

  test("hiding Low leaves nothing to fold, and Skip all would have nothing to act on", () => {
    const f = setPriorityShown(setPriorityShown(noFilters(), "low", false), "minimum", false);
    const v = filterInbox(f, all);
    expect(splitOverdue(v.overdue, v.folded).folded).toEqual([]);
  });

  test("unfiltered, everything the core folded is in the row", () => {
    const v = filterInbox(noFilters(), all);
    expect(splitOverdue(v.overdue, v.folded).folded).toHaveLength(2);
  });
});

describe("the words", () => {
  test("the label counts and pluralises", () => {
    expect(foldLabel(4)).toBe("4 older quiet reminders");
    expect(foldLabel(1)).toBe("1 older quiet reminder");
  });
  test("the disclosure mark", () => {
    expect(foldMark(false)).toBe("▸");
    expect(foldMark(true)).toBe("▾");
  });
  test("the dialog says how many and that higher priorities are left alone", () => {
    expect(skipAllQuestion(4)).toContain("all 4 older quiet reminders");
    expect(skipAllQuestion(4)).toContain("Medium priority and above are never skipped");
    expect(skipAllQuestion(1)).toContain("this older quiet reminder");
    expect(skipAllButton(4)).toBe("Skip all 4");
    expect(skipAllButton(1)).toBe("Skip it");
  });
  test("after skipping and after undoing", () => {
    expect(skippedText(3)).toBe("Skipped 3 older quiet reminders.");
    expect(skippedText(1)).toBe("Skipped 1 older quiet reminder.");
    expect(
      undoneText([
        ["a", { outcome: "reopened" }],
        ["b", { outcome: "reopened" }],
        ["c", { outcome: "missed", at: 5 }],
      ]),
    ).toBe("Undone: 2 reopened, 1 left missed, as they had expired.");
    expect(undoneText([["a", { outcome: "reopened" }]])).toBe("Undone: 1 reopened.");
  });
});
