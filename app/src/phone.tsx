/** The phone layout of the shared UI: a bottom navigation, swipes with Undo, and a bottom sheet. */
import type { ComponentChildren } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";

/** Phones lay out as the Android screens; larger windows as the desktop window. */
export function usePhone(): boolean {
  const query = "(max-width: 700px)";
  const [phone, setPhone] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const m = window.matchMedia(query);
    const on = () => setPhone(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, []);
  return phone;
}

/** The view last used, remembered between runs. Storage can be unavailable, so it's optional. */
export function rememberedView<T extends string>(views: readonly T[], fallback: T): T {
  try {
    const v = localStorage.getItem("view") as T | null;
    return v && views.includes(v) ? v : fallback;
  } catch {
    return fallback;
  }
}
export function rememberView(view: string) {
  try {
    localStorage.setItem("view", view);
  } catch {
    /* not remembered */
  }
}

/** How far to drag, in pixels, before a swipe counts. */
export const SWIPE_DISTANCE = 80;

/** A row that swipes right for one action and left for another. Rows that aren't open don't swipe. */
export function Swipeable(props: {
  enabled: boolean;
  onRight: () => void;
  onLeft: () => void;
  children: ComponentChildren;
  class?: string;
}) {
  const start = useRef<{ x: number; y: number } | null>(null);
  const [dx, setDx] = useState(0);
  if (!props.enabled) return <li class={props.class}>{props.children}</li>;

  const finish = () => {
    if (dx >= SWIPE_DISTANCE) props.onRight();
    else if (dx <= -SWIPE_DISTANCE) props.onLeft();
    start.current = null;
    setDx(0);
  };
  return (
    <li
      class={`swipeable ${props.class ?? ""} ${dx > 0 ? "swiping-right" : dx < 0 ? "swiping-left" : ""}`}
      style={{ transform: `translateX(${dx}px)`, touchAction: "pan-y" }}
      data-testid="swipeable"
      onPointerDown={(e) => {
        start.current = { x: e.clientX, y: e.clientY };
      }}
      onPointerMove={(e) => {
        if (!start.current) return;
        const x = e.clientX - start.current.x;
        // mostly sideways: vertical scrolling is left alone
        if (Math.abs(x) > Math.abs(e.clientY - start.current.y)) {
          // capture only once it's a drag, so a plain tap still reaches the row's own buttons
          if (Math.abs(x) > 8 && dx === 0) e.currentTarget.setPointerCapture(e.pointerId);
          setDx(x);
        }
      }}
      onPointerUp={finish}
      onPointerCancel={() => {
        start.current = null;
        setDx(0);
      }}
    >
      {props.children}
    </li>
  );
}

export interface UndoAction {
  label: string;
  undo: () => Promise<unknown>;
}

/** "Done · Undo", for 5 seconds. */
export function UndoToast({ action, onGone }: { action: UndoAction | null; onGone: () => void }) {
  useEffect(() => {
    if (!action) return;
    const t = setTimeout(onGone, 5000);
    return () => clearTimeout(t);
  }, [action]);
  if (!action) return null;
  return (
    <div class="toast" role="status">
      {action.label}
      <button onClick={() => action.undo().then(onGone, onGone)}>Undo</button>
    </div>
  );
}

export const VIEWS = ["inbox"] as const;
export type View = (typeof VIEWS)[number];
const VIEW_NAMES: Record<View, string> = { inbox: "Inbox" };

export function BottomNav({ view, onView }: { view: View; onView: (v: View) => void }) {
  return (
    <nav class="bottom">
      {VIEWS.map((v) => (
        <button class={v === view ? "current" : ""} onClick={() => onView(v)}>
          {VIEW_NAMES[v]}
        </button>
      ))}
    </nav>
  );
}

export function TopBar(props: { title: string; onFilter: () => void; onSettings: () => void }) {
  return (
    <header class="topbar">
      <h1>{props.title}</h1>
      <button title="Filters" aria-label="Filter" onClick={props.onFilter}>
        ⚲
      </button>
      <button aria-label="Settings" onClick={props.onSettings}>
        ⚙
      </button>
    </header>
  );
}
