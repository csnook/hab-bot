/**
 * The view switcher: which views the window has so far, and which one it
 * opens on. The core remembers the last one on this device; anything it
 * remembers that this window can't show yet (the Calendar and History
 * come with later tickets) opens as the Inbox.
 */

export type View = "inbox" | "agenda" | "board";

export const VIEWS: Array<{ id: View; label: string }> = [
  { id: "inbox", label: "Inbox" },
  { id: "agenda", label: "Agenda" },
  { id: "board", label: "Board" },
];

export function isView(v: string): v is View {
  return VIEWS.some((x) => x.id === v);
}

/** The view to open on, from what the core remembered. */
export function openingView(remembered: string | null | undefined): View {
  return remembered && isView(remembered) ? remembered : "inbox";
}

export function viewTitle(v: View): string {
  return VIEWS.find((x) => x.id === v)?.label ?? "Inbox";
}

/** The first-run tip's text: the views, and that the last one is remembered. */
export function tipText(): string {
  return (
    "Inbox lists what needs you now and what is coming today. " +
    "Agenda lays out yesterday, today and tomorrow in time order. " +
    "Board shows every reminder as a card, in columns by its state. " +
    "The window opens on the view you used last."
  );
}
