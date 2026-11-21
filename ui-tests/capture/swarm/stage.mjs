// The split-screen recording: the real console above, three agents' terminals
// below, one take against a real server.
//
//   node stage.mjs <out dir>            record frames (two streams) for scripts/make-swarm-clip.sh
//   STAGE_STILLS=1 node stage.mjs <out> save a still of both halves at each beat
//
// Every terminal line is a call the script made and what the server answered.
// The agents are scripted; the advice is the decision model's own answer.
import { chromium } from "@playwright/test";
import { mkdirSync, rmSync, writeFileSync } from "node:fs";
import { writeFile } from "node:fs/promises";
import { join } from "node:path";
import { ADMIN, O, cast, claimNext, events, finished, mcp, pause, rest } from "./lib.mjs";

const OUT = process.argv[2];
const STILLS = process.env.STAGE_STILLS === "1";
// The console in the page's theme; the terminals are dark in both.
const THEME = process.env.STAGE_THEME === "light" ? "light" : "dark";
const W = 1280;
const CONSOLE_H = Number(process.env.STAGE_CONSOLE_H ?? 540);
const TERM_H = Number(process.env.STAGE_TERM_H ?? 320);
const SCALE = 1.5;
mkdirSync(OUT, { recursive: true });

// ------------------------------------------------------------ the terminals

const TERMINALS = /* html */ `<!doctype html><meta charset="utf-8"><title>agents</title>
<style>
  :root { color-scheme: dark; --bg: #0b0b0d; --pane: #121214; --line: #26262b; --ink: #ece8e1; --muted: #8d897f; --dim: #5d5a54;
    --signal: #ff6a35; --green: #44d19a; --amber: #f2b441; --red: #f0706a; }
  * { box-sizing: border-box; }
  html, body { margin: 0; height: 100%; background: var(--bg); color: var(--ink); overflow: hidden;
    font: 13px/1.5 ui-monospace, "SF Mono", Menlo, monospace; -webkit-font-smoothing: antialiased; }
  body { display: grid; grid-template-rows: 34px 1fr; }
  #events { display: flex; align-items: center; gap: 12px; padding: 0 16px; border-top: 1px solid var(--line); border-bottom: 1px solid var(--line);
    background: #0e0e10; white-space: nowrap; overflow: hidden; }
  #events b { font-weight: 600; font-size: 11px; letter-spacing: 0.1em; color: var(--muted); }
  #events .dot { inline-size: 7px; block-size: 7px; border-radius: 50%; background: var(--signal); opacity: 0.35; transition: opacity 0.5s; }
  #events .dot.on { opacity: 1; transition: none; box-shadow: 0 0 0 4px rgb(255 106 53 / 0.18); }
  #events .ev { color: var(--muted); transition: color 0.8s; }
  #events .ev.new { color: var(--ink); transition: none; }
  #events .ev i { font-style: normal; color: var(--dim); }
  #events .ev + .ev::before { content: "·"; color: var(--dim); margin-right: 12px; }
  #panes { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 12px; padding: 12px; min-height: 0; }
  .pane { display: grid; grid-template-rows: 30px 1fr; min-height: 0; background: var(--pane); border: 1px solid var(--line); border-radius: 10px; overflow: hidden; }
  .pane header { display: flex; align-items: center; gap: 8px; padding: 0 12px; border-bottom: 1px solid var(--line); font-size: 12px; color: var(--ink); }
  .pane header .who { font-weight: 600; }
  .pane header .via { margin-left: auto; color: var(--dim); font-size: 11px; }
  .pane header .lamp { inline-size: 8px; block-size: 8px; border-radius: 50%; background: var(--dim); }
  .pane.busy header .lamp { background: var(--green); }
  .pane .body { padding: 8px 12px 10px; display: flex; flex-direction: column; justify-content: flex-start; overflow: hidden; }
  .ln { white-space: pre; overflow: hidden; text-overflow: ellipsis; }
  .ln.call::before { content: "● "; color: var(--signal); }
  .ln.call.wait::before { color: var(--dim); }
  .ln.ans { color: var(--muted); }
  .ln.ans::before { content: "  ⎿ "; color: var(--dim); }
  .ln.note { color: var(--dim); }
  .ln .ok { color: var(--green); } .ln .warn { color: var(--amber); } .ln .bad { color: var(--red); } .ln .hi { color: var(--ink); }
  .ln.call.pending::after { content: " …"; color: var(--dim); }
</style>
<div id="events"><b>EVENTS</b><span class="dot"></span></div>
<div id="panes"></div>
<script>
  const esc = (s) => s.replace(/[&<>]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;" }[c]));
  // [[class|text]] marks a span; everything else is plain text.
  const rich = (s) => esc(s).replace(/\\[\\[(\\w+)\\|([^\\]]*)\\]\\]/g, '<span class="$1">$2</span>');
  window.stage = {
    pane(id, who) {
      const el = document.createElement("section");
      el.className = "pane"; el.id = id;
      el.innerHTML = '<header><span class="lamp"></span><span class="who">' + esc(who) + '</span><span class="via">agent · MCP</span></header><div class="body"></div>';
      document.getElementById("panes").appendChild(el);
    },
    line(id, kind, text) {
      const body = document.querySelector("#" + id + " .body");
      const ln = document.createElement("div");
      ln.className = "ln " + kind; ln.innerHTML = rich(text);
      body.appendChild(ln);
      while (body.children.length > 11) body.firstChild.remove();
      return body.children.length - 1;
    },
    settle(id) { const p = document.querySelector("#" + id + " .ln.pending"); if (p) p.classList.remove("pending"); },
    busy(id, on) { document.getElementById(id).classList.toggle("busy", on); },
    event(text) {
      const bar = document.getElementById("events");
      const ev = document.createElement("span");
      ev.className = "ev new"; ev.innerHTML = rich(text);
      bar.insertBefore(ev, bar.children[2] ?? null);
      while (bar.children.length > 6) bar.lastChild.remove();
      const dot = bar.querySelector(".dot"); dot.classList.add("on");
      setTimeout(() => { dot.classList.remove("on"); ev.classList.remove("new"); }, 700);
    },
  };
</script>`;

