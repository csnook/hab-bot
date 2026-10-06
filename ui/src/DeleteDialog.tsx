import { useEffect, useRef, useState } from "preact/hooks";
import { deleteReminder } from "./api";
import { DELETE_CHOICES, deleteQuestion, withHistory, type DeleteChoice } from "./deleting";

/**
 * Asks whether to keep a reminder's history, marked deleted, or to delete it
 * with its history, and does it.
 */
export function DeleteDialog({
  reminderId,
  title,
  onCancel,
  onDeleted,
}: {
  reminderId: string;
  title: string;
  onCancel: () => void;
  onDeleted: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [choice, setChoice] = useState<DeleteChoice>("keep");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

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

  const confirm = async (e: Event) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      await deleteReminder(reminderId, withHistory(choice));
      onDeleted();
    } catch (err) {
      setError(String(err));
      setBusy(false);
    }
  };

  return (
    <dialog ref={ref} class="confirm" aria-labelledby="delete-title" onClose={onCancel}>
      <form onSubmit={confirm} aria-label="Delete reminder">
        <h1 id="delete-title">{deleteQuestion(title)}</h1>
        {error && <p class="error" role="alert">{error}</p>}
        {DELETE_CHOICES.map((c) => (
          <label key={c.value} class="radio">
            <input
              type="radio"
              name="delete-choice"
              checked={choice === c.value}
              onChange={() => setChoice(c.value)}
            />
            <span>
              {c.label}
              <span class="muted block">{c.detail}</span>
            </span>
          </label>
        ))}
        <div class="buttons">
          <button type="button" onClick={onCancel}>Cancel</button>
          <button type="submit" class="danger" disabled={busy}>Delete</button>
        </div>
      </form>
    </dialog>
  );
}
