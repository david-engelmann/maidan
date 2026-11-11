import { mkdirSync, rmSync, writeFileSync } from "fs";
import { writeFile } from "fs/promises";
import { tmpdir } from "os";
import { join } from "path";
import { test, expect, request as apiRequest, type APIRequestContext, type Locator, type Page } from "@playwright/test";

// Records the docs' motion clips from the real console on the public demo.
// Run it through scripts/make-motion.sh, which turns the frames into the GIFs
// and MP4s under docs/assets/motion/.
//
// The demo starts empty and runs no simulation, so this spec plays every part
// itself. It starts a sandbox the way a visitor does (Start a sandbox, then
// Open the console, which signs the browser in as the sandbox's admin), and
// never types a credential: the agents act with the agent token the sandbox
// hands this visitor, and with two more agents the admin adds, all of which
// stay in memory and die with the sandbox within a day.
//
// Each frame is kept with the time it was taken, so a clip keeps the real
// timing of what happened on screen; make-motion.sh holds each frame until the
// next.

const FRAMES = process.env.MOTION_FRAMES ?? join(tmpdir(), "maidan-motion-frames");
const THEME = (process.env.MOTION_THEME ?? "dark") as "dark" | "light";

interface Credential {
  member_id: string;
  handle: string;
  token: string;
}
interface Sandbox {
  workspace_id: string;
  channel_id: string;
  task_id: string;
  admin: Credential;
  agent: Credential;
}
interface Claimed {
  id: string;
  title: string;
  claim_lease_id: string;
}

// ---------------------------------------------------------------- recording

// Each frame is a screenshot rendered at 1.5 times the CSS pixels: sharp
// once make-motion.sh halves it to 960 px, and still about 20 frames a
// second. Chromium's screencast would be cheaper, but headless it paints at
// one CSS pixel per pixel whatever the device scale.
const SCALE = 1.5;

// What the take has sent to the demo, and how much of it was refused. The
// demo allows one client 60 requests a minute and one workspace 120, and the
// console reloads the board on every live event, so a clip that runs into the
// limit shows "Too many requests" on screen. A take with a refusal while it
// records fails rather than ship that.
const traffic = { console: 0, members: 0, refused: 0 };

class Recorder {
  private frames: { file: string; at: number }[] = [];
  private writes: Promise<void>[] = [];
  private dir = "";
  private clip = "";
  private running = false;
  private loop: Promise<void> = Promise.resolve();
  private before = { ...traffic };

  constructor(private page: Page) {}

  async start(clip: string) {
    this.clip = clip;
    this.before = { ...traffic };
    this.dir = join(FRAMES, clip);
    rmSync(this.dir, { recursive: true, force: true });
    mkdirSync(this.dir, { recursive: true });
    this.frames = [];
    this.writes = [];
    const cdp = await this.page.context().newCDPSession(this.page);
    const { width, height } = this.page.viewportSize() ?? { width: 1280, height: 800 };
    this.running = true;
    this.loop = (async () => {
      while (this.running) {
        const at = Date.now() / 1000;
        const shot = await cdp.send("Page.captureScreenshot", {
          format: "png",
          optimizeForSpeed: true,
          clip: { x: 0, y: 0, width, height, scale: SCALE },
        });
        const file = `${String(this.frames.length).padStart(5, "0")}.png`;
        this.frames.push({ file, at });
        this.writes.push(writeFile(join(this.dir, file), Buffer.from(shot.data, "base64")));
      }
      await cdp.detach();
    })();
  }

