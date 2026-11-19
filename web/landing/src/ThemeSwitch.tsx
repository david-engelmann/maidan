import { useEffect, useState } from "react";
import { createPortal } from "react-dom";

// The console's own choice (web/console/src/core/theme.ts): the same key,
// labels and page colours, so a choice made here themes the console too, and
// one made there already themes this page before its first paint (theme.js).
type Choice = "dark" | "light" | "system";
const KEY = "maidan_console_theme";
const CHOICES: readonly { id: Choice; label: string }[] = [
  { id: "dark", label: "Dark" },
  { id: "light", label: "Light" },
  { id: "system", label: "System" },
];
const COLOR = { dark: "#0c0d0f", light: "#f2eee6" } as const;
const LIGHT_QUERY = "(prefers-color-scheme: light)";

function storedChoice(): Choice {
  try {
    const raw = window.localStorage.getItem(KEY);
    return raw === "light" || raw === "system" ? raw : "dark";
  } catch {
    return "dark";
  }
}

function applyChoice(choice: Choice): void {
  let light = choice === "light";
  if (choice === "system") {
    try {
      light = window.matchMedia(LIGHT_QUERY).matches;
    } catch {
      light = false;
    }
  }
  const root = document.documentElement;
  root.setAttribute("data-theme", light ? "light" : "dark");
  root.setAttribute("data-theme-choice", choice);
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", light ? COLOR.light : COLOR.dark);
}

/** Dark, Light or System, in the footer's slot (#theme-switch). Everything
 * drawn on the page follows <html data-theme>, so it all changes at once. */
export function ThemeSwitch() {
  const [choice, setChoice] = useState<Choice>(storedChoice);
  // While the choice is System, the page follows the system as it changes
  useEffect(() => {
    if (choice !== "system") return;
    const query = window.matchMedia(LIGHT_QUERY);
    const follow = () => applyChoice("system");
    query.addEventListener("change", follow);
    return () => query.removeEventListener("change", follow);
  }, [choice]);
  const pick = (next: Choice) => {
    setChoice(next);
    try {
      window.localStorage.setItem(KEY, next);
    } catch {
      // Storage blocked: the switch holds until the page closes.
    }
    applyChoice(next);
  };
  const slot = typeof document === "undefined" ? null : document.getElementById("theme-switch");
  if (!slot) return null;
  return createPortal(
    <div className="theme-switch" role="group" aria-label="Theme">
      {CHOICES.map((c) => (
        <button key={c.id} type="button" aria-pressed={choice === c.id} onClick={() => pick(c.id)}>
          {c.label}
        </button>
      ))}
    </div>,
    slot,
  );
}
