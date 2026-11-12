// One line over every place while the server is out of reach (UI-008). The
// screens keep what they last loaded, so the line says it may be out of date;
// it goes away on the first answer from the server.
import { useEffect, useReducer, useSyncExternalStore } from "react";
import { offlineBanner, reach } from "../core/api.ts";

export function useReach() {
  useEffect(() => {
    const sync = () => reach.browser(navigator.onLine !== false);
    sync();
    window.addEventListener("online", sync);
    window.addEventListener("offline", sync);
    return () => {
      window.removeEventListener("online", sync);
      window.removeEventListener("offline", sync);
    };
  }, []);
  return useSyncExternalStore(reach.subscribe, reach.getSnapshot);
}

export function OfflineBanner() {
  const r = useReach();
  const [, tick] = useReducer((n: number) => n + 1, 0);
  const text = offlineBanner(r);
  const shown = text !== null;
  const lastAnswerAt = r.lastAnswerAt;
  // The store only speaks when reach changes, so during a steady outage one
  // timer re-renders the line when the last answer turns a minute old.
  useEffect(() => {
    if (!shown || lastAnswerAt === null) return;
    const wait = lastAnswerAt + 60_000 - Date.now();
    if (wait < 0) return;
    const timer = setTimeout(tick, wait + 1);
    return () => clearTimeout(timer);
  }, [shown, lastAnswerAt]);
  if (!text) return null;
  return (
    <div className="stale-banner offline-banner" role="status">
      <span>{text}</span>
    </div>
  );
}