// ------------------------------------------------------------------ capture

class Recorder {
  constructor(page, dir, width, height) { Object.assign(this, { page, dir, width, height, frames: [], writes: [], running: false }); }
  async start() {
    rmSync(this.dir, { recursive: true, force: true });
    mkdirSync(this.dir, { recursive: true });
    const cdp = await this.page.context().newCDPSession(this.page);
    this.running = true;
    this.loop = (async () => {
      while (this.running) {
        const at = Date.now() / 1000;
        const shot = await cdp.send("Page.captureScreenshot", { format: "png", optimizeForSpeed: true, clip: { x: 0, y: 0, width: this.width, height: this.height, scale: SCALE } });
        const file = `${String(this.frames.length).padStart(5, "0")}.png`;
        this.frames.push({ file, at });
        this.writes.push(writeFile(join(this.dir, file), Buffer.from(shot.data, "base64")));
      }
      await cdp.detach();
    })();
  }
  async stop(t0, end) {
    this.running = false;
    await this.loop;
    await Promise.all(this.writes);
    // Each frame lasts until the next was taken; both streams start at t0.
    const lines = ["ffconcat version 1.0"];
    const first = this.frames[0];
    if (first.at > t0) lines.push(`file '${first.file}'`, `duration ${(first.at - t0).toFixed(4)}`);
    this.frames.forEach((f, i) => {
      const next = i + 1 < this.frames.length ? this.frames[i + 1].at : end;
      lines.push(`file '${f.file}'`, `duration ${Math.max(0.001, next - f.at).toFixed(4)}`);
    });
    lines.push(`file '${this.frames.at(-1).file}'`);
    writeFileSync(join(this.dir, "frames.ffconcat"), lines.join("\n") + "\n");
    // A screenshot that hung leaves one frame on screen for seconds while the
    // other half moves on: that take is not one to keep.
    const gaps = this.frames.map((f, i) => (i + 1 < this.frames.length ? this.frames[i + 1].at : end) - f.at);
    const worst = Math.max(...gaps);
    if (worst > 1.5) throw new Error(`a frame of ${this.dir} stayed up for ${worst.toFixed(1)} s: the capture stalled, take it again`);
    return this.frames.length;
  }
}

