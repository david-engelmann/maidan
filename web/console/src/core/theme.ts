// Dark, Light or System, kept per browser. Dark is the default: a person who
// never chose sees the dark console whatever their system says. The choice is
// applied to <html data-theme> before the first paint by public/theme.js;
// this module applies it again after a change and follows the system while
// the choice is System.

export type ThemeChoice = "dark" | "light" | "system";
export type Theme = "dark" | "light";

export const THEME_KEY = "maidan_console_theme";
export const THEME_CHOICES: readonly { id: ThemeChoice; label: string }[] = [
  { id: "dark", label: "Dark" },
  { id: "light", label: "Light" },
  { id: "system", label: "System" },
];
/** The page colour of each theme, for the browser's own chrome (theme-color). */
export const THEME_COLOR: Record<Theme, string> = { dark: "#0c0d0f", light: "#f2eee6" };
const LIGHT_QUERY = "(prefers-color-scheme: light)";

export function parseChoice(raw: string | null | undefined): ThemeChoice {
  return raw === "light" || raw === "system" ? raw : "dark";
}

export function resolveTheme(choice: ThemeChoice, systemLight: boolean): Theme {
  if (choice === "system") return systemLight ? "light" : "dark";
  return choice;
}

export function readChoice(): ThemeChoice {
  try {
    return parseChoice(window.localStorage.getItem(THEME_KEY));
  } catch {
    return "dark";
  }
}

function systemLight(): boolean {
  try {
    return window.matchMedia(LIGHT_QUERY).matches;
  } catch {
    return false;
  }
}

/** Puts a choice on the page: the palette, form controls and the browser's chrome. */
export function applyTheme(choice: ThemeChoice, doc: Document = document): Theme {
  const theme = resolveTheme(choice, systemLight());
  doc.documentElement.dataset.theme = theme;
  doc.documentElement.dataset.themeChoice = choice;
  doc.querySelector('meta[name="theme-color"]')?.setAttribute("content", THEME_COLOR[theme]);
  return theme;
}

const listeners = new Set<() => void>();
let current: ThemeChoice | null = null;

export function getChoice(): ThemeChoice {
  current ??= readChoice();
  return current;
}

export function subscribeTheme(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

/** Saves and applies a choice. A browser that refuses storage still switches for this visit. */
export function setChoice(choice: ThemeChoice): void {
  current = choice;
  try {
    window.localStorage.setItem(THEME_KEY, choice);
  } catch {
    // Storage blocked: the switch holds until the page closes.
  }
  applyTheme(choice);
  for (const fn of listeners) fn();
}

/** Applies the stored choice and keeps the page in step with the system and other tabs. */
export function startTheme(): () => void {
  applyTheme(getChoice());
  const onSystem = () => {
    if (getChoice() === "system") applyTheme("system");
  };
  // Another tab changed the theme (key is ours) or cleared storage (key is
  // null). Read what is stored now rather than the event's newValue: events
  // queue, and an older one must not undo a newer choice.
  const onStorage = (e: StorageEvent) => {
    if (e.key !== null && e.key !== THEME_KEY) return;
    if (e.storageArea !== null && e.storageArea !== window.localStorage) return;
    current = readChoice();
    applyTheme(current);
    for (const fn of listeners) fn();
  };
  let media: MediaQueryList | null = null;
  try {
    media = window.matchMedia(LIGHT_QUERY);
    media.addEventListener("change", onSystem);
  } catch {
    media = null;
  }
  window.addEventListener("storage", onStorage);
  return () => {
    media?.removeEventListener("change", onSystem);
    window.removeEventListener("storage", onStorage);
  };
}
