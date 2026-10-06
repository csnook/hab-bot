import { useEffect, useRef, useState } from "preact/hooks";
import { skipAllButton, skipAllQuestion } from "./folding";

/**
 * "Skip all…" on the older-quiet row: says how many it will skip and asks
 * once. Only what the row lists is skipped, so the sidebar's filters decide.
 */
export function SkipAllDialog({
  count,
  onConfirm,
  onCancel,
}: {
  count: number;
  onConfirm: () => Promise<void>;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  useEffect(() => {
    const d = ref.current;
    if (d && !d.open) {
      try {
        d.showModal();
      } catch {
        d.setAttribute("open", "");
      }
    }
  }, []);

  const submit = async (e: Event) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await onConfirm();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return (
    <dialog ref={ref} class="confirm" aria-labelledby="skip-all-title" onClose={onCancel}>
      <form onSubmit={submit} aria-label="Skip all">
        <h1 id="skip-all-title">{skipAllQuestion(count)}</h1>
        {error && <p class="error" role="alert">{error}</p>}
        <div class="buttons">
          <button type="button" onClick={onCancel}>Cancel</button>
          <button type="submit" disabled={busy}>{skipAllButton(count)}</button>
        </div>
      </form>
    </dialog>
  );
}
