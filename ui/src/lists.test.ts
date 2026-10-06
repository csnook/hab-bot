import { describe, expect, test } from "vitest";
import type { ListInfo } from "./api";
import {
  PALETTE,
  PERSONAL_NAME,
  canDeleteList,
  checkListName,
  isColour,
  listColour,
  listName,
  nextColour,
  whyNotDeletable,
} from "./lists";
import { DELETE_CHOICES, deleteQuestion, withHistory } from "./deleting";

const list = (id: string, over: Partial<ListInfo> = {}): ListInfo => ({
  id,
  name: id,
  colour: null,
  personal: false,
  reminders: 0,
  pause: null,
  ...over,
});
const personal = list("p", { name: null, personal: true });

describe("lists", () => {
  test("the personal list is named Personal and can't be deleted", () => {
    expect(listName(personal)).toBe(PERSONAL_NAME);
    expect(canDeleteList(personal)).toBe(false);
    expect(whyNotDeletable(personal)).toMatch(/personal list/);
  });

  test("only an empty list can be deleted", () => {
    expect(canDeleteList(list("a"))).toBe(true);
    expect(canDeleteList(list("a", { reminders: 2 }))).toBe(false);
    expect(whyNotDeletable(list("a", { reminders: 2 }))).toMatch(/Move or delete/);
    expect(whyNotDeletable(list("a"))).toBeNull();
  });

  test("a list without a colour takes one from the palette by position", () => {
    const all = [personal, list("a"), list("b", { colour: "#123456" })];
    expect(listColour(all[0], all)).toBe(PALETTE[0]);
    expect(listColour(all[1], all)).toBe(PALETTE[1]);
    expect(listColour(all[2], all)).toBe("#123456");
  });

  test("names are trimmed and can't be empty", () => {
    expect(checkListName("  Home ")).toEqual({ ok: true, value: "Home" });
    expect(checkListName("   ").ok).toBe(false);
    expect(checkListName("x".repeat(81)).ok).toBe(false);
  });

  test("a new list starts with a colour not yet in use", () => {
    expect(nextColour([personal])).toBe(PALETTE[0]);
    expect(nextColour([personal, list("a", { colour: PALETTE[0] })])).toBe(PALETTE[1]);
  });

  test("colours are #rrggbb", () => {
    expect(isColour("#a1B2c3")).toBe(true);
    expect(isColour("a1b2c3")).toBe(false);
    expect(isColour("#abc")).toBe(false);
  });
});

describe("deleting a reminder", () => {
  test("asks whether to keep the history or delete it too", () => {
    expect(DELETE_CHOICES.map((c) => c.value)).toEqual(["keep", "purge"]);
    expect(DELETE_CHOICES[0].label).toMatch(/Keep its history, marked deleted/);
    expect(DELETE_CHOICES[1].label).toMatch(/with its history/);
    expect(DELETE_CHOICES[1].detail).toMatch(/can't be recalled/);
    expect(withHistory("keep")).toBe(false);
    expect(withHistory("purge")).toBe(true);
    expect(deleteQuestion("Bins")).toBe("Delete “Bins”?");
  });
});
