import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { THEME_COLOR, THEME_KEY, parseChoice, resolveTheme } from "../src/core/theme.ts";

// public/theme.js sets the theme before the first paint and theme.ts after a
// change. If they disagreed, a reload would flip a page someone had just set.

const script = readFileSync(new URL("../public/theme.js", import.meta.url), "utf8");
const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8");
const html = readFileSync(new URL("../index.html", import.meta.url), "utf8");

/** What theme.js puts on <html> for a stored value and a system preference. */
function boot(stored: string | null | "throws", systemLight: boolean): { theme: string; choice: string; color: string | null } {
  const attrs = new Map<string, string>();
  let color: string | null = null;
  const window = {
    localStorage: {
      getItem(key: string) {
        if (stored === "throws") throw new Error("SecurityError");
        return key === THEME_KEY ? stored : null;
      },
    },
    matchMedia: (q: string) => ({ matches: q === "(prefers-color-scheme: light)" && systemLight }),
  };
  const document = {
    documentElement: { setAttribute: (k: string, v: string) => attrs.set(k, v) },
    querySelector: (sel: string) => (sel === 'meta[name="theme-color"]' ? { setAttribute: (_k: string, v: string) => (color = v) } : null),
  };
  runInNewContext(script, { window, document, matchMedia: window.matchMedia });
  return { theme: attrs.get("data-theme") ?? "", choice: attrs.get("data-theme-choice") ?? "", color };
}

test("dark is the default: nothing stored, a junk value, or blocked storage", () => {
  for (const stored of [null, "", "sepia", "throws"] as const) {
    for (const systemLight of [false, true]) {
      assert.equal(boot(stored, systemLight).theme, "dark", `${stored} with a light system: ${systemLight}`);
    }
  }
  assert.equal(parseChoice(null), "dark");
  assert.equal(parseChoice("sepia"), "dark");
});

test("the boot script and theme.ts resolve every choice the same way", () => {
  for (const stored of ["dark", "light", "system", null] as const) {
    for (const systemLight of [false, true]) {
      const got = boot(stored, systemLight);
      const want = resolveTheme(parseChoice(stored), systemLight);
      assert.equal(got.theme, want, `${stored}, light system ${systemLight}`);
      assert.equal(got.choice, parseChoice(stored));
      assert.equal(got.color, THEME_COLOR[want]);
    }
  }
});

test("the browser chrome colour of each theme is that theme's page colour", () => {
  const paper = (block: string) => css.slice(css.indexOf(block)).match(/--paper:\s*(#[0-9a-f]{6})/i)?.[1];
  assert.equal(THEME_COLOR.dark, paper(':root, :root[data-theme="dark"] {'));
  assert.equal(THEME_COLOR.light, paper(':root[data-theme="light"] {'));
  assert.match(html, new RegExp(`<meta name="theme-color" content="${THEME_COLOR.dark}" />`));
});

test("the page loads the boot script before its stylesheet and its app", () => {
  // A blocking script ahead of the CSS runs before the first paint; the
  // module app runs after it, too late to keep a Light page from flashing.
  const boot = html.indexOf('<script src="/theme.js"></script>');
  assert.ok(boot > 0, "index.html loads /theme.js (built to /ui/theme.js)");
  assert.ok(boot < html.indexOf('type="module"'), "before the app");
  const link = html.indexOf('rel="stylesheet"');
  assert.ok(link === -1 || boot < link, "before any stylesheet");
});
