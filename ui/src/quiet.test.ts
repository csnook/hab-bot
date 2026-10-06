import { expect, test } from "vitest";
import type { QuietHours, SnoozeAllView } from "./api";
import {
  checkQuietHours,
  chipText,
  chips,
  describeDays,
  describeQuietHours,
  lengthEnd,
  newQuietHours,
  quietLine,
  quietNow,
  scopeOfValue,
  toggleDay,
  untilLabel,
  valueOfScope,
} from "./quiet";

const at = (h: number, m: number, day = 3) => Math.floor(new Date(2026, 9, day, h, m).getTime() / 1000);

const view = (over: Partial<SnoozeAllView> = {}): SnoozeAllView => ({
  id: "s1",
  source: "snooze_all",
  scope: { kind: "all" },
  list_name: null,
  include_maximum: false,
  from: at(17, 0),
  until: at(19, 0),
  ...over,
});

test("a length ends that far on, at a time of day, or tomorrow at 08:00", () => {
  expect(lengthEnd({ kind: "minutes", minutes: 90 }, at(9, 0))).toBe(at(10, 30));
  expect(lengthEnd({ kind: "minutes", minutes: 0 }, at(9, 0))).toBeNull();
  expect(lengthEnd({ kind: "time", time: "19:00" }, at(9, 0))).toBe(at(19, 0));
  expect(lengthEnd({ kind: "time", time: "07:00" }, at(9, 0))).toBe(at(7, 0, 4));
  expect(lengthEnd({ kind: "time", time: "soon" }, at(9, 0))).toBeNull();
  expect(lengthEnd({ kind: "tomorrow_morning" }, at(23, 30))).toBe(at(8, 0, 4));
  expect(lengthEnd({ kind: "tomorrow_morning" }, at(1, 0))).toBe(at(8, 0, 4));
});

test("the chip reads 'All snoozed until 19:00', or names the list, and says if Maximum is in", () => {
  const now = at(17, 30);
  expect(chipText(view(), now)).toBe("All snoozed until 19:00");
  expect(
    chipText(view({ scope: { kind: "list", list_id: "l" }, list_name: "Household" }), now),
  ).toBe("Household snoozed until 19:00");
  expect(chipText(view({ include_maximum: true }), now)).toBe(
    "All snoozed until 19:00, Maximum included",
  );
  // Another day says which.
  expect(untilLabel(at(7, 0, 4), now)).toMatch(/07:00$/);
  expect(untilLabel(at(7, 0, 4), now).length).toBeGreaterThan("07:00".length);
});

test("only snooze-alls that haven't ended are chips, and quiet hours aren't", () => {
  const now = at(18, 0);
  const quiet = view({ id: null, source: "quiet_hours" });
  const over = view({ id: "s2", until: at(17, 59) });
  expect(chips([view(), quiet, over], now).map((v) => v.id)).toEqual(["s1"]);
  expect(quietNow([view(), quiet], now)).toBe(quiet);
  expect(quietNow([view()], now)).toBeNull();
});

test("days are described the way people say them", () => {
  expect(describeDays(["MO", "TU", "WE", "TH", "FR"])).toBe("weeknights");
  expect(describeDays(["SU", "SA"])).toBe("weekend nights");
  expect(describeDays(["MO", "TU", "WE", "TH", "FR", "SA", "SU"])).toBe("every night");
  expect(describeDays(["FR", "MO", "WE"])).toBe("Mon, Wed and Fri nights");
  expect(describeDays(["TU"])).toBe("Tue nights");
  expect(describeDays([])).toBe("no nights");
});

test("a quiet hours rule is described, with its list and Maximum", () => {
  const q = newQuietHours();
  expect(describeQuietHours(q)).toBe(
    "22:00 to 07:00 on weeknights, for all reminders, Maximum left out",
  );
  expect(
    describeQuietHours({ ...q, scope: { kind: "list", list_id: "l" }, include_maximum: true }, "Household"),
  ).toBe("22:00 to 07:00 on weeknights, for Household, Maximum included");
});

test("toggling a day keeps the week's order", () => {
  expect(toggleDay(["MO", "FR"], "WE", true)).toEqual(["MO", "WE", "FR"]);
  expect(toggleDay(["MO", "FR"], "MO", false)).toEqual(["FR"]);
  expect(toggleDay([], "SU", true)).toEqual(["SU"]);
});

test("a rule is refused for the same reasons the core refuses it", () => {
  const ok: QuietHours = newQuietHours();
  expect(checkQuietHours(ok)).toBeNull();
  expect(checkQuietHours({ ...ok, days: [] })).toMatch(/at least one day/);
  expect(checkQuietHours({ ...ok, to: "22:00" })).toMatch(/same time/);
  expect(checkQuietHours({ ...ok, from: "25:00" })).toMatch(/times such as/);
  expect(checkQuietHours({ ...ok, from: "" })).toMatch(/times such as/);
  // A rule that crosses midnight, or doesn't, is fine.
  expect(checkQuietHours({ ...ok, from: "13:00", to: "15:00" })).toBeNull();
});

test("a scope round-trips through a select's value", () => {
  expect(scopeOfValue("")).toEqual({ kind: "all" });
  expect(scopeOfValue("abc")).toEqual({ kind: "list", list_id: "abc" });
  expect(valueOfScope({ kind: "list", list_id: "abc" })).toBe("abc");
  expect(valueOfScope({ kind: "all" })).toBe("");
});

test("the sidebar footer says what quiet hours are doing", () => {
  const now = at(23, 0);
  const quiet = view({ id: null, source: "quiet_hours", until: at(7, 0, 4) });
  expect(quietLine([newQuietHours()], [quiet], now)).toMatch(/^Quiet hours until .*07:00$/);
  expect(quietLine([newQuietHours()], [], at(12, 0))).toBe("Quiet hours 22:00–07:00");
  expect(quietLine([newQuietHours(), newQuietHours()], [], at(12, 0))).toBe(
    "Quiet hours 22:00–07:00 and 1 more",
  );
  expect(quietLine([], [], at(12, 0))).toBe("No quiet hours");
});
