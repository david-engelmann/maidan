import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// The console's surfaces share one radius scale and one palette, so a corner
// or a colour changes in one place and light and dark stay in step.

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
// Set at runtime from script (viewport, keyboard, composer, drag position, font).
const RUNTIME = new Set(["--vv-top", "--vv-h", "--composer-h", "--kb", "--left", "--font"]);

/** The body of the one palette block for a theme. */
function palette(scheme: "dark" | "light"): string {
  const head = scheme === "dark" ? ':root, :root[data-theme="dark"] {' : ':root[data-theme="light"] {';
  const at = css.indexOf(head);
  assert.ok(at >= 0, `${scheme} palette block`);
  return css.slice(at + head.length, css.indexOf("}", at));
}

test("every corner above 4px comes from the radius scale, in every value of a shorthand", () => {
  // Reading only the first value let `var(--r-lg) 18px 0 0` through. A token
  // with a px fallback counts as a literal too: the fallback drifts from the scale.
  const raw = [...css.matchAll(/border(?:-[a-z]+)*-radius:\s*([^;}]+)/g)].flatMap((m) =>
    [...m[1].replace(/var\(--r-[\w-]+\)/g, "").matchAll(/(?<![\d.])(\d*\.?\d+)px/g)]
      .filter((px) => Number(px[1]) > 4)
      .map((px) => `${px[0]} in "${m[1].trim()}"`),
  );
  assert.deepEqual(raw, []);
});

test("every custom property a rule reads is defined or set at runtime", () => {
  const missing = [...css.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]).filter((v) => !defined.has(v) && !RUNTIME.has(v));
  assert.deepEqual([...new Set(missing)], []);
});

test("the review colour is a token with a twin in each palette, not a fallback literal", () => {
  assert.ok(!/var\(--review,/.test(css));
  assert.match(palette("dark"), /--review:\s*#/);
  assert.match(palette("light"), /--review:\s*#/);
});

test("in dark mode a board card sits a step above its lane, and the lane above the page", () => {
  const bg = (sel: string) => css.match(new RegExp(`\\n${sel.replace(/[.]/g, "\\.")} \\{[^}]*background: var\\((--[\\w-]+)\\)`))?.[1];
  assert.equal(bg(".lanes.wide .lane"), "--lane-bg");
  assert.match(palette("dark"), /--lane-bg: var\(--elev-1\);/);
  assert.equal(bg(".task"), "--elev-2");
});

test("the board's new-task form and the thread composer do not share a class", () => {
  // They once did, and the board's flex-wrap rule shrank the thread's message field to a pill.
  assert.equal(css.match(/(^|\n)\.composer \{/g)?.length, 1);
  const board = readFileSync(new URL("../src/board/Board.tsx", import.meta.url), "utf8");
  assert.ok(!/className="composer"/.test(board));
});

test("the empty Threads and People detail panes share one titled empty state", () => {
  for (const f of ["threads/Threads.tsx", "people/People.tsx"]) {
    const src = readFileSync(new URL(`../src/${f}`, import.meta.url), "utf8");
    assert.match(src, /className="pane-empty">\s*<span className="pane-empty-mark" \/>\s*<strong>/, f);
  }
  assert.match(css, /\.pane-empty \{/);
  assert.ok(!/member-detail[^{]*\{[^}]*dashed/.test(css));
});

test("four board lanes fit a 1024px window", () => {
  const m = css.match(/\.lanes\.wide \{ grid-template-columns: repeat\(4, minmax\((\d+)px, 1fr\)\)/);
  assert.ok(m, "wide lanes rule");
  // 1024 window - 240 sidebar - 36 padding = 748; four lanes and three 12px gaps must fit.
  assert.ok(Number(m![1]) * 4 + 36 <= 748, `lane min ${m![1]}px is too wide`);
});

test("card meta wraps instead of clipping in a narrow lane", () => {
  assert.match(css, /\.task-meta \{[^}]*flex-wrap: wrap/);
});

test("disclosure summaries get a rounded focus ring", () => {
  assert.match(css, /summary:focus-visible \{ border-radius: var\(--r-xs\); \}/);
});

test("the palette's search field has no clipped outline of its own", () => {
  // The global input ring ran past the palette's top edge; the open dialog and caret already show focus.
  assert.match(css, /\.palette-input:focus-visible \{ outline: none; \}/);
});

test("one button style: .button and .btn share the height and the pill shape", () => {
  const rule = (sel: string) => css.match(new RegExp(`\\n\\${sel} \\{([^}]*)\\}`))?.[1] ?? "";
  for (const sel of [".button", ".btn"]) {
    assert.match(rule(sel), /min-height: var\(--btn-h\)/, sel);
    assert.match(rule(sel), /border-radius: var\(--r-pill\)/, sel);
  }
});

test("each theme sets its palette in one block, and the system choice is the script's, not a media query's", () => {
  // A second prefers-color-scheme block once fought the data-theme one; the
  // script resolves System to dark or light before the first paint.
  assert.equal(css.match(/:root, :root\[data-theme="dark"\] \{/g)?.length, 1);
  assert.equal(css.match(/:root\[data-theme="light"\] \{/g)?.length, 1);
  assert.doesNotMatch(css, /prefers-color-scheme/);
  assert.match(palette("dark"), /color-scheme: dark;/);
  assert.match(palette("light"), /color-scheme: light;/);
});

test("both palettes set the same tokens, so neither theme falls back to the other's colour", () => {
  const names = (scheme: "dark" | "light") => [...palette(scheme).matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]).sort();
  assert.deepEqual(names("light"), names("dark"));
});

test("no colour is written outside the token blocks", () => {
  // A hard-coded colour does not change with the theme: the swarm tile's
  // bright steel stayed bright in dark until every one moved into a token.
  const rules = css.replace(/(?<=^|[{};])\s*:root[^{]*\{[^}]*\}/g, "");
  const literals = [...rules.matchAll(/#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|hwb|lab|lch|oklab|oklch)\(|\b(?:white|black)\b(?!-)/gi)].map((m) => {
    const line = rules.slice(0, m.index).split("\n").length;
    return `${m[0]} near "${rules.split("\n")[line - 1].trim().slice(0, 80)}"`;
  });
  assert.deepEqual(literals, []);
});

test("Activity's Pause and Resume use the shared button style", () => {
  const act = readFileSync(new URL("../src/activity/Activity.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(act, /<button onClick=/);
});

test("Board controls share one height: filters match the lane and view pills", () => {
  const rule = (sel: string) => css.match(new RegExp(`\\n${sel.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")} \\{([^}]*)\\}`))?.[1] ?? "";
  assert.match(css, /--control-h: 44px;/);
  for (const sel of [".filter-q input", ".filter-pick select", ".filter-clear", ".lane-pick button", ".view-pick .chip-btn"]) {
    assert.match(rule(sel), /min-height: var\(--control-h\)/, sel);
  }
});

test("the Board health strip sizes its text from the type scale", () => {
  for (const sel of [".health-chip", ".stale-banner"]) {
    const rule = css.match(new RegExp(`\\n\\${sel} \\{([^}]*)\\}`))?.[1] ?? "";
    assert.match(rule, /font-size: var\(--text-base\)/, sel);
  }
});
