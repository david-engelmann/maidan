import { test } from "node:test";
import assert from "node:assert/strict";
import { sheet } from "./sheet.ts";

// The wide board is four lanes. Whatever the last `.lanes.wide` rule says,
// its smallest track times four plus three gaps must fit the content column,
// or Done is pushed past the right edge at 1024px.

const css = sheet().replace(/\/\*[\s\S]*?\*\//g, "");
const space3 = 12;

function lastColumns(): string {
  const all = [...css.matchAll(/(?:^|\n)\.lanes\.wide\s*\{([^}]*)\}/g)].map((m) => m[1].match(/grid-template-columns:\s*([^;]+);/)?.[1]).filter((v): v is string => !!v);
  assert.ok(all.length, "a .lanes.wide rule sets grid-template-columns");
  return all[all.length - 1];
}

/** The smallest a track can be in a column `width` px wide. */
export function minTrack(columns: string, width: number): number {
  const m = columns.match(/^repeat\(4,\s*minmax\((.+),\s*1fr\)\)$/);
  assert.ok(m, `four minmax tracks: ${columns}`);
  const expr = m[1].replace(/var\(--space-3\)/g, `${space3}px`).replace(/100%/g, `${width}px`).replace(/(\d+(?:\.\d+)?)px/g, "$1").replace(/\bcalc\(/g, "(").replace(/\bmin\(/g, "Math.min(");
  assert.match(expr, /^[\d\s+\-*/().,Mathmin]+$/, expr);
  return Number(new Function(`return ${expr}`)());
}

for (const [viewport, column] of [[1024, 1024 - 240 - 2 * 18], [1280, 1280 - 240 - 2 * 18]] as const) {
  test(`four lanes fit a ${viewport}px window`, () => {
    const need = 4 * minTrack(lastColumns(), column) + 3 * space3;
    assert.ok(need <= column, `lanes need ${need}px, column is ${column}px`);
  });
}

test("lanes keep 220px when there is room", () => {
  assert.equal(minTrack(lastColumns(), 1440 - 240 - 36), 220);
});
