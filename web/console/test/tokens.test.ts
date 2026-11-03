import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// The console's surfaces share one radius scale and one palette, so a corner
// or a colour changes in one place and light and dark stay in step.

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");
const defined = new Set([...css.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));
// Set at runtime from script (viewport, keyboard, composer, drag position, font).
const RUNTIME = new Set(["--vv-top", "--vv-h", "--composer-h", "--kb", "--left", "--font"]);

test("every corner above 4px comes from the radius scale", () => {
  const raw = [...css.matchAll(/border-radius:\s*(\d+)px/g)].map((m) => Number(m[1])).filter((n) => n > 4);
  assert.deepEqual(raw, []);
});

test("every custom property a rule reads is defined or set at runtime", () => {
  const missing = [...css.matchAll(/var\((--[\w-]+)/g)].map((m) => m[1]).filter((v) => !defined.has(v) && !RUNTIME.has(v));
  assert.deepEqual([...new Set(missing)], []);
});

test("the review colour is a token with a dark twin, not a fallback literal", () => {
  assert.ok(!/var\(--review,/.test(css));
  const dark = css.slice(css.indexOf("prefers-color-scheme: dark"));
  assert.match(dark, /--review:\s*#/);
});

test("in dark mode a board card sits a step above its lane, and the lane above the page", () => {
  const dark = css.slice(css.lastIndexOf("@media (prefers-color-scheme: dark)"));
  const bg = (sel: string) => dark.match(new RegExp(`${sel.replace(/[.]/g, "\\.")} \\{[^}]*background: var\\((--[\\w-]+)\\)`))?.[1];
  assert.equal(bg(".lanes.wide .lane"), "--elev-1");
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

test("dark mode sets its tokens in one :root block, not two that disagree", () => {
  const blocks = [...css.matchAll(/@media \(prefers-color-scheme: dark\) \{\s*:root \{([^}]*)\}/g)];
  assert.equal(blocks.length, 1);
  assert.match(blocks[0][1], /--elev-1:/);
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
