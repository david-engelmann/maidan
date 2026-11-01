import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";

// Every class and id a console stylesheet names is used by the markup, so a
// rename or removal in a component takes its CSS with it. Ported from the
// classic board's ui_css_dead contract.

const root = new URL("..", import.meta.url).pathname;

function walk(dir: string, keep: (p: string) => boolean): string[] {
  return readdirSync(dir).flatMap((n) => {
    const p = join(dir, n);
    return statSync(p).isDirectory() ? walk(p, keep) : keep(p) ? [p] : [];
  }).sort();
}

export function selectors(css: string): string[] {
  const out: string[] = [];
  let buf = "";
  for (const c of css.replace(/\/\*[\s\S]*?\*\//g, "")) {
    if (c === "{") {
      const p = buf.trim();
      if (p && !p.startsWith("@") && !/^(from|to|\d+%)/.test(p)) out.push(p);
      buf = "";
    } else if (c === ";" || c === "}") buf = "";
    else buf += c;
  }
  return out;
}

export function names(selector: string): string[] {
  const text = selector.replace(/\[[^\]]*\]/g, "").replace(/\([^()]*\)/g, (m) => m.replace(/[^.#\w\s-]/g, " "));
  return [...text.matchAll(/([.#])([A-Za-z_-][\w-]*)/g)].map((m) => m[1] + m[2]);
}

export function used(name: string, src: string): boolean {
  const word = new RegExp(`(^|[^\\w-])${name.replace(/[-]/g, "\\-")}($|[^\\w-])`);
  if (word.test(src)) return true;
  // Built from a literal prefix: `kind-${k}` or "kind-" + k.
  return [...name.matchAll(/-/g)].some((m) => {
    const prefix = name.slice(0, (m.index ?? 0) + 1);
    return src.includes(`${prefix}\${`) || src.includes(`${prefix}" +`) || src.includes(`${prefix}' +`);
  });
}

test("the scan finds names and skips values", () => {
  const found = selectors("/* .gone */ @media (max-width: 720px) { #board .card:not(.mine) > a[href=\"#x.y\"] { color: #fff; margin: .5rem; } }").flatMap(names);
  assert.deepEqual(found, ["#board", ".card", ".mine"]);
  assert.ok(used("kind-open_gate", "className={`need-kind kind-${k}`}"));
  assert.ok(!used("mine", "const mineCount = 1;"));
});

test("every class and id in the console's CSS is used", () => {
  const src = [join(root, "index.html"), ...walk(join(root, "src"), (p) => /\.(tsx?)$/.test(p))]
    .map((p) => readFileSync(p, "utf8")).join("\n");
  const sheets = walk(join(root, "src"), (p) => p.endsWith(".css"));
  const named = sheets.flatMap((p) => selectors(readFileSync(p, "utf8")).flatMap(names));
  assert.ok(named.length > 100, `only ${named.length} names found; the scan may be broken`);
  const dead = [...new Set(named.filter((n) => !used(n.slice(1), src)))].sort();
  assert.deepEqual(dead, [], "styles name classes or ids nothing in web/console uses; delete the rules or fix the names");
});