// Headless Chromium paints no pointer; this draws the person's.
const CURSOR = () => {
  const install = () => {
    if (document.getElementById("motion-cursor")) return;
    const c = document.createElement("div");
    c.id = "motion-cursor";
    c.innerHTML = '<svg width="24" height="24" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 3l13.5 8.6-6.2 1.3-3.4 5.6z" fill="#fff" stroke="#16130f" stroke-width="1.6" stroke-linejoin="round"/></svg>';
    c.style.cssText = "position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;transform:translate(-80px,-80px);filter:drop-shadow(0 1px 2px rgba(0,0,0,.35))";
    document.documentElement.appendChild(c);
    let x = -80, y = -80, s = 1;
    const place = () => (c.style.transform = `translate(${x - 5}px, ${y - 3}px) scale(${s})`);
    addEventListener("mousemove", (e) => { x = e.clientX; y = e.clientY; place(); }, true);
    addEventListener("mousedown", () => { s = 0.82; place(); }, true);
    addEventListener("mouseup", () => { s = 1; place(); }, true);
  };
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", install);
  else install();
};
let mouse = { x: 1180, y: CONSOLE_H - 40 };
async function glide(page, x, y, ms = 600) {
  const from = { ...mouse };
  const t0 = Date.now();
  for (;;) {
    const t = Math.min(1, (Date.now() - t0) / ms);
    const e = t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2;
    await page.mouse.move(from.x + (x - from.x) * e, from.y + (y - from.y) * e);
    if (t >= 1) break;
    await page.waitForTimeout(16);
  }
  mouse = { x, y };
}
async function press(page, target, ms = 600) {
  await target.scrollIntoViewIfNeeded();
  const box = await target.boundingBox();
  if (!box) throw new Error("target has no box");
  await glide(page, box.x + box.width / 2, box.y + box.height / 2, ms);
  await page.waitForTimeout(140);
  await page.mouse.down();
  await page.waitForTimeout(90);
  await page.mouse.up();
}

// -------------------------------------------------------------------- scene

const s = await cast();
// The board the take opens on: two tasks already done, one in progress.
await finished(s, s.codex, s.claude, "Fix the flaky login test on Safari", "The test raced the redirect. It now waits for the session cookie before it asserts.", { status: "changed", summary: "The login test waits for the session cookie; 40 of 40 WebKit runs pass." });
await finished(s, s.cursor, s.claude, "Update the onboarding checklist", "Checklist now matches the new sign-in flow.", { status: "done", summary: "Onboarding checklist matches the new sign-in flow." });

const browser = await chromium.launch({ args: ["--force-color-profile=srgb", "--hide-scrollbars"] });
const ctx = await browser.newContext({ viewport: { width: W, height: CONSOLE_H }, deviceScaleFactor: 2, colorScheme: THEME, locale: "en-US", timezoneId: "America/New_York", serviceWorkers: "block" });
const board = await ctx.newPage();
await board.addInitScript(CURSOR);
const signedIn = await ctx.request.post(O + "/auth/session/from-token", { headers: { authorization: `Bearer ${ADMIN}`, accept: "application/json" } });
if (!signedIn.ok()) throw new Error(`console sign-in answered ${signedIn.status()}`);
await board.addInitScript(([theme]) => { try { localStorage.setItem("maidan_console_theme", theme); } catch {} }, [THEME]);
await board.goto(O + "/ui/#/board");
await board.waitForTimeout(1500);

