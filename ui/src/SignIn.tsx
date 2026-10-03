import { useEffect, useState } from "preact/hooks";
import {
  approvalScan,
  approvalShow,
  cancelApproval,
  finishApproval,
  readSignIn,
  signInServer,
  type ApprovalCode,
  type Profile,
  type SignInFound,
} from "./api";
import {
  canContinueSignIn,
  type Approving,
  emptySignIn,
  nextStep,
  previousStep,
  type SignInForm,
  type SignInStep,
} from "./signin";
import { ServerSees } from "./ServerSees";

/** Sign in on this device with an account that already exists. */
export function SignIn({
  onCancel,
  onSignedIn,
}: {
  onCancel: () => void;
  onSignedIn: (p: Profile) => void;
}) {
  const [step, setStep] = useState<SignInStep>("server");
  const [form, setForm] = useState<SignInForm>(emptySignIn);
  const [found, setFound] = useState<SignInFound | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [approving, setApproving] = useState<Approving>(null);

  const set = (patch: Partial<SignInForm>) => setForm((f) => ({ ...f, ...patch }));
  const fromCode = found?.fromCode === true;
  const enabled = canContinueSignIn(step, form) && !busy;

  const back = () => {
    setError("");
    const prev = previousStep(step, fromCode, approving);
    if (step === "approve") void cancelApproval();
    if (prev === null) onCancel();
    else setStep(prev);
    if (prev === "server" || prev === "account") setApproving(null);
  };

  const next = async () => {
    setError("");
    if (step === "server") {
      setBusy(true);
      try {
        const f = await readSignIn(form.target);
        setFound(f);
        const scanning = f.approvalLink !== null ? "scan" : null;
        setApproving(scanning);
        setStep(nextStep("server", f.fromCode, scanning)!);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    } else if (step === "device" && approving) {
      setStep("approve");
    } else if (step === "device") {
      if (!found) return;
      setBusy(true);
      try {
        const profile = await signInServer({
          address: found.address,
          fingerprint: found.fingerprint,
          serverName: found.name,
          username: form.username,
          password: form.password,
          portable: form.portable === true,
        });
        // The password is no longer needed.
        set({ password: "" });
        onSignedIn(profile);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    } else {
      setStep(nextStep(step, fromCode, approving)!);
    }
  };

  const approveInstead = () => {
    setError("");
    setApproving("show");
    setStep("device");
  };

  return (
    <main>
      <h1>Sign in</h1>
      {error && <p class="error" role="alert">{error}</p>}

      {step === "server" && (
        <section>
          <label>
            Sign-in code or server address
            <input
              value={form.target}
              placeholder="hab-bot://sign-in?… or home.lan:8443"
              autocomplete="off"
              spellcheck={false}
              onInput={(e) => set({ target: e.currentTarget.value })}
            />
          </label>
          <p class="muted">
            Another device that is signed in shows a sign-in code in Settings → Account. It carries
            the server's name and certificate fingerprint, so you have nothing to compare.
          </p>
        </section>
      )}

      {step === "confirm" && found && (
        <section>
          <p>Is this your server?</p>
          <Server found={found} />
          <p class="muted">
            Compare the fingerprint with the one in Settings → Account on a signed-in device. This
            device will only talk to a server with this certificate.
          </p>
          <ServerSees />
        </section>
      )}

      {step === "account" && found && (
        <section>
          <Server found={found} />
          {fromCode && (
            <p class="muted">The certificate matches the sign-in code and is now pinned.</p>
          )}
          <label>
            Username
            <input
              value={form.username}
              autocomplete="username"
              onInput={(e) => set({ username: e.currentTarget.value })}
            />
          </label>
          <label>
            Password
            <input
              type="password"
              value={form.password}
              autocomplete="current-password"
              onInput={(e) => set({ password: e.currentTarget.value })}
            />
          </label>
          <p class="muted">
            Or skip the password: another device of yours that is signed in can approve this one.
          </p>
          <button type="button" onClick={approveInstead}>Approve from another device</button>
        </section>
      )}

      {step === "device" && (
        <section>
          <p>Does this device go where you go?</p>
          <label class="radio">
            <input
              type="radio"
              name="portable"
              checked={form.portable === true}
              onChange={() => set({ portable: true })}
            />
            Portable, like a laptop. Its Wi-Fi and Bluetooth connections say where you are.
          </label>
          <label class="radio">
            <input
              type="radio"
              name="portable"
              checked={form.portable === false}
              onChange={() => set({ portable: false })}
            />
            Stationary, like a desktop. Its connections are ignored by default.
          </label>
          <p class="muted">
            Your other devices will be told that this one signed in, and can remove it if it wasn't
            you.
          </p>
        </section>
      )}

      {step === "approve" && found && approving && (
        <WaitForApproval
          found={found}
          mode={approving}
          portable={form.portable === true}
          onSignedIn={onSignedIn}
          onError={(e) => {
            setError(e);
            setStep("device");
          }}
        />
      )}

      <div class="buttons">
        <button type="button" onClick={back} disabled={busy}>Back</button>
        {step !== "approve" && (
          <button type="button" onClick={next} disabled={!enabled}>
            {busy
              ? "Working…"
              : step === "device"
                ? approving
                  ? "Continue"
                  : "Sign in"
                : "Continue"}
          </button>
        )}
      </div>
    </main>
  );
}

function Server({ found }: { found: SignInFound }) {
  return (
    <dl>
      <dt>Name</dt>
      <dd>{found.name}</dd>
      <dt>Address</dt>
      <dd>{found.address}</dd>
      <dt>Certificate fingerprint (SHA-256)</dt>
      <dd class="fingerprint">{found.fingerprint}</dd>
    </dl>
  );
}

/**
 * Sign in without the password: this device shows a code (or sends its request
 * to the code it scanned), then waits until the other device's user approves.
 */
function WaitForApproval({
  found,
  mode,
  portable,
  onSignedIn,
  onError,
}: {
  found: SignInFound;
  mode: "scan" | "show";
  portable: boolean;
  onSignedIn: (p: Profile) => void;
  onError: (message: string) => void;
}) {
  const [code, setCode] = useState<ApprovalCode | null>(null);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const profile = await finishApproval();
        if (cancelled) return;
        if (profile) onSignedIn(profile);
        else timer = setTimeout(poll, 1000);
      } catch (e) {
        if (!cancelled) onError(String(e));
      }
    };
    const start = async () => {
      try {
        if (mode === "show") {
          const c = await approvalShow(found.address, found.fingerprint, found.name, portable);
          if (cancelled) return;
          setCode(c);
        } else {
          await approvalScan(found.approvalLink ?? "", portable);
        }
        if (cancelled) return;
        setReady(true);
        timer = setTimeout(poll, 1000);
      } catch (e) {
        if (!cancelled) onError(String(e));
      }
    };
    void start();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, []);

  return (
    <section aria-live="polite">
      {mode === "show" ? (
        <>
          <p>
            On a device that is already signed in, open Settings → Account → Approve a new device,
            and scan this code or paste its link.
          </p>
          {code && (
            <>
              <div
                class="qr"
                role="img"
                aria-label="Approval code as a QR code"
                // The SVG is made by this app from the server's own address.
                dangerouslySetInnerHTML={{ __html: code.svg }}
              />
              <p class="fingerprint" data-testid="approval-link">{code.link}</p>
            </>
          )}
        </>
      ) : (
        <p>
          The request was sent. Your other device shows this device's name; confirm it there.
        </p>
      )}
      <p class="muted">
        {ready ? "Waiting for your other device to approve this one…" : "Working…"} The code works
        once and expires after a few minutes. No password is typed on this device.
      </p>
    </section>
  );
}
