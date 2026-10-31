// The toasts. A notice shows for five seconds, or until dismissed; the
// recent list on Tools keeps it after that. Moving to another page clears
// what was showing: a toast speaks about the page it was raised on.
import { useEffect, useState, useSyncExternalStore } from "react";
import { notices } from "../core/notices.ts";

export const TOAST_MS = 5000;

export function Toasts() {
  const list = useSyncExternalStore(notices.subscribe, notices.snapshot);
  const [gone, setGone] = useState<ReadonlySet<number>>(() => new Set());
  const [now, setNow] = useState(() => Date.now());
  const [navAt, setNavAt] = useState(0);
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
  // A notice newer than the last tick is judged from its own time, so a stale
  // "now" never hides or stretches it.
  const at = Math.max(now, list[0]?.at ?? 0);
  const live = list.filter((n) => !gone.has(n.id) && n.at >= navAt && at - n.at < TOAST_MS).slice(0, 3);
  const busy = live.length > 0;
  useEffect(() => {
    if (!busy) return;
    const t = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(t);
  }, [busy]);
  return (
    <div className="toasts" role="status" aria-live="polite">
      {live.map((n) => (
        <div key={n.id} className={`toast ${n.tone}`}>
          <span>{n.text}</span>
          <button type="button" className="toast-x" aria-label="Dismiss" onClick={() => setGone((g) => new Set(g).add(n.id))}>×</button>
        </div>
      ))}
    </div>
  );
}