const tctx = await browser.newContext({ viewport: { width: W, height: TERM_H }, deviceScaleFactor: 2, colorScheme: "dark" });
const terms = await tctx.newPage();
await terms.setContent(TERMINALS);
const T = {
  pane: (id, who) => terms.evaluate(([i, w]) => window.stage.pane(i, w), [id, who]),
  line: (id, kind, text) => terms.evaluate(([i, k, t]) => window.stage.line(i, k, t), [id, kind, text]),
  settle: (id) => terms.evaluate((i) => window.stage.settle(i), id),
  busy: (id, on) => terms.evaluate(([i, o]) => window.stage.busy(i, o), [id, on]),
  event: (text) => terms.evaluate((t) => window.stage.event(t), text),
};
await T.pane("claude", "Claude Code");
await T.pane("codex", "Codex");
await T.pane("cursor", "Cursor");

/** One call in an agent's terminal: the call, then what the server answered. */
async function call(id, label, run, answer, hold = 950) {
  await T.busy(id, true);
  await T.line(id, "call pending", label);
  const t0 = Date.now();
  const got = await run();
  // A call that answers at once still reads as a call and an answer.
  await pause(Math.max(0, 480 - (Date.now() - t0)));
  await T.settle(id);
  const text = typeof answer === "function" ? answer(got) : answer;
  for (const l of [].concat(text ?? [])) await T.line(id, "ans", l);
  await T.busy(id, false);
  await pause(hold);
  return got;
}
/** An agent parked on an event: the wait shows, and resolves when it fires. */
function park(id, label, token, kinds, want) {
  const shown = T.line(id, "call wait pending", label);
  const woke = (async () => {
    for (;;) {
      const got = await mcp(token, "wait_for_event", { kinds, timeout_ms: 60000 });
      if (got && want(got.event ?? got)) return got;
    }
  })();
  return { shown, woke };
}

const names = new Map([[s.maya.id, "Maya"], [s.claude.id, "Claude Code"], [s.codex.id, "Codex"], [s.cursor.id, "Cursor"]]);
const titles = new Map();
const KIND = {
  thread_created: (e) => `[[hi|thread_created]] “${titles.get(e.thread_id) ?? e.thread?.title ?? ""}”`,
  thread_assignment_changed: (e) => `[[hi|thread_assignment_changed]] ${e.assignee_id ? "held by " + (names.get(e.assignee_id) ?? "an agent") : "released"}`,
  message_posted: () => `[[hi|message_posted]]`,
  thread_result_set: () => `[[hi|thread_result_set]]`,
  thread_state_changed: (e) => `[[hi|thread_state_changed]] ${e.from ?? e.from_state ?? ""} → ${e.to ?? e.to_state ?? e.state ?? ""}`,
  review_submitted: (e) => `[[hi|review_submitted]] ${e.decision ?? ""}`,
};
let showEvents = false;
const stream = events(ADMIN, s.ws, (e) => {
  if (e.thread?.title) titles.set(e.thread.id ?? e.thread_id, e.thread.title);
  if (!showEvents) return;
  const by = names.get(e.attribution?.actor_id) ?? "";
  const text = (KIND[e.kind] ?? (() => `[[hi|${e.kind}]]`))(e);
  const who = by && e.kind !== "thread_assignment_changed" ? ` [[note|by ${by}]]` : "";
  void T.event(`[[note|#${e.log_id}]] ${text}${who}`);
});
await stream.ready;

let stillN = 0;
async function still(name) {
  if (!STILLS) return;
  stillN += 1;
  const base = join(OUT, `${String(stillN).padStart(2, "0")}-${name}`);
  await board.screenshot({ path: base + "-console.png" });
  await terms.screenshot({ path: base + "-agents.png" });
}

// Where each agent stands as the take opens.
const cursorWait = park("cursor", "wait_for_event(thread_created)", s.cursor.token, ["thread_created"], () => true);
await cursorWait.shown;
const claudeWait = park("claude", "wait_for_event(thread_created)", s.claude.token, ["thread_created"], () => true);
const codexWait = park("codex", "wait_for_event(thread_state_changed)", s.codex.token, ["thread_state_changed"], (e) => (e.to ?? e.to_state ?? e.state) === "in_review" || JSON.stringify(e).includes("in_review"));
await claudeWait.shown; await codexWait.shown;
await board.waitForTimeout(600);
showEvents = true;

