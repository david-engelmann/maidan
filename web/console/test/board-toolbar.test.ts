import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { sheet } from "./sheet.ts";

// The Board opens on one toolbar row, then the lanes. Before, five rows of
// controls (title, channel and view, filters, presence, health) came before
// the first card, and on a phone the first task started two thirds down.
const board = readFileSync(new URL("../src/board/Board.tsx", import.meta.url), "utf8");
const css = sheet().replace(/\/\*[\s\S]*?\*\//g, "");

function toolbar(): string {
  const open = board.indexOf('<div className="board-bar"');
  const after = board.indexOf("{composing && data.channelId", open);
  assert.ok(open >= 0 && after > open, "the board-bar opens before the composer");
  return board.slice(open, after);
}

test("filters, health, presence, the count and New task all sit inside the one toolbar row", () => {
  const bar = toolbar();
  for (const part of ["<ChannelPick", 'className="view-pick"', "<FilterBar ", 'className="health"', 'className="presence-strip"', 'className="board-summary"', "new-task"]) {
    assert.ok(bar.includes(part), part);
  }
  const rest = board.slice(board.indexOf("{composing && data.channelId"));
  for (const part of ["<FilterBar ", 'className="presence-strip"', 'className="health"']) {
    assert.ok(!rest.slice(0, rest.indexOf("function ")).includes(part), `${part} is a row of its own again`);
  }
});

test("the toolbar names itself for a screen reader", () => {
  assert.match(toolbar(), /<div className="board-bar" role="group" aria-label="Board controls">/);
});

function last(selector: string, prop: string): string | undefined {
  const esc = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const all = [...css.matchAll(new RegExp(`(?:^|\\n|\\}\\s*)${esc}\\s*\\{([^}]*)\\}`, "g"))]
    .map((m) => new RegExp(`(?:^|;|\\s)${prop}:\\s*([^;]+);`).exec(m[1])?.[1]?.trim())
    .filter((v): v is string => v !== undefined);
  return all[all.length - 1];
}

test("wide lanes size to their cards instead of stretching to the bottom of the window", () => {
  assert.equal(last(".lanes.wide", "align-items"), "start");
  assert.equal(last(".lanes.wide", "min-height"), "0");
});

test("the row contains its hidden labels, and the filter group can wrap instead of overflowing its neighbours", () => {
  assert.equal(last(".board-bar", "position"), "relative");
  assert.equal(last(".board-bar .board-filters", "flex-wrap"), "wrap");
});

test("above the phone width, health and presence chips may wrap rather than widen the row", () => {
  const wide = css.slice(0, css.lastIndexOf("@media (max-width: 599px)"));
  assert.doesNotMatch(wide, /\.board-bar \.(health|presence-strip)[^{]*\{[^}]*flex-wrap: nowrap/);
});

test("on a phone the toolbar is one line that scrolls inside itself, New task first", () => {
  const phone = css.slice(css.lastIndexOf("@media (max-width: 599px)"));
  assert.match(phone, /\.board-bar \{ flex-wrap: nowrap; overflow-x: auto;/);
  assert.match(phone, /\.board-bar \.new-task \{ order: -1;/);
  assert.match(phone, /\.board-bar \.health, \.board-bar \.presence-strip, \.board-bar \.presence-strip ul \{ flex-wrap: nowrap; \}/);
});