  /** Stops after `holdMs` more, and writes when each frame starts and ends. */
  async stop(holdMs = 1200) {
    await this.page.waitForTimeout(holdMs);
    this.running = false;
    await this.loop;
    const end = Date.now() / 1000;
    await Promise.all(this.writes);
    const secs = end - this.frames[0].at;
    console.log(
      `${this.clip}: ${this.frames.length} frames in ${secs.toFixed(1)} s; ` +
        `${traffic.console - this.before.console} requests from the console, ${traffic.members - this.before.members} from the members`,
    );
    expect(this.frames.length, "the take recorded no frames").toBeGreaterThan(1);
    expect(traffic.refused - this.before.refused, `the demo refused requests while ${this.clip} recorded`).toBe(0);
    // ffconcat: each frame lasts until the next one was taken. The last file
    // is listed twice because the concat demuxer drops the last duration.
    const lines = ["ffconcat version 1.0"];
    this.frames.forEach((f, i) => {
      const next = i + 1 < this.frames.length ? this.frames[i + 1].at : end;
      lines.push(`file '${f.file}'`, `duration ${Math.max(0.001, next - f.at).toFixed(4)}`);
    });
    lines.push(`file '${this.frames[this.frames.length - 1].file}'`);
    writeFileSync(join(this.dir, "frames.ffconcat"), lines.join("\n") + "\n");
  }
}

// ------------------------------------------------------------------- cursor

// Headless Chromium paints no pointer, so a clip would show buttons pressing
// themselves. This draws one that follows the mouse and dips on a click.
const CURSOR = () => {
  const install = () => {
    if (document.getElementById("motion-cursor")) return;
    const c = document.createElement("div");
    c.id = "motion-cursor";
    c.innerHTML =
      '<svg width="24" height="24" viewBox="0 0 24 24" aria-hidden="true"><path d="M5 3l13.5 8.6-6.2 1.3-3.4 5.6z" fill="#fff" stroke="#16130f" stroke-width="1.6" stroke-linejoin="round"/></svg>';
    c.style.cssText =
      "position:fixed;left:0;top:0;z-index:2147483647;pointer-events:none;transform:translate(-80px,-80px);filter:drop-shadow(0 1px 2px rgba(0,0,0,.35))";
    document.documentElement.appendChild(c);
    let x = -80;
    let y = -80;
    let s = 1;
    const place = () => (c.style.transform = `translate(${x - 5}px, ${y - 3}px) scale(${s})`);
    addEventListener("mousemove", (e) => { x = e.clientX; y = e.clientY; place(); }, true);
    addEventListener("mousedown", () => { s = 0.82; place(); }, true);
    addEventListener("mouseup", () => { s = 1; place(); }, true);
  };
  if (document.readyState === "loading") addEventListener("DOMContentLoaded", install);
  else install();
};

let mouse = { x: 1100, y: 760 };

/**
 * Moves the pointer along an eased path, as a hand would. Paced by the clock,
 * not by a count of steps: while frames are being taken each move is slower,
 * and a fixed count of steps would stretch the glide with it.
 */
