import { useEffect, useState } from "preact/hooks";
import type { ComponentChild } from "preact";
import type { BoardCard, Filters, ListInfo } from "./api";
import { buildBoard, fakingText, statusLine } from "./board";
import { listById, listName } from "./lists";
import type { PanelTarget } from "./OccurrencePanel";

/**
 * The Board: every reminder as a card, in columns by its state (board.ts).
 * Cards can't be dragged. Opening a card (it is a button, so Enter and Space
 * work) shows the reminder panel: Edit reminder…, Pause or Resume and
 * Duplicate, plus the buttons of its open occurrence.
 *
 * Type-checked only for this component: it has never been run in a real
 * window. The logic it shows is unit-tested in board.ts.
 */
export function Board({
  cards,
  filters,
  lists,
  dot,
  onOpen,
}: {
  cards: BoardCard[];
  filters: Filters;
  lists: ListInfo[];
  dot: (listId: string) => ComponentChild;
  onOpen: (t: PanelTarget) => void;
}) {
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));
  const [showOlder, setShowOlder] = useState(false);
  useEffect(() => {
    const t = setInterval(() => setNow(Math.floor(Date.now() / 1000)), 30_000);
    return () => clearInterval(t);
  }, []);
  const columns = buildBoard(filters, cards, now, showOlder);

  if (columns.length === 0) {
    return <p class="empty">No reminders to show.</p>;
  }
  return (
    <div class="board">
      {columns.map((col) => (
        <section key={col.id} class={`board-column ${col.id}`} aria-labelledby={`board-${col.id}`}>
          <h2 id={`board-${col.id}`}>
            {col.title} <span class="muted">{col.cards.length}</span>
          </h2>
          <ul>
            {col.cards.map((c) => {
              const list = listById(lists, c.list_id);
              return (
                <li key={c.reminder_id} class="board-card">
                  <button
                    type="button"
                    class="card-main"
                    onClick={() => onOpen({ state: "reminder", card: c })}
                  >
                    <span class="title">{c.title}</span>
                    <span class="status">{statusLine(c, now)}</span>
                    <span class="meta">
                      {dot(c.list_id)}
                      <span>{list ? listName(list) : ""}</span>
                      <span class="priority">{c.priority}</span>
                      <span class="faking">{fakingText(c.faking)}</span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
          {col.id === "finished" && !showOlder && col.olderHidden > 0 && (
            <button type="button" onClick={() => setShowOlder(true)}>
              Show older ({col.olderHidden})
            </button>
          )}
          {col.id === "finished" && showOlder && (
            <button type="button" onClick={() => setShowOlder(false)}>
              Hide older
            </button>
          )}
        </section>
      ))}
    </div>
  );
}
