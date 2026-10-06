import type {
  EarlierItem,
  ExpectedItem,
  Filters,
  Inbox,
  ListInfo,
  PriorityName,
  Snapshot,
} from "./api";

/**
 * The sidebar's list and priority checkboxes. They filter what the window
 * lists, in every view, and nothing else: the alerts are decided in the core,
 * which never looks at them, so hiding a list doesn't silence it.
 *
 * What is hidden is stored, not what is shown, so a list or priority made
 * later shows by default.
 */

export const PRIORITIES: PriorityName[] = ["maximum", "high", "medium", "low", "minimum"];

export const noFilters = (): Filters => ({ hidden_lists: [], hidden_priorities: [] });

/** Anything the views list: it is in a list and has a priority. */
export interface Filterable {
  list_id: string;
  priority: PriorityName;
}

export function showsList(f: Filters, listId: string): boolean {
  return !f.hidden_lists.includes(listId);
}

export function showsPriority(f: Filters, p: PriorityName): boolean {
  return !f.hidden_priorities.includes(p);
}

export function shows(f: Filters, item: Filterable): boolean {
  return showsList(f, item.list_id) && showsPriority(f, item.priority);
}

export function visible<T extends Filterable>(f: Filters, items: T[]): T[] {
  return items.filter((i) => shows(f, i));
}

function toggled<T>(hidden: T[], value: T, show: boolean): T[] {
  const without = hidden.filter((h) => h !== value);
  return show ? without : [...without, value];
}

/** Ticks or unticks a list's checkbox. */
export function setListShown(f: Filters, listId: string, show: boolean): Filters {
  return { ...f, hidden_lists: toggled(f.hidden_lists, listId, show) };
}

/** Ticks or unticks a priority's checkbox. */
export function setPriorityShown(f: Filters, p: PriorityName, show: boolean): Filters {
  return { ...f, hidden_priorities: toggled(f.hidden_priorities, p, show) };
}

/** Forgets lists that no longer exist, so the stored filter doesn't grow. */
export function pruned(f: Filters, lists: ListInfo[]): Filters {
  const known = new Set(lists.map((l) => l.id));
  const hidden = f.hidden_lists.filter((id) => known.has(id));
  return hidden.length === f.hidden_lists.length ? f : { ...f, hidden_lists: hidden };
}

/** Whether a filter hides anything at all. */
export function isFiltering(f: Filters): boolean {
  return f.hidden_lists.length > 0 || f.hidden_priorities.length > 0;
}

/** The Inbox's sections with the filters applied. */
export function filterInbox(f: Filters, inbox: Inbox): Inbox {
  return {
    overdue: visible(f, inbox.overdue),
    due: visible(f, inbox.due),
    later_today: visible<ExpectedItem>(f, inbox.later_today),
    earlier_today: visible<EarlierItem>(f, inbox.earlier_today),
    paused: inbox.paused.filter((p) => shows(f, p.item)),
  };
}

/** The snapshot's lists of reminders with the filters applied. Notices are never filtered. */
export function filterSnapshot(f: Filters, snap: Snapshot): Snapshot {
  return {
    ...snap,
    due: visible(f, snap.due),
    upcoming: visible(f, snap.upcoming),
    countdowns: visible(f, snap.countdowns),
  };
}

/** How many open occurrences the filters hide, for "3 hidden by filters". */
export function hiddenCount(f: Filters, inbox: Inbox): number {
  const all = inbox.overdue.length + inbox.due.length;
  const shown = visible(f, inbox.overdue).length + visible(f, inbox.due).length;
  return all - shown;
}
