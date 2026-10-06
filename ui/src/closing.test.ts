import { expect, test } from "vitest";
import type { ClosedEntry } from "./api";
import {
  buttonsFor,
  cleanNote,
  historyLine,
  noteChoices,
  outcomeLabel,
  saidTime,
  undoMessage,
} from "./closing";

const now = Math.floor(new Date("2026-10-03T12:00:00").getTime() / 1000);

test("an open occurrence has Done, Snooze, Skip and a More menu", () => {
  expect(buttonsFor("open")).toEqual({
    main: ["Done", "Snooze ▾", "Skip", "More ▾"],
    more: ["Done at a different time", "Edit reminder"],
  });
});

test("an expected occurrence offers early closing only when it can be closed early", () => {
  expect(buttonsFor("expected", { canCloseEarly: true }).main).toEqual([
    "Complete early",
    "Skip ahead",
    "Snooze ahead",
  ]);
  expect(buttonsFor("expected", { canCloseEarly: false }).main).toEqual(["Snooze ahead"]);
});

test("a closed occurrence has Undo and Correct, and a miss only Correct", () => {
  expect(buttonsFor("closed", { canUndo: true }).main).toEqual(["Undo", "Correct"]);
  expect(buttonsFor("closed", { canUndo: false }).main).toEqual(["Correct"]);
});

test("the time said defaults to now, may be before the firing, and never to come", () => {
  expect(saidTime("", now)).toEqual({ ok: true, value: now });
  expect(saidTime("2026-10-03T06:55", now)).toEqual({
    ok: true,
    value: Math.floor(new Date("2026-10-03T06:55").getTime() / 1000),
  });
  expect(saidTime("2026-10-03T12:01", now).ok).toBe(false);
  expect(saidTime("garbage", now).ok).toBe(false);
});

test("notes are trimmed and recent ones are offered as you type", () => {
  expect(cleanNote("  away ")).toBe("away");
  expect(cleanNote("   ")).toBeUndefined();
  const recent = ["away", "travelling", "away", "ill"];
  expect(noteChoices(recent, "")).toEqual(["away", "travelling", "ill"]);
  expect(noteChoices(recent, "tra")).toEqual(["travelling"]);
  // What is already typed in full needn't be offered again.
  expect(noteChoices(recent, "Away")).toEqual([]);
});

test("what an undo left is said plainly", () => {
  expect(undoMessage({ outcome: "reopened" })).toBe("Reopened.");
  expect(undoMessage({ outcome: "expected" })).toMatch(/Expected again/);
  expect(undoMessage({ outcome: "missed", at: 5 })).toMatch(/missed/);
});

test("history lines show what was said, tapped and received, and what was replaced", () => {
  const f = (t: number) => `t${t}`;
  const e: ClosedEntry = {
    event_id: "e",
    what: "completed",
    at: 1000,
    note: null,
    by: "u",
    tapped_at: 4600,
    received_at: 4601,
    superseded: false,
  };
  expect(historyLine(e, f)).toBe("Completed at t1000, tapped t4600, received t4601");
  expect(historyLine({ ...e, what: "skipped", note: "away", tapped_at: 1010, received_at: null, superseded: true }, f)).toBe(
    "Skipped at t1000, “away”, replaced",
  );
  expect(historyLine({ ...e, what: "reopened", at: null, received_at: null }, f)).toBe("Undone: reopened");
});

test("a miss that was corrected is labelled done late", () => {
  expect(outcomeLabel("done_late")).toBe("Done late");
  expect(outcomeLabel("missed")).toBe("Missed");
});
