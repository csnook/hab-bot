// The logic of the sign-in screens, apart from the screens themselves.

export type SignInStep = "server" | "confirm" | "account" | "device";

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
export function nextStep(step: SignInStep, fromCode: boolean): SignInStep | null {
  switch (step) {
    case "server":
      return fromCode ? "account" : "confirm";
    case "confirm":
      return "account";
    case "account":
      return "device";
    case "device":
      return null;
  }
}

export function previousStep(step: SignInStep, fromCode: boolean): SignInStep | null {
  switch (step) {
    case "server":
      return null;
    case "confirm":
      return "server";
    case "account":
      return fromCode ? "server" : "confirm";
    case "device":
      return "account";
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
  }
}
