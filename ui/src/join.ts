// The logic of the join screens, apart from the screens themselves.

export type Step = "server" | "confirm" | "profile" | "password" | "device";

export const STEPS: Step[] = ["server", "confirm", "profile", "password", "device"];

export interface JoinForm {
  address: string;
  setupCode: string;
  username: string;
  displayName: string;
  password: string;
  repeat: string;
  portable: boolean | null;
}

export const emptyForm: JoinForm = {
  address: "",
  setupCode: "",
  username: "",
  displayName: "",
  password: "",
  repeat: "",
  portable: null,
};

/** The same rule the server applies. */
export function validUsername(name: string): boolean {
  return /^[a-z0-9][a-z0-9._-]{0,31}$/.test(name);
}

export function validDisplayName(name: string): boolean {
  const t = name.trim();
  // eslint-disable-next-line no-control-regex
  return t.length > 0 && [...t].length <= 64 && !/[\u0000-\u001f\u007f]/.test(t);
}

/** Setup codes are shown in groups of four, and may be typed loosely. */
export function looksLikeSetupCode(code: string): boolean {
  return code.replace(/[\s-]/g, "").length >= 8;
}

/** Whether Continue is enabled. The password step also needs the strength check to pass. */
export function canContinue(step: Step, form: JoinForm, passwordOk: boolean): boolean {
  switch (step) {
    case "server":
      return form.address.trim() !== "" && looksLikeSetupCode(form.setupCode);
    case "confirm":
      return true;
    case "profile":
      return validUsername(form.username) && validDisplayName(form.displayName);
    case "password":
      return passwordOk && form.password === form.repeat;
    case "device":
      return form.portable !== null;
  }
}

export const METER_LABELS = ["Very weak", "Weak", "Fair", "Good", "Strong"];

export function meterLabel(score: number): string {
  return METER_LABELS[Math.max(0, Math.min(4, score))];
}

/** Shown while joining and again in Settings → Account. */
export const SERVER_SEES = [
  "It stores your account, your devices, who has access to what, and when each list changed.",
  "It can't read your reminders, history or settings: they are encrypted on your devices.",
  "It never sees your password.",
  "It doesn't log IP addresses unless its admin turns on debug logging.",
  "Anything in between, such as a VPN provider, Google's push service or your internet provider, can log when and from where you connect, whatever the server does.",
];

export const DATA_LOSS_WARNING =
  "If you lose every device and forget this password, your data is lost. There is no recovery without one of them.";
