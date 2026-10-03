import { expect, test } from "vitest";
import { canContinue, emptyForm, looksLikeSetupCode, meterLabel, validUsername } from "./join";

const form = { ...emptyForm };

test("the server step needs an address and a plausible setup code", () => {
  expect(canContinue("server", form, false)).toBe(false);
  expect(canContinue("server", { ...form, address: "home.lan", setupCode: "AB" }, false)).toBe(false);
  expect(
    canContinue("server", { ...form, address: "home.lan", setupCode: "0123-4567-89AB" }, false),
  ).toBe(true);
  expect(looksLikeSetupCode("0123 4567")).toBe(true);
});

test("usernames follow the server's rule", () => {
  expect(validUsername("chris")).toBe(true);
  expect(validUsername("a.b_c-9")).toBe(true);
  expect(validUsername("Chris")).toBe(false);
  expect(validUsername("")).toBe(false);
  expect(validUsername("-a")).toBe(false);
  expect(validUsername("a".repeat(33))).toBe(false);
});

test("the profile step needs both names", () => {
  expect(canContinue("profile", { ...form, username: "chris", displayName: "  " }, false)).toBe(false);
  expect(canContinue("profile", { ...form, username: "chris", displayName: "Chris" }, false)).toBe(true);
});

test("Continue stays disabled until the password is strong and repeated", () => {
  const pw = { ...form, password: "correct horse", repeat: "correct horse" };
  expect(canContinue("password", pw, false)).toBe(false);
  expect(canContinue("password", pw, true)).toBe(true);
  expect(canContinue("password", { ...pw, repeat: "correct hors" }, true)).toBe(false);
});

test("the device step needs an answer, either one", () => {
  expect(canContinue("device", form, false)).toBe(false);
  expect(canContinue("device", { ...form, portable: false }, false)).toBe(true);
  expect(canContinue("device", { ...form, portable: true }, false)).toBe(true);
});

test("the meter has a label for every score", () => {
  expect([0, 1, 2, 3, 4].map(meterLabel)).toEqual(["Very weak", "Weak", "Fair", "Good", "Strong"]);
  expect(meterLabel(9)).toBe("Strong");
});
