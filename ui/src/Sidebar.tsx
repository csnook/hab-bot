import { useState } from "preact/hooks";
import {
  colourList,
  createList,
  deleteList,
  renameList,
  type Filters,
  type ListInfo,
  type PriorityName,
} from "./api";
import { PRIORITIES, setListShown, setPriorityShown, showsList, showsPriority } from "./filters";
import {
  PALETTE,
  canDeleteList,
  checkListName,
  listColour,
  listName,
  nextColour,
  whyNotDeletable,
} from "./lists";
import { priorityName } from "./delays";

/**
 * The left sidebar: a checkbox for each list and each priority, which filter
 * every view and are remembered, and where lists are made, renamed, coloured
 * and deleted. Hiding a list only hides it from the window: its alerts still
 * come.
 */
export function Sidebar({
  lists,
  filters,
  onFilters,
  onChanged,
  onError,
}: {
  lists: ListInfo[];
  filters: Filters;
  onFilters: (f: Filters) => void;
  /** A list was made, renamed, coloured or deleted. */
  onChanged: () => void;
  onError: (message: string) => void;
}) {
  const [adding, setAdding] = useState(false);
  const [managing, setManaging] = useState<string | null>(null);

  return (
    <aside class="sidebar" aria-label="Filters">
      <section aria-labelledby="side-lists">
        <h2 id="side-lists">Lists</h2>
        <ul>
          {lists.map((l) => (
            <li key={l.id}>
              <label class="check">
                <input
                  type="checkbox"
                  checked={showsList(filters, l.id)}
                  onChange={(e) => onFilters(setListShown(filters, l.id, e.currentTarget.checked))}
                />
                <span class="dot" style={{ background: listColour(l, lists) }} aria-hidden="true" />
                <span class="name">{listName(l)}</span>
                <span class="count">{l.reminders}</span>
              </label>
              <button
                type="button"
                class="more"
                aria-label={`Manage ${listName(l)}`}
                aria-expanded={managing === l.id}
                onClick={() => setManaging(managing === l.id ? null : l.id)}
              >
                ⋯
              </button>
              {managing === l.id && (
                <ManageList
                  list={l}
                  lists={lists}
                  onDone={() => {
                    setManaging(null);
                    onChanged();
                  }}
                  onError={onError}
                />
              )}
            </li>
          ))}
        </ul>
        {adding ? (
          <NewList
            lists={lists}
            onDone={() => {
              setAdding(false);
              onChanged();
            }}
            onCancel={() => setAdding(false)}
            onError={onError}
          />
        ) : (
          <button type="button" onClick={() => setAdding(true)}>New list…</button>
        )}
      </section>

      <section aria-labelledby="side-priorities">
        <h2 id="side-priorities">Priorities</h2>
        <ul>
          {PRIORITIES.map((p: PriorityName) => (
            <li key={p}>
              <label class="check">
                <input
                  type="checkbox"
                  checked={showsPriority(filters, p)}
                  onChange={(e) =>
                    onFilters(setPriorityShown(filters, p, e.currentTarget.checked))
                  }
                />
                <span class="name">{priorityName(p)}</span>
              </label>
            </li>
          ))}
        </ul>
      </section>
    </aside>
  );
}

function NewList({
  lists,
  onDone,
  onCancel,
  onError,
}: {
  lists: ListInfo[];
  onDone: () => void;
  onCancel: () => void;
  onError: (message: string) => void;
}) {
  const [name, setName] = useState("");
  const [colour, setColour] = useState(() => nextColour(lists));
  const [error, setError] = useState("");

  const submit = async (e: Event) => {
    e.preventDefault();
    const checked = checkListName(name);
    if (!checked.ok) return setError(checked.error);
    try {
      await createList(checked.value, colour);
      onDone();
    } catch (err) {
      onError(String(err));
    }
  };

  return (
    <form class="list-form" onSubmit={submit} aria-label="New list">
      <label>
        Name
        <input value={name} onInput={(e) => setName(e.currentTarget.value)} required />
      </label>
      <Swatches value={colour} onChange={setColour} />
      {error && <p class="error" role="alert">{error}</p>}
      <div class="buttons">
        <button type="button" onClick={onCancel}>Cancel</button>
        <button type="submit">Make list</button>
      </div>
    </form>
  );
}

function ManageList({
  list,
  lists,
  onDone,
  onError,
}: {
  list: ListInfo;
  lists: ListInfo[];
  onDone: () => void;
  onError: (message: string) => void;
}) {
  const [name, setName] = useState(list.name ?? "");
  const [error, setError] = useState("");
  const current = listColour(list, lists);
  const run = (f: () => Promise<void>) => f().then(onDone).catch((e) => onError(String(e)));

  const rename = (e: Event) => {
    e.preventDefault();
    const checked = checkListName(name);
    if (!checked.ok) return setError(checked.error);
    run(() => renameList(list.id, checked.value));
  };
  const why = whyNotDeletable(list);

  return (
    <div class="list-form" role="group" aria-label={`${listName(list)} settings`}>
      {!list.personal && (
        <form onSubmit={rename}>
          <label>
            Name
            <input value={name} onInput={(e) => setName(e.currentTarget.value)} />
          </label>
          <button type="submit">Rename</button>
        </form>
      )}
      {list.personal && <p class="muted">The personal list is the default and keeps its name.</p>}
      <Swatches value={current} onChange={(c) => run(() => colourList(list.id, c))} />
      {error && <p class="error" role="alert">{error}</p>}
      <button
        type="button"
        class="danger"
        disabled={!canDeleteList(list)}
        title={why ?? undefined}
        onClick={() => run(() => deleteList(list.id))}
      >
        Delete list
      </button>
      {why && <p class="muted">{why}</p>}
    </div>
  );
}

function Swatches({ value, onChange }: { value: string; onChange: (c: string) => void }) {
  return (
    <fieldset class="swatches">
      <legend>Colour</legend>
      {PALETTE.map((c) => (
        <label key={c} class="swatch" title={c}>
          <input
            type="radio"
            name="list-colour"
            checked={value.toLowerCase() === c}
            onChange={() => onChange(c)}
            aria-label={c}
          />
          <span style={{ background: c }} />
        </label>
      ))}
    </fieldset>
  );
}
