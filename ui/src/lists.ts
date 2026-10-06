import type { ListInfo } from "./api";

/** What the personal list is called, as it has no name of its own. */
export const PERSONAL_NAME = "Personal";

/** Colours offered for lists, and the ones lists without a colour of their own use. */
export const PALETTE = [
  "#3584e4",
  "#33a06f",
  "#e5a50a",
  "#e66100",
  "#d6405e",
  "#9141ac",
  "#865e3c",
  "#5e6a75",
];

export const listName = (l: Pick<ListInfo, "name">): string => l.name ?? PERSONAL_NAME;

/** A list's colour: its own, or one from the palette chosen by where it stands in the list. */
export function listColour(l: ListInfo, all: ListInfo[]): string {
  if (l.colour) return l.colour;
  const i = Math.max(0, all.findIndex((x) => x.id === l.id));
  return PALETTE[i % PALETTE.length];
}

/** Looks a list up by id, for the rows that carry only `list_id`. */
export function listById(all: ListInfo[], id: string): ListInfo | undefined {
  return all.find((l) => l.id === id);
}

/** Whether the list can be deleted: only an empty one that isn't the personal list. */
export function canDeleteList(l: ListInfo): boolean {
  return !l.personal && l.reminders === 0;
}

/** Why a list can't be deleted, for the button's tooltip; null when it can. */
export function whyNotDeletable(l: ListInfo): string | null {
  if (l.personal) return "The personal list can't be deleted.";
  if (l.reminders > 0) return "Move or delete its reminders first.";
  return null;
}

/** A new list's name, trimmed, or an error to show. */
export function checkListName(name: string): { ok: true; value: string } | { ok: false; error: string } {
  const value = name.trim();
  if (!value) return { ok: false, error: "Give the list a name." };
  if (value.length > 80) return { ok: false, error: "That name is too long (80 characters at most)." };
  return { ok: true, value };
}

/** The colour the next new list starts with: the first palette colour not in use. */
export function nextColour(all: ListInfo[]): string {
  const used = new Set(all.map((l) => l.colour?.toLowerCase()));
  return PALETTE.find((c) => !used.has(c)) ?? PALETTE[all.length % PALETTE.length];
}

/** Whether text is a colour the core accepts. */
export const isColour = (c: string): boolean => /^#[0-9a-fA-F]{6}$/.test(c);
