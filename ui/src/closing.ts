import type { ClosedEntry, ClosedView, ExpectedItem, UndoOutcome } from "./api";

/**
 * What the details panel offers (spec: Desktop → Occurrence details and
 * buttons). The buttons follow the occurrence's state: open, expected or
 * closed.
 */
export type PanelState = "open" | "expected" | "closed";

export interface Buttons {
  /** Always shown, in this order. */
  main: string[];
  /** In the More ▾ menu, if there is one. */
  more: string[];
}

/** `closedCanUndo` is false for a miss, which is corrected instead. */
export function buttonsFor(
  state: PanelState,
  options: { canCloseEarly?: boolean; canUndo?: boolean } = {},
): Buttons {
  switch (state) {
    case "open":
      return {
        main: ["Done", "Snooze ▾", "Skip", "More ▾"],
        more: ["Done at a different time", "Edit reminder"],
      };
    case "expected":
      // Complete early and Skip ahead close the next expected occurrence only.
      return {
        main: [
          ...(options.canCloseEarly ? ["Complete early", "Skip ahead"] : []),
          "Snooze ahead",
        ],
        more: ["Edit reminder"],
      };
    case "closed":
      return {
        main: [...(options.canUndo === false ? [] : ["Undo"]), "Correct"],
        more: [],
      };
  }
}

/** The expected occurrence offers early closing only if it is the next one. */
export const canCloseEarly = (e: Pick<ExpectedItem, "can_close_early">) => e.can_close_early;

/**
 * A time typed into a datetime-local box, as unix seconds, to be the time
 * something was done or skipped: empty is "now", and a time still to come
 * isn't allowed. It may be before the firing ("took it at 6:55").
 */
export function saidTime(
  text: string,
  now: number,
): { ok: true; value: number } | { ok: false; error: string } {
  if (!text) return { ok: true, value: now };
  const ms = new Date(text).getTime();
  if (Number.isNaN(ms)) return { ok: false, error: "That isn't a time." };
  const seconds = Math.floor(ms / 1000);
  if (seconds > now) return { ok: false, error: "That time hasn't come yet." };
  return { ok: true, value: seconds };
}

/** A note typed or picked: trimmed, and none if empty. */
export function cleanNote(text: string): string | undefined {
  const t = text.trim();
  return t ? t : undefined;
}

/** The recent notes worth offering: those that match what is being typed, each once. */
export function noteChoices(recent: string[], typed: string): string[] {
  const t = typed.trim().toLowerCase();
  const seen = new Set<string>();
  return recent.filter((n) => {
    if (seen.has(n) || n.toLowerCase() === t) return false;
    seen.add(n);
    return t === "" || n.toLowerCase().includes(t);
  });
}

/** What the window says after an undo, which can leave the occurrence in three ways. */
export function undoMessage(outcome: UndoOutcome): string {
  switch (outcome.outcome) {
    case "reopened":
      return "Reopened.";
    case "expected":
      return "Expected again. It will fire at its time.";
    case "missed":
      return "It would no longer be open, so it counts as missed.";
  }
}

/** One line of a closed occurrence's history. */
export function historyLine(
  e: ClosedEntry,
  format: (unixSeconds: number) => string,
): string {
  const what: Record<ClosedEntry["what"], string> = {
    completed: "Completed",
    skipped: "Skipped",
    missed: "Missed",
    reopened: "Undone: reopened",
    expected: "Undone: expected again",
  };
  const parts = [what[e.what]];
  if (e.at !== null) parts[0] += ` at ${format(e.at)}`;
  if (e.note) parts.push(`“${e.note}”`);
  // The time said and the time tapped, once they differ, and when the server had it.
  if (e.at !== null && Math.abs(e.tapped_at - e.at) >= 60) parts.push(`tapped ${format(e.tapped_at)}`);
  if (e.received_at !== null) parts.push(`received ${format(e.received_at)}`);
  if (e.superseded) parts.push("replaced");
  return parts.join(", ");
}

/** "Done late" and the like, for the panel's heading. */
export function outcomeLabel(outcome: ClosedView["outcome"]): string {
  return {
    done_on_time: "Done on time",
    done_late: "Done late",
    skipped: "Skipped",
    missed: "Missed",
  }[outcome];
}
