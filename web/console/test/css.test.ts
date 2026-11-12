import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { sheet } from "./sheet.ts";

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

/** The value the last rule for exactly `selector` gives `prop`, across the console's sheets. */
function lastValue(selector: string, prop: string): string | null {
  const css = walk(join(root, "src"), (p) => p.endsWith(".css")).map((p) => readFileSync(p, "utf8")).join("\n").replace(/\/\*[\s\S]*?\*\//g, "");
  let out: string | null = null;
  for (const m of css.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
    if (!m[1].split(",").some((s) => s.trim() === selector)) continue;
    for (const d of m[2].split(";")) {
      const [k, ...v] = d.split(":");
      if (k.trim() === prop) out = v.join(":").trim();
    }
  }
  return out;
}

test("the sign-in card's buttons span the card like its token field", () => {
  // .edit-actions packs to the end elsewhere; on the sign-in grid that put
  // "Sign in" and "Create a workspace" in a narrow column on the right.
  assert.equal(lastValue(".edit-actions", "justify-content"), "flex-end");
  assert.equal(lastValue(".sign-in .edit-actions", "display"), "grid");
  assert.equal(lastValue(".sign-in .edit-actions", "justify-content"), "stretch");
});

test("reduced motion removes animation, transition and smooth scrolling", () => {
  const css = sheet();
  const block = /@media \(prefers-reduced-motion: reduce\) \{\s*\*, \*::before, \*::after \{([^}]+)\}/.exec(css)?.[1] ?? "";
  assert.match(block, /animation:\s*none\s*!important/);
  assert.match(block, /transition:\s*none\s*!important/);
  assert.match(block, /scroll-behavior:\s*auto\s*!important/);
  // The composer that actually animates is .task-composer. A rule aimed at
  // .composer never reached it.
  assert.match(css, /@media \(prefers-reduced-motion: reduce\) \{ \.task-composer \{ animation: none; \} \}/);
  assert.doesNotMatch(css, /\.mode-chip \{ display: none/);
});

test("the skip link sits off screen until it is focused", () => {
  const css = sheet();
  assert.equal(lastValue(".skip-link", "top"), "-100px");
  assert.match(css, /\.skip-link:focus, \.skip-link:focus-visible \{ top: calc\(env\(safe-area-inset-top, 0px\) \+ 8px\); \}/);
  const shell = readFileSync(join(root, "src/shell/Shell.tsx"), "utf8");
  const skip = shell.indexOf('className="skip-link"');
  assert.ok(skip >= 0 && skip < shell.indexOf("<nav") && skip < shell.indexOf("<header"));
  assert.match(shell, />\s*Skip to content\s*</);
  assert.match(shell, /getElementById\("main"\)\?\.focus\(\)/);
});

test("a phone header shows Search and does not drop the credential word", () => {
  const css = sheet();
  assert.match(css, /@media \(max-width: 599px\) \{[\s\S]*\.top \.icon-btn \.btn-label \{ display: inline;/);
  assert.match(css, /@media \(max-width: 599px\) \{[\s\S]*\.top \.live-label \{ position: static;/);
  assert.match(css, /@media \(max-width: 599px\) \{[\s\S]*\.mode-chip \{ display: inline-flex;/);
  const shell = readFileSync(join(root, "src/shell/Shell.tsx"), "utf8");
  assert.match(shell, /className="btn-label">Search</);
  assert.match(shell, /className="live-label"/);
});

test("text hidden for screen readers uses the class the stylesheet defines", () => {
  // An undefined hiding class shows its text: the presence chip printed
  // ", online, free" beside the name.
  assert.equal(lastValue(".visually-hidden", "position"), "absolute");
  const offenders = walk(join(root, "src"), (p) => p.endsWith(".tsx")).filter((p) => /\bsr-only\b/.test(readFileSync(p, "utf8")));
  assert.deepEqual(offenders, []);
});

test("on a phone the board's fields and the channel picker are 16px, so iOS does not zoom", () => {
  const css = sheet();
  const phone = /\/\* Phones: iOS zooms[^*]*\*\/\s*@media \(max-width: 599px\) \{\s*([^{}]+)\{([^}]+)\}/.exec(css);
  assert.ok(phone, "the phone field-size block is missing");
  const sels = phone[1].split(",").map((x) => x.trim());
  for (const s of [".filter-q input", ".filter-pick select", ".composer-field input", ".channel-pick select", ".content .channel-pick select"]) {
    assert.ok(sels.includes(s), `${s} is not raised to 16px on a phone`);
  }
  assert.match(phone[2], /font-size: var\(--text-input\)/);
  assert.match(css, /--text-input: 16px;/);
});

test("at 360px the header title gives way so the header keeps one row", () => {
  const css = sheet();
  assert.match(css, /@media \(max-width: 380px\) \{[\s\S]{0,400}?\.top h1 \{[^}]*flex-basis: 6rem;/);
  assert.equal(lastValue(".top h1", "flex-basis"), "6rem");
});

test("an activity row's channel link is a 44px target, however short the channel name", () => {
  // `#build` drew 38px wide on a phone, under the touch-target floor.
  assert.equal(lastValue(".activity-channel", "min-width"), "44px");
  assert.equal(lastValue(".activity-channel", "min-height"), "44px");
});
