import { expect, test } from "vitest";
import { alarmIdOf, dueLine, ringingHeading } from "./alarm";

test("the heading names the list and the priority", () => {
  expect(ringingHeading({ list_name: "Household", priority: "high" })).toBe(
    "Ringing · Household · High",
  );
  expect(ringingHeading({ list_name: null, priority: "maximum" })).toBe(
    "Ringing · Personal · Maximum",
  );
});

test("the due line says when it was due and how long ago", () => {
  const due = Math.floor(new Date(2026, 9, 3, 9, 30).getTime() / 1000);
  expect(dueLine(due, due + 300)).toContain("(5 minutes ago)");
  expect(dueLine(due, due)).toContain("(just now)");
});

test("the occurrence id comes from the window's query", () => {
  expect(alarmIdOf("?alarm=0f%2F1%4017%209")).toBe("0f/1@17 9");
  expect(alarmIdOf("")).toBeNull();
});
