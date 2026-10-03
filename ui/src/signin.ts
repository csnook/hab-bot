// The logic of the sign-in screens, apart from the screens themselves.

export type SignInStep = "server" | "confirm" | "account" | "device" | "approve";

/** How a device signs in without the password (approval from another device):
 * it scanned the other's code ("scan"), or shows a code for the other to scan
 * ("show"). null means with the password. */
export type Approving = "scan" | "show" | null;

export interface SignInForm {
  /** A sign-in code (its link), or a server address. */
  target: string;
  username: string;
  password: string;
  portable: boolean | null;
}

export const emptySignIn: SignInForm = {
  target: "",
  username: "",
  password: "",
  portable: null,
};

/** The step after `step`. A sign-in code already carries the fingerprint, so
 * there is nothing to confirm by eye. */
export function nextStep(
  step: SignInStep,
  fromCode: boolean,
  approving: Approving = null,
): SignInStep | null {
  switch (step) {
    case "server":
      // An existing device's approval code needs no username or password.
      if (approving === "scan") return "device";
      return fromCode ? "account" : "confirm";
    case "confirm":
      return "account";
    case "account":
      return "device";
    case "device":
      return approving ? "approve" : null;
    case "approve":
      return null;
  }
}

export function previousStep(
  step: SignInStep,
  fromCode: boolean,
  approving: Approving = null,
): SignInStep | null {
  switch (step) {
    case "server":
      return null;
    case "confirm":
      return "server";
    case "account":
      return fromCode ? "server" : "confirm";
    case "device":
      return approving === "scan" ? "server" : "account";
    case "approve":
      return "device";
  }
}

/** The same rule the server applies. */
function validUsername(name: string): boolean {
  return /^[a-z0-9][a-z0-9._-]{0,31}$/.test(name);
}

/** Whether Continue is enabled. Signing in takes no strength check: the
 * password is the one already set. */
export function canContinueSignIn(step: SignInStep, form: SignInForm): boolean {
  switch (step) {
    case "server":
      return form.target.trim() !== "";
    case "confirm":
      return true;
    case "account":
      return validUsername(form.username) && form.password !== "";
    case "device":
      return form.portable !== null;
    case "approve":
      // The step waits for the other device and moves on by itself.
      return false;
  }
}
