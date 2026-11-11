// One line over every place while the server is out of reach (UI-008). The
// screens keep what they last loaded, so the line says it may be out of date;
// it goes away on the first answer from the server.
import { useEffect, useSyncExternalStore } from "react";
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
  const text = offlineBanner(useReach());
  if (!text) return null;
  return (
    <div className="stale-banner offline-banner" role="status">
      <span>{text}</span>
    </div>
  );
}