const rec = STILLS ? null : [new Recorder(board, join(OUT, "console"), W, CONSOLE_H), new Recorder(terms, join(OUT, "agents"), W, TERM_H)];
const t0 = Date.now() / 1000;
if (rec) await Promise.all(rec.map((r) => r.start()));
await still("open");
await pause(1400);

// --- Maya files the task in the console.
await press(board, board.getByRole("button", { name: /new task/i }).first());
const title = board.getByRole("textbox", { name: /title|task/i }).first();
await title.waitFor();
await board.keyboard.type("Rate-limit the public API", { delay: 38 });
await pause(250);
await press(board, board.getByRole("button", { name: /^create$/i }).first(), 400);
void glide(board, 1180, CONSOLE_H - 60, 700);

// --- Claude Code wakes on the event and takes it.
const created = await claudeWait.woke;
await T.settle("claude");
await T.line("claude", "ans", "woke: “Rate-limit the public API”");
await still("filed");
await pause(900);
const task = await call("claude", "claim_next_thread(#api)", () => claimNext(s.claude, s.channel), "[[ok|claimed]] · lease 15m", 700);
// Cursor woke on the same event, a moment later: the task has its one holder.
await cursorWait.woke;
await T.settle("cursor");
await T.line("cursor", "ans", "woke: “Rate-limit the public API”");
await call("cursor", "claim_next_thread(#api)", () => mcp(s.cursor.token, "claim_next_thread", { channel_id: s.channel }), (got) => (got ? "claimed" : "nothing to claim · Claude Code holds it"), 900);
void park("cursor", "wait_for_event(thread_created)", s.cursor.token, ["thread_created"], () => true).shown;
await still("claimed");
await call("claude", "post_message", () => mcp(s.claude.token, "post_message", { thread_id: task.id, body: "Added a token-bucket limiter keyed by token id on every /v1 route: 60 requests a minute, then 429 with Retry-After." }), "“Token-bucket limiter on every /v1 route.”", 800);
await call("claude", "set_thread_result", () => mcp(s.claude.token, "set_thread_result", { thread_id: task.id, claim_lease_id: task.claim_lease_id, result: { status: "changed", summary: "Adds a token-bucket limiter keyed by token id on every /v1 route: 60 requests a minute, then 429 with Retry-After." } }), "result stored", 700);
await call("claude", "transition_thread(start_review)", () => mcp(s.claude.token, "transition_thread", { thread_id: task.id, action: "start_review" }), "open → [[hi|in_review]]", 600);
const claudeBack = park("claude", "wait_for_event(review_submitted)", s.claude.token, ["review_submitted"], () => true);

// --- Codex wakes on the hand-off, asks the decision model, and sends it back.
await codexWait.woke;
await T.settle("codex");
await T.line("codex", "ans", "woke: a task is in review");
await pause(900);
const first = await call("codex", "advise_land_gate", () => mcp(s.codex.token, "advise_land_gate", { thread_id: task.id }), (a) => {
  const flagged = a.judgments.filter((j) => j.concern).map((j) => j.reading.toLowerCase());
  return a.land === "green" ? "[[ok|nothing flagged]]" : `[[${a.land === "red" ? "bad" : "warn"}|${a.land === "red" ? "problem reported" : "worth a look"}]]: ${flagged[0] ?? ""}`;
}, 2200);
// The console shows the same advice on the open task.
await board.getByText("Worth a look").first().waitFor({ timeout: 8000 }).catch(() => {});
// The README's picture of the advice is cut from a frame of this take. A
// second screenshot here would race the recorder's: each resizes the page for
// a moment, and the open sheet closes under them.
const adviceBox = await board.getByRole("region", { name: "Advice" }).boundingBox();
await pause(400);
if (rec && adviceBox) {
  const pad = 8;
  writeFileSync(join(OUT, "advice-still.json"), JSON.stringify({
    frame: join("console", rec[0].frames.at(-1).file),
    x: Math.round((adviceBox.x - pad) * SCALE), y: Math.round((adviceBox.y - pad) * SCALE),
    w: Math.round((adviceBox.width + 2 * pad) * SCALE), h: Math.round((adviceBox.height + 2 * pad) * SCALE),
  }));
}
await pause(600);
await still("advice-amber");
await call("codex", "submit_review(request_changes)", () => mcp(s.codex.token, "submit_review", { thread_id: task.id, decision: "request_changes", note: "No test run is reported. Run the suite and say what passed." }), "“Run the suite and say what passed.”", 1300);
const codexAgain = park("codex", "wait_for_event(thread_state_changed)", s.codex.token, ["thread_state_changed"], (e) => JSON.stringify(e).includes("in_review"));
await still("sent-back");

