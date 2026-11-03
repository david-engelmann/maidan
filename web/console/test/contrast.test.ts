import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// WCAG 1.4.11: a status dot is a graphic that carries meaning (offline, open),
// so it needs 3:1 against every surface it sits on, in light and in dark.

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

function darkRanges(): Array<[number, number]> {
  const out: Array<[number, number]> = [];
  for (const m of css.matchAll(/@media\s*\(prefers-color-scheme:\s*dark\)\s*\{/g)) {
    let depth = 1;
    let i = (m.index ?? 0) + m[0].length;
    for (; i < css.length && depth > 0; i++) depth += css[i] === "{" ? 1 : css[i] === "}" ? -1 : 0;
    out.push([m.index ?? 0, i]);
  }
  return out;
}

function depthAt(at: number): number {
  let depth = 0;
  for (let i = 0; i < at; i++) depth += css[i] === "{" ? 1 : css[i] === "}" ? -1 : 0;
  return depth;
}

// Light is the top-level :root blocks; dark adds the :root blocks inside a
// prefers-color-scheme: dark query. Print and other media are left out.
export function tokens(scheme: "light" | "dark"): Map<string, string> {
  const dark = darkRanges();
  const out = new Map<string, string>();
  for (const pass of scheme === "light" ? [false] : [false, true]) {
    for (const b of css.matchAll(/:root\s*\{([^}]*)\}/g)) {
      const at = b.index ?? 0;
      const inDark = dark.some(([s, e]) => at > s && at < e);
      if (pass ? !inDark : depthAt(at) !== 0) continue;
      for (const d of b[1].matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out.set(d[1], d[2].trim());
    }
  }
  return out;
}

function hex(t: Map<string, string>, name: string): string {
  let v = t.get(name);
  for (let k = 0; v && v.startsWith("var(") && k < 5; k++) v = t.get(v.slice(4, -1).trim());
  assert.match(v ?? "", /^#[0-9a-f]{6}$/i, `${name} resolves to a hex colour`);
  return v as string;
}

export function ratio(a: string, b: string): number {
  const lum = (h: string) => {
    const [r, g, bl] = [1, 3, 5].map((i) => parseInt(h.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
    return 0.2126 * r + 0.7152 * g + 0.0722 * bl;
  };
  const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

for (const scheme of ["light", "dark"] as const) {
  test(`${scheme}: idle status dots reach 3:1 on every surface`, () => {
    const t = tokens(scheme);
    for (const bg of ["--paper", "--paper-2", "--card", "--elev-2"]) {
      const r = ratio(hex(t, "--idle"), hex(t, bg));
      assert.ok(r >= 3, `--idle on ${bg} is ${r.toFixed(2)}:1`);
    }
  });
}

for (const scheme of ["light", "dark"] as const) {
  test(`${scheme}: vote ink reaches 4.5:1 on every surface a vote sits on`, () => {
    const t = tokens(scheme);
    for (const fg of ["--vote-yes", "--vote-no"]) {
      for (const bg of ["--paper", "--paper-2", "--card", "--elev-2", "--elev-3"]) {
        const r = ratio(hex(t, fg), hex(t, bg));
        assert.ok(r >= 4.5, `${fg} on ${bg} is ${r.toFixed(2)}:1`);
      }
    }
  });
}

test("the offline pip, the open state dot and the live dot use --idle", () => {
  for (const sel of [".live .dot", ".pip", ".state-dot"]) {
    const rule = css.match(new RegExp(`(^|\\n)${sel.replace(/\./g, "\\.")}\\s*\\{([^}]*)\\}`));
    assert.ok(rule, sel);
    assert.match(rule[2], /background:\s*var\(--idle\)/, sel);
  }
});