async function glide(page: Page, x: number, y: number, ms = 600) {
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

async function pointAt(page: Page, target: Locator, ms = 600) {
  await target.scrollIntoViewIfNeeded();
  const box = await target.boundingBox();
  if (!box) throw new Error("target has no box");
  await glide(page, box.x + box.width / 2, box.y + box.height / 2, ms);
}

async function press(page: Page, target: Locator, ms = 600) {
  await pointAt(page, target, ms);
  await page.waitForTimeout(140);
  await page.mouse.down();
  await page.waitForTimeout(90);
  await page.mouse.up();
}

// --------------------------------------------------------------------- API

class Member {
  constructor(
    readonly name: string,
    readonly id: string,
    private api: APIRequestContext,
    private origin: string,
  ) {}

  async call<T = unknown>(method: string, path: string, body?: unknown): Promise<T> {
    // The demo limits each client to 60 requests a minute; a take waits out a
    // 429 rather than failing on it.
    for (let attempt = 0; ; attempt++) {
      traffic.members++;
      const res = await this.api.fetch(path, {
        method,
        headers: {
          accept: "application/json",
          // A cookie session's writes are refused without an Origin of its own.
          origin: this.origin,
          "idempotency-key": crypto.randomUUID(),
          ...(body === undefined ? {} : { "content-type": "application/json" }),
        },
        data: body === undefined ? undefined : JSON.stringify(body),
      });
      const text = await res.text();
      if (res.status() === 429) traffic.refused++;
      if (res.status() === 429 && attempt < 6) {
        const wait = Number(res.headers()["retry-after"] ?? "5");
        await new Promise((r) => setTimeout(r, Math.min(60, Math.max(1, wait)) * 1000));
        continue;
      }
      if (!res.ok()) throw new Error(`${this.name}: ${method} ${path} answered ${res.status()}: ${text.slice(0, 300)}`);
      return (text ? JSON.parse(text) : null) as T;
    }
  }

  say(t: { id: string }, body: string) {
    return this.call("POST", `/threads/${t.id}/messages`, { body });
  }

  /** Approves another member's work, by the packet the reviewer read. */
  async approve(tid: string, note?: string) {
    const packet = await this.call<{ evidence_root: string }>("GET", `/threads/${tid}/review-packet`);
    await this.call("POST", `/threads/${tid}/reviews`, { decision: "approve", evidence_root: packet.evidence_root, note });
  }

  close(tid: string) {
    return this.call("POST", `/threads/${tid}`, { action: "close" });
  }
}

class Agent extends Member {
  constructor(name: string, id: string, api: APIRequestContext, origin: string, private channel: () => string) {
    super(name, id, api, origin);
  }

  async claim(): Promise<Claimed> {
    const t = await this.call<Claimed | null>("POST", `/channels/${this.channel()}/threads/claim-next`, { lease_secs: 1800 });
    if (!t) throw new Error(`${this.name}: nothing to claim`);
    await this.call("POST", `/threads/${t.id}/claim/acknowledge`, { claim_lease_id: t.claim_lease_id });
    return t;
  }

  ask(t: { id: string }, note: string) {
    return this.call("PUT", `/threads/${t.id}/status`, { status: "needs_input", note });
  }

  evidence(t: { id: string }, kind: string, source: string, note: string) {
    return this.call("POST", `/threads/${t.id}/evidence-records`, { kind, source, note });
  }

  async handIn(t: Claimed, result: Record<string, unknown>) {
    await this.call("PUT", `/threads/${t.id}/result`, { claim_lease_id: t.claim_lease_id, result });
    await this.call("POST", `/threads/${t.id}`, { action: "start_review" });
    await this.call("POST", `/threads/${t.id}/claim/release`, { claim_lease_id: t.claim_lease_id });
  }
}

// ------------------------------------------------------------------- scene

interface Scene {
  page: Page;
  sb: Sandbox;
  channel: string;
  maya: Member;
  claude: Agent;
  codex: Agent;
  cursor: Agent;
}

/** Opens a sandbox the way a visitor does, and signs the console in. */
async function startSandbox(page: Page): Promise<Sandbox> {
  await page.goto("/");
  const minted = page.waitForResponse((r) => r.url().endsWith("/sandbox") && r.request().method() === "POST");
  await page.getByRole("button", { name: "Start a sandbox" }).click();
  const res = await minted;
  expect(res.status(), "the demo refused a sandbox (it allows three an hour from one address)").toBe(201);
  const sb = (await res.json()) as Sandbox;
  await expect(page.getByRole("heading", { name: /sandbox is ready/i })).toBeVisible();
  await page.getByRole("button", { name: "Open the console" }).click();
  await page.waitForURL(/\/ui\//);
  // The console stays closed while the scene is set, so its own requests do
  // not share the demo's per-client budget with the setup.
  await page.goto("about:blank");
  return sb;
}

const pause = (ms: number) => new Promise((r) => setTimeout(r, ms));

async function cast(page: Page, sb: Sandbox, baseURL: string): Promise<Scene> {
  const origin = new URL(baseURL).origin;
  // The admin acts through the browser's own session cookie.
  const maya = new Member("Maya", sb.admin.member_id, page.request, origin);
  // A member renames only itself.
  await maya.call("PATCH", `/members/${sb.admin.member_id}`, { display_name: "Maya" });

  const scene = { page, sb, maya, channel: "" } as Scene;
  const channel = () => scene.channel;
  const agentApi = (token: string) => apiRequest.newContext({ baseURL, extraHTTPHeaders: { authorization: `Bearer ${token}` } });
  scene.claude = new Agent("Claude Code", sb.agent.member_id, await agentApi(sb.agent.token), origin, channel);
  await scene.claude.call("PATCH", `/members/${sb.agent.member_id}`, { display_name: "Claude Code" });
  const added = async (handle: string, display_name: string) => {
    const a = await maya.call<{ member: { id: string }; token: { secret: string } }>("POST", `/workspaces/${sb.workspace_id}/agents`, { handle, display_name });
    return new Agent(display_name, a.member.id, await agentApi(a.token.secret), origin, channel);
  };
  scene.codex = await added("codex", "Codex");
  scene.cursor = await added("cursor", "Cursor");

  const ch = await maya.call<{ id: string }>("POST", `/workspaces/${sb.workspace_id}/channels`, { name: "build", topic: "What we are shipping this week" });
  scene.channel = ch.id;
  return scene;
}

// Titles in the order the agents take them: claim-next hands out the oldest
// open task first.
const TASKS = {
  onboarding: "Update the onboarding checklist",
  flaky: "Fix the flaky login test on Safari",
  limits: "Add rate limiting to the public API",
  guide: "Write the v2 migration guide",
  avif: "Move image uploads to AVIF",
  notes: "Draft the 2.4 release notes",
  triage: "Triage this week's bug reports",
  search: "Speed up search on large workspaces",
} as const;

interface Held {
  guide: Claimed;
  avif: Claimed;
}

// The board the clips open on: work done, in review, in progress and open.
async function stage(s: Scene): Promise<Held> {
  const { maya, claude, codex, cursor, channel } = s;
  // Codex files the rate-limiting task and names Maya its reviewer, so the
  // review waits on her: an owner's own approval does not count.
  for (const [key, title] of Object.entries(TASKS)) {
    const by = key === "limits" ? codex : maya;
    await by.call("POST", `/channels/${channel}/threads`, { title });
  }

  const onboarding = await cursor.claim();
  await cursor.say(onboarding, "Checklist now matches the new sign-in flow. Removed the two steps nobody needs any more.");
  await cursor.handIn(onboarding, { summary: "Onboarding checklist matches the new sign-in flow.", status: "done" });
  await claude.approve(onboarding.id, "Read it through as a new hire would. Good.");
  await maya.close(onboarding.id);

  const flaky = await codex.claim();
  await codex.say(flaky, "The test raced the redirect. It now waits for the session cookie before it asserts.");
  await codex.evidence(flaky, "test_run", "playwright", "login.spec.ts: 40 runs on WebKit, 40 passed");
  await codex.handIn(flaky, { summary: "The login test waits for the session cookie; 40 of 40 WebKit runs pass.", status: "done" });
  await claude.approve(flaky.id);
  await maya.close(flaky.id);

  await pause(1500);
  const limits = await claude.claim();
  await claude.say(limits, "Token bucket per API key: 600 requests a minute, and Retry-After on a 429.");
  await claude.evidence(limits, "test_run", "cargo test", "rate_limit: 18 passed, 0 failed");
  await claude.evidence(limits, "diff", "git diff --stat", "6 files changed, 212 insertions(+), 9 deletions(-)");
  await claude.evidence(limits, "note", "load test", "p99 stays under 40 ms at 2,000 requests a second; request 601 in a minute gets a 429.");
  await codex.call("POST", `/threads/${limits.id}/reviewers`, { member_id: maya.id });
  await claude.handIn(limits, {
    summary: "Rate limiting on every public route: a token bucket per API key, 600 a minute, Retry-After on a 429.",
    status: "done",
    tests: "18 passed",
  });

  const guide = await cursor.claim();
  await maya.say(guide, "Cover the token changes first. Most upgrade questions are about tokens.");
  await cursor.say(guide, "Sections 1 to 3 are drafted: tokens, channels, and the new task states.");
  // Cursor asks its question just before the thread clip, so Needs you
  // opens on the one review and ends with nothing left waiting.

  await pause(1500);
  const avif = await codex.claim();
  await codex.say(avif, "Encoder is in behind a flag. Measuring sizes on the sample set now.");
  return { guide, avif };
}

/**
 * Opens a console route, then leaves it idle for a minute. The demo's rate
 * limits are fixed one-minute windows per client and per workspace, and a
 * route's first load (and a page's scripts and fonts) costs a good part of
 * one; after the minute the clip starts on a settled screen with its whole
 * budget. The console sends nothing while nothing happens.
 */
async function settle(page: Page, hash: string, ready: Locator) {
  await page.goto(`/ui/${hash}`);
  await expect(ready).toBeVisible();
  // Fonts and avatars land after the route renders.
  await page.evaluate(() => document.fonts.ready);
  const before = traffic.console;
  await pause(62_000);
  await expect(ready).toBeVisible();
  console.log(`${hash}: ${traffic.console - before} requests from the idle console`);
}

const parkPointer = (page: Page) => glide(page, 1160, 740, 1);

test.use({ colorScheme: THEME });

test(`motion clips (${THEME})`, async ({ page, baseURL }) => {
  await page.addInitScript(CURSOR);
  const origin = new URL(baseURL ?? "https://try.maidan.world").origin;
  page.on("request", (r) => {
    if (r.url().startsWith(origin)) traffic.console++;
  });
  page.on("response", (r) => {
    if (r.status() === 429 && r.url().startsWith(origin)) traffic.refused++;
  });
  const sb = await startSandbox(page);
  const s = await cast(page, sb, baseURL ?? "https://try.maidan.world");
  const held = await stage(s);
  const { maya, claude, codex, cursor } = s;
  await page.addInitScript(
    ([t, ch]) => {
      try {
        window.localStorage.setItem("maidan_console_theme", t);
        window.localStorage.setItem("maidan_console_channel", ch);
      } catch {
        /* without storage the take still records, in the System theme */
      }
    },
    [THEME, s.channel] as const,
  );
  const rec = new Recorder(page);
  const away = (ms = 500) => glide(page, 1160, 740, ms);

  // 1. The board, live: a person files a task, agents take and hand in work,
  // and another member's review closes one.
  // Evidence lands before the clip; in it, Codex only hands the work in.
  await codex.evidence(held.avif, "note", "sample set", "1,200 images: 41% smaller than WebP at the same visual score");
  await settle(page, "#/board", page.locator("h2.lane-head", { hasText: "Needs review" }));
  await parkPointer(page);
  await rec.start("board-live");
  await page.waitForTimeout(400);
  await press(page, page.getByRole("button", { name: /New task/ }).first(), 650);
  const composer = page.getByPlaceholder("What needs doing?");
  await expect(composer).toBeVisible();
  await page.waitForTimeout(150);
  await composer.pressSequentially("Add a status page for the API", { delay: 40 });
  await page.waitForTimeout(250);
  await composer.press("Enter");
  // The new task opens in its sheet; put it away to watch the lanes.
  const sheetX = page.getByRole("button", { name: "Close the task sheet" });
  await expect(sheetX).toBeVisible();
  await page.waitForTimeout(500);
  await press(page, sheetX, 450);
  await expect(sheetX).toBeHidden();
  await away();
  const notes = await claude.claim();
  await page.waitForTimeout(800);
  await codex.handIn(held.avif, { summary: "Uploads are stored as AVIF: 41% smaller than WebP on the sample set.", status: "done" });
  await page.waitForTimeout(900);
  await claude.approve(held.avif.id, "Spot-checked 20 images side by side. No visible loss.");
  await maya.close(held.avif.id);
  await rec.stop(1600);

  // 2. Needs you: the review waiting on a person, approved and closed, and
  // nothing left waiting.
  await settle(page, "#/needs", page.getByRole("button", { name: /^Approve/ }).first());
  await parkPointer(page);
  await rec.start("needs-you");
  await page.waitForTimeout(500);
  const card = page.locator(".need-card", { hasText: TASKS.limits });
  await pointAt(page, card.locator(".need-title"), 650);
  await page.waitForTimeout(700);
  await press(page, card.getByRole("button", { name: /^Approve/ }), 550);
  const go = card.locator('[data-need-control="confirm-go"]');
  await expect(go).toBeVisible();
  await page.waitForTimeout(450);
  await press(page, go, 350);
  const closeTask = card.getByRole("button", { name: "Close task" });
  await expect(closeTask).toBeVisible();
  await page.waitForTimeout(550);
  await press(page, closeTask, 450);
  await away(600);
  await expect(page.getByText("All caught up")).toBeVisible();
  await rec.stop(1800);

  // 3. A thread: the agent asked a question; the person answers it, and the
  // agent finishes and hands the work in.
  // The question goes in the thread as a message, and the task waits on a person.
  const question = "Should the guide keep the v1 examples beside the v2 ones?";
  await cursor.say(held.guide, question);
  await cursor.ask(held.guide, question);
  await settle(page, `#/threads/t/${held.guide.id}`, page.getByPlaceholder("Message"));
  await parkPointer(page);
  await rec.start("thread");
  await page.waitForTimeout(600);
  const box = page.getByPlaceholder("Message");
  await press(page, box, 650);
  await box.pressSequentially("Yes, side by side. And link the changelog at the end.", { delay: 32 });
  await page.waitForTimeout(250);
  await box.press("Enter");
  await away();
  await page.waitForTimeout(900);
  await cursor.say(held.guide, "Done. Every section shows v1 and v2 side by side, and the changelog is linked at the end.");
  await page.waitForTimeout(800);
  await cursor.evidence(held.guide, "note", "link check", "All 37 links resolve; 12 code samples run against v2.");
  await cursor.handIn(held.guide, { summary: "The v2 migration guide: tokens, channels and task states, with v1 and v2 side by side.", status: "done" });
  await rec.stop(2000);

  // 4. Activity: everything happening in the workspace, as it happens.
  await settle(page, "#/activity", page.getByRole("heading", { name: "Activity" }).first());
  await parkPointer(page);
  await rec.start("activity");
  await page.waitForTimeout(700);
  const triage = await codex.claim();
  await page.waitForTimeout(500);
  await codex.say(triage, "31 new reports this week. Grouping them by area first.");
  await page.waitForTimeout(500);
  await claude.say(notes, "First draft of the 2.4 notes is up: rate limits, AVIF uploads, the migration guide.");
  await page.waitForTimeout(500);
  await claude.handIn(notes, { summary: "Release notes for 2.4, in the docs folder.", status: "done" });
  await page.waitForTimeout(600);
  await codex.approve(notes.id, "Reads well. Added the AVIF size numbers.");
  await page.waitForTimeout(500);
  await maya.close(notes.id);
  await rec.stop(1800);

  // 5. Catalog: the tools an agent can call here, and what each one takes.
  await settle(page, "#/catalog", page.getByPlaceholder("Find a tool"));
  await parkPointer(page);
  await rec.start("catalog");
  await page.waitForTimeout(500);
  const find = page.getByPlaceholder("Find a tool");
  await press(page, find, 650);
  await find.pressSequentially("claim next", { delay: 65 });
  await page.waitForTimeout(600);
  const tool = page.locator(".tool-row", { hasText: "Claim next thread" }).filter({ hasNotText: "workspace" });
  await press(page, tool, 650);
  await expect(page.getByRole("region", { name: "Tool details" })).toBeVisible();
  await page.waitForTimeout(500);
  await away(600);
  await rec.stop(2200);
});
