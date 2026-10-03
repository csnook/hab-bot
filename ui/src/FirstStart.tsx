import { useEffect, useState } from "preact/hooks";
import {
  chooseStandalone,
  joinServer,
  passphraseSuggestion,
  passwordCheck,
  probeServer,
  type FoundServer,
  type PasswordCheck,
  type Profile,
  type Setup,
} from "./api";
import {
  canContinue,
  DATA_LOSS_WARNING,
  emptyForm,
  meterLabel,
  STEPS,
  type JoinForm,
  type Step,
} from "./join";
import { ServerSees } from "./ServerSees";
import { SignIn } from "./SignIn";

type Screen = "choice" | "join" | "signin" | "done";

export function FirstStart({ onDone }: { onDone: (s: Setup) => void }) {
  const [screen, setScreen] = useState<Screen>("choice");
  const [error, setError] = useState("");
  const [joined, setJoined] = useState<Profile | null>(null);

  const standalone = async () => {
    try {
      await chooseStandalone();
      onDone({ mode: "standalone" });
    } catch (e) {
      setError(String(e));
    }
  };

  if (screen === "join") {
    return (
      <Join
        onCancel={() => setScreen("choice")}
        onJoined={(p) => {
          setJoined(p);
          setScreen("done");
        }}
      />
    );
  }

  if (screen === "done" && joined) {
    return (
      <main>
        <h1>You're in</h1>
        <p>
          {joined.display_name}, your account <strong>{joined.username}</strong> on{" "}
          <strong>{joined.server_name}</strong> is ready, and you are its server admin. The setup code
          no longer works.
        </p>
        <button onClick={() => onDone({ mode: "joined", ...joined })}>Open Reminders</button>
      </main>
    );
  }

  if (screen === "signin") {
    return (
      <SignIn
        onCancel={() => setScreen("choice")}
        onSignedIn={(p) => onDone({ mode: "joined", ...p })}
      />
    );
  }

  return (
    <main>
      <h1>Reminders</h1>
      <p>How do you want to use this device?</p>
      {error && <p class="error" role="alert">{error}</p>}
      <div class="choices">
        <button class="choice" onClick={standalone}>
          <strong>This device only</strong>
          <span>Everything works here, with no server.</span>
        </button>
        <button class="choice" onClick={() => setScreen("join")}>
          <strong>Join with an invite</strong>
          <span>Or with the setup code, to create the first account on your server.</span>
        </button>
        <button class="choice" onClick={() => setScreen("signin")}>
          <strong>Sign in</strong>
          <span>You already have an account on a server.</span>
        </button>
      </div>
    </main>
  );
}

function Join({
  onCancel,
  onJoined,
}: {
  onCancel: () => void;
  onJoined: (p: Profile) => void;
}) {
  const [step, setStep] = useState<Step>("server");
  const [form, setForm] = useState<JoinForm>(emptyForm);
  const [found, setFound] = useState<FoundServer | null>(null);
  const [check, setCheck] = useState<PasswordCheck | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const set = (patch: Partial<JoinForm>) => setForm((f) => ({ ...f, ...patch }));

  // Judge the password as it is typed. A slow answer for an old password is dropped.
  useEffect(() => {
    if (form.password === "") {
      setCheck(null);
      return;
    }
    let current = true;
    passwordCheck(form.password, form.username, form.displayName)
      .then((c) => current && setCheck(c))
      .catch(() => current && setCheck(null));
    return () => {
      current = false;
    };
  }, [form.password, form.username, form.displayName]);

  const index = STEPS.indexOf(step);
  const enabled = canContinue(step, form, check?.ok === true) && !busy;

  const back = () => {
    setError("");
    if (index === 0) onCancel();
    else setStep(STEPS[index - 1]);
  };

  const next = async () => {
    setError("");
    if (step === "server") {
      setBusy(true);
      try {
        setFound(await probeServer(form.address.trim()));
        setStep("confirm");
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    } else if (step === "device") {
      if (!found) return;
      setBusy(true);
      try {
        const profile = await joinServer({
          address: form.address.trim(),
          fingerprint: found.fingerprint,
          serverName: found.name,
          setupCode: form.setupCode,
          username: form.username,
          displayName: form.displayName.trim(),
          password: form.password,
          portable: form.portable === true,
        });
        onJoined(profile);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    } else {
      setStep(STEPS[index + 1]);
    }
  };

  const suggest = async () => {
    const p = await passphraseSuggestion();
    set({ password: p, repeat: p });
  };

  return (
    <main>
      <h1>Join a server</h1>
      <p class="muted">Step {index + 1} of {STEPS.length}</p>
      {error && <p class="error" role="alert">{error}</p>}

      {step === "server" && (
        <section>
          <label>
            Server address
            <input
              value={form.address}
              placeholder="home.lan:8443"
              onInput={(e) => set({ address: e.currentTarget.value })}
            />
          </label>
          <label>
            Setup code
            <input
              value={form.setupCode}
              placeholder="shown in the server's console"
              autocomplete="off"
              spellcheck={false}
              onInput={(e) => set({ setupCode: e.currentTarget.value })}
            />
          </label>
        </section>
      )}

      {step === "confirm" && found && (
        <section>
          <p>Is this your server?</p>
          <dl>
            <dt>Name</dt>
            <dd>{found.name}</dd>
            <dt>Version</dt>
            <dd>{found.version}</dd>
            <dt>Certificate fingerprint (SHA-256)</dt>
            <dd class="fingerprint">{found.fingerprint}</dd>
          </dl>
          <p class="muted">
            Compare the fingerprint with the one the server printed. This device will only talk to a
            server with this certificate.
          </p>
          <ServerSees />
        </section>
      )}

      {step === "profile" && (
        <section>
          <label>
            Username
            <input
              value={form.username}
              autocomplete="off"
              onInput={(e) => set({ username: e.currentTarget.value })}
            />
            <small class="muted">Lower-case letters, digits, dots, dashes and underscores.</small>
          </label>
          <label>
            Display name
            <input
              value={form.displayName}
              onInput={(e) => set({ displayName: e.currentTarget.value })}
            />
          </label>
        </section>
      )}

      {step === "password" && (
        <section>
          <label>
            Password
            <input
              type="password"
              value={form.password}
              autocomplete="new-password"
              onInput={(e) => set({ password: e.currentTarget.value })}
            />
          </label>
          <div class="meter" aria-live="polite">
            <progress max={4} value={check ? check.score : 0} data-ok={check?.ok ? "yes" : "no"} />
            <span>
              {check ? meterLabel(check.score) : "Enter a password"}
              {check && !check.ok && " — not strong enough yet"}
            </span>
          </div>
          {check?.warning && <p class="muted">{check.warning}</p>}
          {check && !check.ok && check.suggestions.map((s) => <p class="muted" key={s}>{s}</p>)}
          <label>
            Repeat the password
            <input
              type="password"
              value={form.repeat}
              autocomplete="new-password"
              onInput={(e) => set({ repeat: e.currentTarget.value })}
            />
          </label>
          {form.repeat !== "" && form.repeat !== form.password && (
            <p class="error">The passwords don't match.</p>
          )}
          <button type="button" onClick={suggest}>Suggest a passphrase</button>
          <p class="warning">{DATA_LOSS_WARNING}</p>
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
        </section>
      )}

      <div class="buttons">
        <button type="button" onClick={back} disabled={busy}>Back</button>
        <button type="button" onClick={next} disabled={!enabled}>
          {busy ? "Working…" : step === "device" ? "Create account" : "Continue"}
        </button>
      </div>
    </main>
  );
}
