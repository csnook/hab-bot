import { useEffect, useState } from "preact/hooks";
import {
  changePassword,
  passphraseSuggestion,
  passwordCheck,
  type PasswordCheck,
  type Profile,
} from "./api";

/**
 * Settings → Account: set a new password from this signed-in device. The old
 * one isn't asked for, so this is also how a forgotten password is replaced
 * while a device is still signed in. Other devices keep working.
 */
export function ChangePassword({ setup }: { setup: Profile }) {
  const [password, setPassword] = useState("");
  const [repeat, setRepeat] = useState("");
  const [check, setCheck] = useState<PasswordCheck | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    if (password === "") {
      setCheck(null);
      return;
    }
    let current = true;
    passwordCheck(password, setup.username, setup.display_name)
      .then((c) => current && setCheck(c))
      .catch(() => current && setCheck(null));
    return () => {
      current = false;
    };
  }, [password, setup.username, setup.display_name]);

  const ready = check?.ok === true && password === repeat && !busy;

  const suggest = async () => {
    const p = await passphraseSuggestion();
    setPassword(p);
    setRepeat(p);
  };

  const submit = async (e: Event) => {
    e.preventDefault();
    if (!ready) return;
    setBusy(true);
    setError("");
    setDone(false);
    try {
      await changePassword(password);
      setPassword("");
      setRepeat("");
      setDone(true);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <form aria-labelledby="change-password" onSubmit={submit}>
      <h3 id="change-password">Change password</h3>
      <p>
        Sets a new password for your account. Your other devices keep working. If you forgot
        your password, set a new one here.
      </p>
      <label>
        New password
        <input
          type="password"
          autocomplete="new-password"
          value={password}
          onInput={(e) => setPassword(e.currentTarget.value)}
        />
      </label>
      {check && !check.ok && (
        <p role="status">{check.warning ?? "That password is too easy to guess."}</p>
      )}
      <label>
        Repeat it
        <input
          type="password"
          autocomplete="new-password"
          value={repeat}
          onInput={(e) => setRepeat(e.currentTarget.value)}
        />
      </label>
      {repeat !== "" && repeat !== password && <p role="status">The passwords differ.</p>}
      <button type="button" onClick={() => void suggest()}>Suggest a passphrase</button>
      <button type="submit" disabled={!ready}>Change password</button>
      {done && <p role="status">Password changed.</p>}
      {error && <p role="alert">{error}</p>}
    </form>
  );
}
