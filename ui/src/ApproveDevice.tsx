import { useEffect, useState } from "preact/hooks";
import {
  approvePending,
  offerApproval,
  pendingApproval,
  type ApprovalCode,
  type PendingDevice,
} from "./api";

/**
 * Settings → Account: approve a new device without its owner typing the
 * password. Either this device shows a code for the new one to scan, or the
 * new one shows a code and its link is pasted here. The new device's name is
 * shown, and nothing happens until the user confirms it.
 */
export function ApproveDevice() {
  const [code, setCode] = useState<ApprovalCode | null>(null);
  const [pasted, setPasted] = useState("");
  const [link, setLink] = useState("");
  const [pending, setPending] = useState<PendingDevice | null>(null);
  const [done, setDone] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  // Ask until the new device has sent its request.
  useEffect(() => {
    if (!link || pending) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const poll = async () => {
      try {
        const found = await pendingApproval(link);
        if (cancelled) return;
        if (found) setPending(found);
        else timer = setTimeout(poll, 1500);
      } catch (e) {
        if (cancelled) return;
        setError(String(e));
        setLink("");
        setCode(null);
      }
    };
    void poll();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [link, pending]);

  const reset = () => {
    setCode(null);
    setLink("");
    setPending(null);
    setPasted("");
  };

  const show = async () => {
    setError("");
    setDone("");
    try {
      const c = await offerApproval();
      setCode(c);
      setLink(c.link);
    } catch (e) {
      setError(String(e));
    }
  };

  const approve = async () => {
    setBusy(true);
    setError("");
    try {
      await approvePending();
      setDone(`${pending?.name ?? "The device"} is approved and signing in.`);
      reset();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section aria-labelledby="approve-device">
      <h3 id="approve-device">Approve a new device</h3>
      <p class="muted">
        An alternative to typing your password on the new device. Show a code here and scan it
        there, or choose Sign in on the new device, approve from another device, and paste the link
        it shows.
      </p>
      {error && <p class="error" role="alert">{error}</p>}
      {done && <p role="status">{done}</p>}

      {pending ? (
        <div role="alertdialog" aria-labelledby="approve-question">
          <p id="approve-question">
            Approve “{pending.name}” ({pending.portable ? "portable" : "stationary"})? It will get
            your keys and be able to read and change your reminders. Only approve a device you are
            holding.
          </p>
          <button type="button" onClick={approve} disabled={busy}>Approve</button>{" "}
          <button type="button" onClick={reset} disabled={busy}>Cancel</button>
        </div>
      ) : code ? (
        <>
          <div
            class="qr"
            role="img"
            aria-label="Approval code as a QR code"
            // The SVG is made by this app from the server's own address.
            dangerouslySetInnerHTML={{ __html: code.svg }}
          />
          <p class="fingerprint" data-testid="approval-offer-link">{code.link}</p>
          <p class="muted">
            Waiting for the new device. The code works once and expires after a few minutes.
          </p>
          <button type="button" onClick={reset}>Cancel</button>
        </>
      ) : (
        <>
          <button type="button" onClick={show}>Show a code</button>
          <label>
            Or paste the link the new device shows
            <input
              value={pasted}
              placeholder="hab-bot://approve?…"
              autocomplete="off"
              spellcheck={false}
              onInput={(e) => setPasted(e.currentTarget.value)}
            />
          </label>
          <button type="button" disabled={pasted.trim() === ""} onClick={() => setLink(pasted.trim())}>
            Look up the device
          </button>
        </>
      )}
    </section>
  );
}
