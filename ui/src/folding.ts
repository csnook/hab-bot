import type { DueItem, UndoOutcome } from "./api";

/**
 * Folding old quiet reminders (spec: Alerts → Keeping the lists tidy). The
 * core decides which overdue occurrences fold (Minimum and Low, overdue for
 * over a week, by the user's own priorities) so every client agrees, and
 * says so in `Inbox.folded`. The window applies the sidebar's filters to
 * the Overdue section first and then folds what is left, so the row counts
 * what the user can see, and Skip all… acts on exactly that.
 *
 * Folding is only how the list is drawn: the alerts, the tray's badge and
 * the Board never look at it.
 */

export interface OverdueSplit {
  /** Shown one by one, in the Overdue section's own order. */
  shown: DueItem[];
  /** In the older-quiet row at the end, in the same order. */
  folded: DueItem[];
}

/** Splits the (already filtered) Overdue section by the core's folded ids. */
export function splitOverdue(overdue: DueItem[], foldedIds: string[]): OverdueSplit {
  const folded = new Set(foldedIds);
  return {
    shown: overdue.filter((d) => !folded.has(d.occurrence_id)),
    folded: overdue.filter((d) => folded.has(d.occurrence_id)),
  };
}

/** The row's label: "4 older quiet reminders". */
export function foldLabel(count: number): string {
  return `${count} older quiet ${count === 1 ? "reminder" : "reminders"}`;
}

/** The row's disclosure mark. */
export function foldMark(expanded: boolean): string {
  return expanded ? "▾" : "▸";
}

/** What the Skip all… dialog says, including how many. */
export function skipAllQuestion(count: number): string {
  const what = count === 1 ? "this older quiet reminder" : `all ${count} older quiet reminders`;
  return (
    `Skip ${what}? Each is recorded as skipped, and you can undo them all right after ` +
    "or undo or correct any one later from its details. Medium priority and above are never skipped."
  );
}

/** The confirmation button. */
export function skipAllButton(count: number): string {
  return count === 1 ? "Skip it" : `Skip all ${count}`;
}

/** The line shown after skipping, beside "Undo all". */
export function skippedText(count: number): string {
  return `Skipped ${count} older quiet ${count === 1 ? "reminder" : "reminders"}.`;
}

/** What Undo all left, for the line that follows it. */
export function undoneText(results: Array<[string, UndoOutcome]>): string {
  const reopened = results.filter(([, o]) => o.outcome === "reopened").length;
  const missed = results.filter(([, o]) => o.outcome === "missed").length;
  const expected = results.length - reopened - missed;
  const parts = [`${reopened} reopened`];
  if (missed > 0) parts.push(`${missed} left missed, as they had expired`);
  if (expected > 0) parts.push(`${expected} expected again`);
  return `Undone: ${parts.join(", ")}.`;
}
