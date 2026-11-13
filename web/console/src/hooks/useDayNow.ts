import { useEffect, useState } from "react";

/**
 * Now, read again just after each local midnight. A label that changes only
 * when the day does ("Today", "Yesterday") stays true in a conversation left
 * open overnight, without re-rendering every few seconds.
 */
export function useDayNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const midnight = new Date(now);
    midnight.setHours(24, 0, 0, 0);
    const t = window.setTimeout(() => setNow(Date.now()), Math.max(midnight.getTime() - Date.now(), 0) + 1000);
    return () => window.clearTimeout(t);
  }, [now]);
  return now;
}
