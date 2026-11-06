// The toasts. A notice shows for five seconds, or until dismissed; the
// recent list on Tools keeps it after that. Moving to another page clears
// what was showing: a toast speaks about the page it was raised on. Hovering
// or focusing pauses the countdown so it does not vanish under the pointer.
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { notices, type Notice, type Tone } from "../core/notices.ts";

export const TOAST_MS = 5000;
export const TOAST_GRACE_MS = 1500;

export function toastRole(tone: Tone): "alert" | "status" {
  return tone === "error" ? "alert" : "status";
}

/** Notices still on screen. `frozenAt` stops the clock while the pointer or focus is inside. */
export function selectLiveToasts(list: Notice[], gone: ReadonlySet<number>, navAt: number, now: number, credits: ReadonlyMap<number, number>, frozenAt: number | null): Notice[] {
  const clock = frozenAt ?? Math.max(now, list[0]?.at ?? 0);
  return list.filter((n) => !gone.has(n.id) && n.at >= navAt && clock - n.at - (credits.get(n.id) ?? 0) < TOAST_MS).slice(0, 3);
}

/**
 * Pause time belongs to the notices that were on screen for it.
 * A notice created after the pause keeps no credit, so it still lasts `TOAST_MS`.
 * Ids that have left `list` are dropped.
 */
export function nextToastCredits(credits: ReadonlyMap<number, number>, list: readonly Notice[], start: number, end: number): ReadonlyMap<number, number> {
  const next = new Map<number, number>();
  for (const n of list) {
    const prev = credits.get(n.id) ?? 0;
    const aliveUntil = n.at + TOAST_MS + prev;
    if (n.at >= end || start >= aliveUntil) {
      next.set(n.id, prev);
      continue;
    }
    const overlap = Math.max(0, end - Math.max(start, n.at));
    let credit = prev + overlap;
    const remain = TOAST_MS - (end - n.at - credit);
    if (remain < TOAST_GRACE_MS) credit += TOAST_GRACE_MS - remain;
    next.set(n.id, credit);
  }
  return next;
}

export function Toasts() {
  const list = useSyncExternalStore(notices.subscribe, notices.snapshot);
  const [gone, setGone] = useState<ReadonlySet<number>>(() => new Set());
  const [now, setNow] = useState(() => Date.now());
  const [navAt, setNavAt] = useState(0);
  const [credits, setCredits] = useState<ReadonlyMap<number, number>>(() => new Map());
  const [frozenAt, setFrozenAt] = useState<number | null>(null);
  const holds = useRef({ hover: false, focus: false });
  const frozenRef = useRef<number | null>(null);
  const rootRef = useRef<HTMLDivElement>(null);
  const listRef = useRef(list);
  const goneRef = useRef(gone);
  const navRef = useRef(navAt);
  useEffect(() => {
    const page = () => window.location.hash.split("/").slice(0, 2).join("/");
    let last = page();
    const on = () => {
      const next = page();
      if (next !== last) setNavAt(Date.now());
      last = next;
    };
    window.addEventListener("hashchange", on);
    return () => window.removeEventListener("hashchange", on);
  }, []);
  const live = selectLiveToasts(list, gone, navAt, now, credits, frozenAt);
  const liveRef = useRef(live);
  useEffect(() => {
    listRef.current = list;
    goneRef.current = gone;
    navRef.current = navAt;
    liveRef.current = live;
  });
  const busy = live.length > 0;
  useEffect(() => {
    if (!busy) return;
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, [busy]);
  const engage = (which: "hover" | "focus") => {
    if (!holds.current.hover && !holds.current.focus) {
      frozenRef.current = Date.now();
      setFrozenAt(frozenRef.current);
    }
    holds.current[which] = true;
  };
  const unfreeze = () => {
    const start = frozenRef.current;
    frozenRef.current = null;
    const t = Date.now();
    if (start !== null) {
      const showing = listRef.current.filter((n) => !goneRef.current.has(n.id) && n.at >= navRef.current);
      setCredits((c) => nextToastCredits(c, showing, start, t));
    }
    setFrozenAt(null);
    setNow(t);
  };
  const release = (which: "hover" | "focus") => {
    holds.current[which] = false;
    if (holds.current.hover || holds.current.focus) return;
    unfreeze();
  };
  const dismiss = () => {
    holds.current = { hover: false, focus: false };
    unfreeze();
    setGone((g) => {
      const next = new Set(g);
      for (const n of liveRef.current) next.add(n.id);
      return next;
    });
  };
  const dismissOne = (id: number) => {
    setGone((g) => new Set(g).add(id));
  };
  useEffect(() => {
    const root = rootRef.current;
    const inside = !!(root && document.activeElement && root.contains(document.activeElement));
    if (!inside) holds.current.focus = false;
    if (live.length === 0) holds.current.hover = false;
    if (holds.current.focus || holds.current.hover || frozenRef.current === null) return;
    unfreeze();
  }, [live.length]);
  return (
    <div
      ref={rootRef}
      className="toasts"
      onMouseEnter={() => engage("hover")}
      onMouseLeave={() => release("hover")}
      onFocus={() => engage("focus")}
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) release("focus");
      }}
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        dismiss();
      }}
    >
      {live.map((n) => (
        <div key={n.id} className={`toast ${n.tone}`} role={toastRole(n.tone)}>
          <span>{n.text}</span>
          <button type="button" className="toast-x" aria-label="Dismiss" onClick={() => dismissOne(n.id)}>×</button>
        </div>
      ))}
    </div>
  );
}
