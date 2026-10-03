import { expect, test } from "vitest";
import { canContinueSignIn, emptySignIn, nextStep, previousStep } from "./signin";
import { ago, signInNoticeText } from "./time";

test("a sign-in code skips the fingerprint check, an address does not", () => {
  expect(nextStep("server", true)).toBe("account");
  expect(nextStep("server", false)).toBe("confirm");
  expect(nextStep("confirm", false)).toBe("account");
  expect(nextStep("account", false)).toBe("device");
  expect(nextStep("device", false)).toBeNull();
  expect(previousStep("account", true)).toBe("server");
  expect(previousStep("account", false)).toBe("confirm");
  expect(previousStep("server", false)).toBeNull();
});

test("approving from another device skips the username and password", () => {
  // The existing device's code was scanned: straight to naming the kind of device.
  expect(nextStep("server", true, "scan")).toBe("device");
  expect(nextStep("device", true, "scan")).toBe("approve");
  expect(previousStep("device", true, "scan")).toBe("server");
  expect(previousStep("approve", true, "scan")).toBe("device");
  // Or the user chose approval at the password step, and shows a code instead.
  expect(nextStep("account", true, "show")).toBe("device");
  expect(nextStep("device", true, "show")).toBe("approve");
  expect(previousStep("device", true, "show")).toBe("account");
  expect(nextStep("approve", true, "show")).toBeNull();
  expect(canContinueSignIn("approve", { ...emptySignIn, portable: true })).toBe(false);
});

test("each step needs what it asks for", () => {
  const form = { ...emptySignIn };
  expect(canContinueSignIn("server", form)).toBe(false);
  expect(canContinueSignIn("server", { ...form, target: " home.lan " })).toBe(true);
  expect(canContinueSignIn("account", { ...form, username: "chris" })).toBe(false);
  expect(canContinueSignIn("account", { ...form, username: "Chris", password: "x" })).toBe(false);
  expect(canContinueSignIn("account", { ...form, username: "chris", password: "x" })).toBe(true);
  expect(canContinueSignIn("device", form)).toBe(false);
  expect(canContinueSignIn("device", { ...form, portable: false })).toBe(true);
});

test("a sign-in is announced as just now, then as it ages", () => {
  expect(ago(1000, 1000)).toBe("just now");
  expect(ago(1000, 1059)).toBe("just now");
  expect(ago(1000, 1060)).toBe("1 minute ago");
  expect(ago(1000, 1000 + 5 * 60)).toBe("5 minutes ago");
  expect(ago(1000, 1000 + 2 * 3600)).toBe("2 hours ago");
  expect(ago(1000, 1000 + 3 * 86400)).toBe("3 days ago");
  expect(ago(2000, 1000)).toBe("just now");
  expect(signInNoticeText("Pixel 9", 1000, 1000)).toBe("New device signed in: Pixel 9, just now.");
});
