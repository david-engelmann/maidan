import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { built } from "../try-src/build.mjs";
import { LEVELS, PERIOD, START, STILL } from "../try/gathering.js";
import { SHADE, SIDE, createBackdrop } from "../try/backdrop.js";

// The gathering behind the try page's hero: the scene the page carries
// (try/gathering.js, built from try-src/gathering.ts) and the wiring around
// it (try/backdrop.js). The scene itself draws only with a GPU; the
// Playwright spec (ui-tests/tests/hosted-pages.spec.ts) drives it in a
// browser. These hold what can be held without one.

const read = (rel) => readFileSync(new URL(rel, import.meta.url), "utf8");
const page = read("../try/index.html");
const scene = read("../try/gathering.js");
const source = read("../try-src/gathering.ts");

test("try/gathering.js is the build of try-src/gathering.ts, so neither is edited alone", () => {
  assert.equal(scene, built(), "run npm run build:try in deploy/cloudflare");
  assert.match(scene, /^\/\/ Built from deploy\/cloudflare\/try-src\/gathering\.ts/);
});

test("the scene's source uses only types that erase, which is all the build handles", () => {
  assert.doesNotMatch(source, /^\s*(export\s+)?(const\s+)?enum\s/m);
  assert.doesNotMatch(source, /^\s*(export\s+)?namespace\s/m);
  assert.doesNotMatch(source, /constructor\s*\(\s*(public|private|protected|readonly)\s/);
});

test("the scene loads nothing, calls nothing and keeps nothing", () => {
  assert.doesNotMatch(scene, /^\s*import\s|\bimport\(/m);
  assert.doesNotMatch(scene, /\bfetch\(|XMLHttpRequest|WebSocket|https?:\/\//);
  assert.doesNotMatch(scene, /localStorage|sessionStorage|document\.cookie/);
  assert.doesNotMatch(scene, /\beval\(|new Function\(/);
});

test("the scene never draws past a pixel ratio of 2, and its moments are within one cycle", () => {
  assert.ok(Math.max(...LEVELS.map((l) => l.dpr)) <= 2);
  assert.ok(LEVELS.every((l, i) => i === 0 || l.dpr <= LEVELS[i - 1].dpr), "each quality level is cheaper than the one before");
  for (const t of [START, STILL]) assert.ok(t >= 0 && t < PERIOD);
});

test("the page loads the backdrop as a module, and the backdrop loads the scene only when it starts", () => {
  assert.match(page, /<script type="module" src="\/backdrop\.js"><\/script>/);
  const backdrop = read("../try/backdrop.js");
  assert.doesNotMatch(backdrop, /^import /m, "nothing is loaded up front");
  assert.match(backdrop, /import\("\.\/gathering\.js"\)/);
});

test("the poster, the canvas and the control are on the page, and the picture is hidden from assistive tech", () => {
  assert.match(page, /<div id="gather" class="gather" data-state="poster" aria-hidden="true">/);
  assert.match(page, /<canvas class="gather__canvas"><\/canvas>/);
  assert.match(page, /<svg class="gather__poster"/);
  assert.match(page, /<button type="button" id="gather-toggle" class="gather-toggle" aria-label="Pause the animation" title="Pause the animation" hidden>/);
});

test("the light clears behind every block of words over the picture, within what the scene can take", () => {
  // The scene clears behind at most 16 blocks (SHADES in try-src/gathering.ts)
  assert.match(source, /const SHADES = 16;/);
  const blocks = (section) => {
    const open = page.indexOf(`<section ${section}`);
    const body = page.slice(page.indexOf(">", open) + 1, page.indexOf("</section>", open));
    let depth = 0;
    let count = 0;
    for (const m of body.matchAll(/<(\/?)([a-z0-9]+)\b[^>]*?(\/?)>/g)) {
      if (m[3] === "/" || ["br", "img", "input"].includes(m[2])) continue;
      if (m[1]) depth--;
      else if (depth++ === 0) count++;
    }
    return count;
  };
  const hero = blocks('class="hero"');
  const ready = blocks('id="ready"');
  assert.ok(hero >= 5 && ready >= 5, `found ${hero} and ${ready} blocks`);
  // The header's brand and link, and every child of both sections
  assert.ok(2 + hero + ready <= 16, `${2 + hero + ready} blocks to clear`);
  assert.equal(SHADE, ".top .brand, .top .top-link, .hero > *, .ready > *");
  assert.equal(SIDE, ".hero > *");
});

// A stand-in for the document and window the backdrop needs.
function world({ ready = "complete", reduced = false, theme = "dark", idle = true } = {}) {
  const attrs = new Map([["data-theme", theme]]);
  const listeners = new Map();
  const host = { dataset: { state: "poster" }, querySelector: (s) => (s === "canvas" ? { canvas: true } : s === ".gather__mark" ? { mark: true } : null) };
  const toggle = {
    hidden: true,
    title: "",
    dataset: {},
    attrs: new Map(),
    setAttribute(k, v) {
      this.attrs.set(k, v);
    },
    addEventListener(type, fn) {
      this.click = fn;
    },
  };
  const motion = {
    matches: reduced,
    addEventListener(type, fn) {
      this.change = fn;
    },
  };
  const later = [];
  const observers = [];
  const doc = {
    readyState: ready,
    documentElement: { getAttribute: (k) => attrs.get(k) ?? null },
    getElementById: (id) => (id === "gather" ? host : id === "gather-toggle" ? toggle : null),
    querySelectorAll: () => [],
  };
  const win = {
    matchMedia: () => motion,
    addEventListener(type, fn) {
      listeners.set(type, fn);
    },
    setTimeout(fn) {
      later.push(fn);
    },
    MutationObserver: class {
      constructor(fn) {
        observers.push(fn);
      }
      observe() {}
    },
  };
  if (idle) win.requestIdleCallback = (fn) => later.push(fn);
  return {
    doc,
    win,
    host,
    toggle,
    motion,
    attrs,
    listeners,
    observers,
    async idle() {
      for (const fn of later.splice(0)) fn();
      await new Promise((r) => setTimeout(r, 0));
    },
  };
}

// A scene that records what it was asked, standing in for gathering.js.
function fakeScene(result = "running") {
  const calls = { mount: [], update: [] };
  return {
    calls,
    load: async () => ({
      mount(m) {
        calls.mount.push(m);
        if (result === null) return null;
        m.onState(result);
        return { update: (next) => calls.update.push(next), destroy() {} };
      },
    }),
  };
}

test("the scene starts once the page has loaded and gone idle, never before", async () => {
  const w = world({ ready: "loading" });
  const s = fakeScene();
  createBackdrop(w.doc, w.win, s.load);
  await w.idle();
  assert.equal(s.calls.mount.length, 0, "not while the page is still loading");
  w.listeners.get("load")();
  await w.idle();
  assert.equal(s.calls.mount.length, 1);
  assert.equal(w.host.dataset.state, "running");
});

test("without an idle callback (Safari) a short timeout starts it instead", async () => {
  const w = world({ idle: false });
  const s = fakeScene();
  createBackdrop(w.doc, w.win, s.load);
  await w.idle();
  assert.equal(s.calls.mount.length, 1);
});

test("the scene is told the page's theme and reduced motion, and follows changes to both", async () => {
  const w = world({ theme: "light", reduced: true });
  const s = fakeScene("still");
  createBackdrop(w.doc, w.win, s.load);
  await w.idle();
  assert.deepEqual(s.calls.mount[0].initial, { light: true, still: true, paused: false });
  w.attrs.set("data-theme", "dark");
  w.observers[0]();
  w.motion.matches = false;
  w.motion.change();
  assert.deepEqual(s.calls.update.at(-1), { light: false, still: false, paused: false });
});

test("the control shows only while the picture moves, and says what a press will do", async () => {
  const w = world();
  const s = fakeScene();
  createBackdrop(w.doc, w.win, s.load);
  assert.equal(w.toggle.hidden, true, "hidden while the poster stands");
  await w.idle();
  assert.equal(w.toggle.hidden, false);
  assert.equal(w.toggle.attrs.get("aria-label"), "Pause the animation");
  w.toggle.click();
  assert.deepEqual(s.calls.update.at(-1), { paused: true });
  assert.equal(w.toggle.attrs.get("aria-label"), "Play the animation");
  assert.equal(w.toggle.title, "Play the animation");
  assert.equal(w.toggle.dataset.paused, "true");
  s.calls.mount[0].onState("paused");
  assert.equal(w.toggle.hidden, false, "a paused picture can be played again");
  for (const still of ["still", "software", "fallback"]) {
    s.calls.mount[0].onState(still);
    assert.equal(w.toggle.hidden, true, `nothing to stop when ${still}`);
  }
});

test("where the scene cannot load or cannot draw, the poster stays and the control stays hidden", async () => {
  for (const load of [async () => Promise.reject(new Error("offline")), fakeScene(null).load]) {
    const w = world();
    createBackdrop(w.doc, w.win, load);
    await w.idle();
    assert.equal(w.host.dataset.state, "fallback");
    assert.equal(w.toggle.hidden, true);
  }
  // A renderer on the CPU proves the scene works, then leaves the poster up
  const w = world();
  createBackdrop(w.doc, w.win, fakeScene("software").load);
  await w.idle();
  assert.equal(w.host.dataset.state, "software");
  assert.equal(w.toggle.hidden, true);
});

test("the poster gives way only to a scene that drew, and never under a software renderer", () => {
  const css = read("../try/try.css").replace(/\/\*[\s\S]*?\*\//g, "");
  const shows = [...css.matchAll(/\.gather:is\(([^)]*)\) \.gather__canvas \{ opacity: 1; \}/g)].map((m) => m[1]);
  assert.equal(shows.length, 1);
  const states = [...shows[0].matchAll(/data-state="([a-z]+)"/g)].map((m) => m[1]).sort();
  assert.deepEqual(states, ["away", "paused", "running", "still"]);
  assert.match(css, /\.gather__canvas \{[^}]*opacity: 0;/);
});
