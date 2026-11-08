import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// WCAG AA for the pairs every screen is made of, in both themes: body and
// muted text on each surface (4.5:1), status ink used as text (4.5:1), and the
// status dots that carry meaning as a graphic (1.4.11, 3:1).

const css = readFileSync(new URL("../src/styles.css", import.meta.url), "utf8").replace(/\/\*[\s\S]*?\*\//g, "");

/** Top-level rules as [selector, body], skipping every at-rule block (media, print, keyframes). */
export function topLevelRules(source: string): Array<[string, string]> {
  const out: Array<[string, string]> = [];
  let depth = 0;
  let head = "";
  let body = "";
  let atRule = false;
  for (const c of source) {
    if (c === "{") {
      if (depth === 0) {
        atRule = head.trim().startsWith("@");
        body = "";
      } else if (depth === 1 && !atRule) body += c;
      depth++;
    } else if (c === "}") {
      depth--;
      if (depth === 0) {
        if (!atRule) out.push([head.trim(), body]);
        head = "";
      }
    } else if (depth === 0) {
      head += c;
      if (c === ";") head = "";
    } else if (!atRule) body += c;
  }
  return out;
}

/** Whether a rule's selector list reaches <html data-theme={scheme}>. */
function reachesRoot(selector: string, scheme: "light" | "dark"): boolean {
  return selector.split(",").some((s) => {
    const part = s.trim();
    return part === ":root" || part === `:root[data-theme="${scheme}"]`;
  });
}

// The cascade on <html data-theme={scheme}>: every top-level :root rule that
// reaches it, in source order, the later one winning.
export function tokens(scheme: "light" | "dark"): Map<string, string> {
  const out = new Map<string, string>();
  for (const [selector, body] of topLevelRules(css)) {
    if (!reachesRoot(selector, scheme)) continue;
    for (const d of body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)) out.set(d[1], d[2].trim());
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

const SURFACES = ["--paper", "--paper-2", "--card", "--elev-1", "--elev-2", "--elev-3"];

function holds(scheme: "light" | "dark", fgs: string[], bgs: string[], min: number): string[] {
  const t = tokens(scheme);
  return fgs.flatMap((fg) =>
    bgs.map((bg) => [fg, bg, ratio(hex(t, fg), hex(t, bg))] as const).filter(([, , r]) => r < min).map(([f, b, r]) => `${f} on ${b} is ${r.toFixed(2)}:1`),
  );
}

for (const scheme of ["light", "dark"] as const) {
  test(`${scheme}: text and muted text reach 4.5:1 on every surface`, () => {
    assert.deepEqual(holds(scheme, ["--ink", "--muted"], SURFACES, 4.5), []);
  });

  test(`${scheme}: status ink used as text reaches 4.5:1 on every surface`, () => {
    assert.deepEqual(holds(scheme, ["--signal-ink", "--ok", "--danger", "--review", "--vote-yes", "--vote-no"], SURFACES, 4.5), []);
  });

  test(`${scheme}: idle status dots reach 3:1 on every surface`, () => {
    assert.deepEqual(holds(scheme, ["--idle"], SURFACES, 3), []);
  });

  test(`${scheme}: the primary action, toasts and avatars keep their label at 4.5:1`, () => {
    assert.deepEqual(
      [
        ...holds(scheme, ["--primary-ink"], ["--primary-flat"], 4.5),
        ...holds(scheme, ["--toast-ink"], ["--toast-bg"], 4.5),
        ...holds(scheme, ["--avatar-agent-ink"], ["--avatar-agent"], 4.5),
        ...holds(scheme, ["--avatar-human-ink"], ["--avatar-human"], 4.5),
      ],
      [],
    );
  });
}

test("dark surfaces step up: page, then each elevation lighter than the last", () => {
  const t = tokens("dark");
  const steps = ["--paper", "--paper-2", "--elev-1", "--elev-2", "--elev-3"].map((n) => hex(t, n));
  for (let i = 1; i < steps.length; i++) assert.ok(ratio(steps[i], "#000000") > ratio(steps[i - 1], "#000000"), `${steps[i]} is not above ${steps[i - 1]}`);
});

test("the offline pip, the open state dot and the live dot use --idle", () => {
  for (const sel of [".live .dot", ".pip", ".state-dot"]) {
    const rule = css.match(new RegExp(`(^|\\n)${sel.replace(/\./g, "\\.")}\\s*\\{([^}]*)\\}`));
    assert.ok(rule, sel);
    assert.match(rule[2], /background:\s*var\(--idle\)/, sel);
  }
});

test("print keeps status and vote ink readable on white paper", () => {
  // Printers drop backgrounds; the dark theme's bright status ink on white was 1.9:1.
  const at = css.indexOf("@media print");
  const body = css.slice(css.indexOf("{", css.indexOf(":root", at)) + 1, css.indexOf("}", css.indexOf(":root", at)));
  const t = new Map([...body.matchAll(/(--[\w-]+)\s*:\s*([^;]+);/g)].map((d) => [d[1], d[2].trim()] as [string, string]));
  const short = ["--ink", "--muted", "--signal-ink", "--ok", "--danger", "--review", "--vote-yes", "--vote-no"]
    .map((n) => [n, t.get(n) ?? "missing"] as const)
    .filter(([, v]) => !/^#[0-9a-f]{3,6}$/i.test(v) || ratio(v.length === 4 ? `#${[...v.slice(1)].map((c) => c + c).join("")}` : v, "#ffffff") < 4.5)
    .map(([n, v]) => `${n}: ${v}`);
  assert.deepEqual(short, []);
});
