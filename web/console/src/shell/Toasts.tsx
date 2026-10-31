// The toasts. A notice shows for five seconds, or until dismissed; the
// recent list on Tools keeps it after that.
import { useEffect, useState, useSyncExternalStore } from "react";
import { notices } from "../core/notices.ts";

export const TOAST_MS = 5000;

export function Toasts() {
  const list = useSyncExternalStore(notices.subscribe, notices.snapshot);
  const [gone, setGone] = useState<ReadonlySet<number>>(() => new Set());
  const [now, setNow] = useState(() => Date.now());
  // A notice newer than the last tick is judged from its own time, so a stale
  // "now" never hides or stretches it.
  const at = Math.max(now, list[0]?.at ?? 0);
  const live = list.filter((n) => !gone.has(n.id) && at - n.at < TOAST_MS).slice(0, 3);
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