// --- Claude Code wakes on the review, runs the tests, hands off again.
await claudeBack.woke;
await T.settle("claude");
await T.line("claude", "ans", "woke: Codex asked for changes");
await pause(900);
const again = await call("claude", "claim_thread", () => mcp(s.claude.token, "claim_thread", { thread_id: task.id }), "claimed again · lease 15m", 700);
const lease = (again?.thread ?? again)?.claim_lease_id ?? task.claim_lease_id;
await call("claude", "post_message", () => mcp(s.claude.token, "post_message", { thread_id: task.id, body: "Ran the suite: cargo test passed, 212 passed and 0 failed. 14 of them are new, for the limiter: 60 requests a minute per token, then 429 with Retry-After." }), "“cargo test: 212 passed, 0 failed.”", 900);
await call("claude", "set_thread_result", () => mcp(s.claude.token, "set_thread_result", { thread_id: task.id, claim_lease_id: lease, result: { status: "changed", summary: "Rate-limits the public API: a token-bucket limiter keyed by token id on every /v1 route, 60 requests a minute, then 429 with Retry-After. 14 new tests; cargo test passed (212 passed, 0 failed)." } }), "result stored · 14 new tests", 700);
await call("claude", "transition_thread(start_review)", () => mcp(s.claude.token, "transition_thread", { thread_id: task.id, action: "start_review" }), "open → [[hi|in_review]]", 600);

// --- Codex reads the second hand-off and approves it.
await codexAgain.woke;
await T.settle("codex");
await T.line("codex", "ans", "woke: back in review");
await pause(800);
const second = await call("codex", "advise_land_gate", () => mcp(s.codex.token, "advise_land_gate", { thread_id: task.id, refresh: true }), (a) => {
  const flagged = a.judgments.filter((j) => j.concern).map((j) => j.reading.toLowerCase());
  return a.land === "green" ? "[[ok|nothing flagged]] · ten readings clear" : `[[${a.land === "red" ? "bad" : "warn"}|${a.land === "red" ? "problem reported" : "worth a look"}]]: ${flagged[0] ?? ""}`;
}, 2000);
await board.getByText("Nothing flagged").first().waitFor({ timeout: 8000 }).catch(() => {});
await pause(600);
await still("advice-green");
await call("codex", "submit_review(approve)", async () => {
  const packet = await mcp(s.codex.token, "get_review_packet", { thread_id: task.id });
  return mcp(s.codex.token, "submit_review", { thread_id: task.id, decision: "approve", evidence_root: packet.evidence_root });
}, "[[ok|approved]] · not its own work", 1500);
await still("approved");

// --- Maya, who has had the task open all along, closes it and then the sheet.
await press(board, board.getByRole("button", { name: /more actions/i }).first(), 800);
await board.waitForTimeout(350);
await press(board, board.getByRole("menuitem", { name: /close task/i }).first(), 500);
await board.waitForTimeout(1500);
await still("sheet-done");
await press(board, board.getByRole("button", { name: "Close the task sheet" }).first(), 600);
await board.waitForTimeout(900);
await still("closed");
await pause(2600);

const end = Date.now() / 1000;
if (rec) {
  const counts = await Promise.all(rec.map((r) => r.stop(t0, end)));
  console.log(`recorded ${(end - t0).toFixed(1)} s: ${counts[0]} console frames, ${counts[1]} agent frames`);
}
console.log(`advice: first ${first.land} ${JSON.stringify(first.reasons)}, second ${second.land} ${JSON.stringify(second.reasons)}`);
stream.close();
await browser.close();
// An agent still parked on an event would hold the process open.
process.exit(0);
