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
export function selectLiveToasts(list: Notice[], gone: ReadonlySet<number>, navAt: number, now: number, credit: number, frozenAt: number | null): Notice[] {
  const clock = frozenAt ?? Math.max(now, list[0]?.at ?? 0);
  return list.filter((n) => !gone.has(n.id) && n.at >= navAt && clock - n.at - credit < TOAST_MS).slice(0, 3);
}

/** Pause time is given back. A toast that would have under 1.5 s left gets that grace instead of snapping shut. */
export function pauseCredit(credit: number, spent: number, now: number, list: Notice[], navAt: number, gone: ReadonlySet<number>): number {
  const next = credit + spent;
  const ages = list.filter((n) => !gone.has(n.id) && n.at >= navAt).map((n) => now - n.at - next);
  if (!ages.length) return next;
  const remain = TOAST_MS - Math.max(...ages);
  return remain < TOAST_GRACE_MS ? next + (TOAST_GRACE_MS - remain) : next;
}

export function Toasts() {
  const list = useSyncExternalStore(notices.subscribe, notices.snapshot);
  const [gone, setGone] = useState<ReadonlySet<number>>(() => new Set());
  const [now, setNow] = useState(() => Date.now());
  const [navAt, setNavAt] = useState(0);
  const [credit, setCredit] = useState(0);
  const [frozenAt, setFrozenAt] = useState<number | null>(null);
  const holds = useRef({ hover: false, focus: false });
  const frozenRef = useRef<number | null>(null);
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
  const live = selectLiveToasts(list, gone, navAt, now, credit, frozenAt);
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
  const release = (which: "hover" | "focus") => {
    holds.current[which] = false;
    if (holds.current.hover || holds.current.focus) return;
    const start = frozenRef.current;
    frozenRef.current = null;
    const spent = start !== null ? Date.now() - start : 0;
    const t = Date.now();
    setCredit((c) => pauseCredit(c, spent, t, listRef.current, navRef.current, goneRef.current));
    setFrozenAt(null);
    setNow(t);
  };
  const dismiss = () => {
    setGone((g) => {
      const next = new Set(g);
      for (const n of liveRef.current) next.add(n.id);
      return next;
    });
  };
  return (
    <div
      className="toasts"
      onMouseEnter={() => engage("hover")}
      onMouseLeave={() => release("hover")}
      onFocus={() => engage("focus")}
      onBlur={(e) => {
        if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
        release("focus");
      }}
      onKeyDown={(e) => {
        if (e.key !== "Escape") return;
        e.preventDefault();
        dismiss();
      }}
    >
      {live.map((n) => (
        <div
          key={n.id}
          className={`toast ${n.tone}`}
          role={toastRole(n.tone)}
          onMouseEnter={() => engage("hover")}
          onMouseLeave={() => release("hover")}
          onFocus={() => engage("focus")}
          onBlur={(e) => {
            if (e.currentTarget.contains(e.relatedTarget as Node | null)) return;
            release("focus");
          }}
        >
          <span>{n.text}</span>
          <button type="button" className="toast-x" aria-label="Dismiss" onClick={() => setGone((g) => new Set(g).add(n.id))}>×</button>
        </div>
      ))}
    </div>
  );
}
