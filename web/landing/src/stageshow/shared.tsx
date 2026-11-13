import { useSyncExternalStore } from "react";
import { Avatar } from "./kit";

// The console's own line icons (web/console/src/shell/icons.tsx), so the
// drawn board carries the product's glyphs rather than a stock set.
const PATHS = {
  board: "M4 5h4v14H4zM10 5h4v9h-4zM16 5h4v6h-4z",
  needs: "M12 3a6 6 0 0 0-6 6v4l-2 3h16l-2-3V9a6 6 0 0 0-6-6zM10 19a2 2 0 0 0 4 0",
  threads: "M4 5h16v10H9l-5 4z",
  people: "M9 11a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM3 19c0-3 3-5 6-5s6 2 6 5M17 11a2.5 2.5 0 1 0 0-5M17 14c2 0 4 1.5 4 4",
  activity: "M3 12h4l3-7 4 14 3-7h4",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM20 20l-4-4",
  check: "M5 12.5l4.5 4.5L19 7",
  chevron: "M7 10l5 5 5-5",
} as const;

export type IconName = keyof typeof PATHS;

export function Icon({ name, size = 14 }: { name: IconName; size?: number }) {
  return (
    <svg className="ml-icon" width={size} height={size} viewBox="0 0 24 24" aria-hidden="true">
      <path d={PATHS[name]} />
    </svg>
  );
}

export type DotState = "open" | "running" | "review" | "done";

export function Dot({ state }: { state: DotState }) {
  return <i className="ml-dot" data-state={state} />;
}

/** An agent's avatar: steel, with a mono initial. */
export function Agent({ name, size = 18 }: { name: string; size?: number }) {
  return <Avatar name={name} tone="ink" size={size} />;
}

// The page's theme lives on <html data-theme> (set by the console's
// theme.js before first paint), so every stage follows it.
function onThemeChange(done: () => void) {
  const watch = new MutationObserver(done);
  watch.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => watch.disconnect();
}

export function usePageTheme(): "light" | "dark" {
  return useSyncExternalStore(
    onThemeChange,
    () => (document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark"),
    () => "dark",
  );
}

function onMotionChange(done: () => void) {
  const query = window.matchMedia("(prefers-reduced-motion: reduce)");
  query.addEventListener("change", done);
  return () => query.removeEventListener("change", done);
}

export function usePrefersReducedMotion(): boolean {
  return useSyncExternalStore(
    onMotionChange,
    () => window.matchMedia("(prefers-reduced-motion: reduce)").matches,
    () => false,
  );
}
