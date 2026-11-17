// The console leg of scripts/launch-gate.sh (docs/Launch Gate.md). It drives
// the real console at /ui in headless Chromium, as one test tenant's person,
// on the session the server issued for that tenant's admin token
// (POST /auth/session/from-token, what the console's own "paste a token"
// sign-in calls). It never signs in through the identity provider: on
// maidan.world that takes a one-time code mailed to a person.
//
// One phase per run (GATE_PHASE): channel, people, review or tenant. It prints
// one line per check, "PASS  ..." or "FAIL  ...: why", and "SET KEY=value" for
// an id the shell needs. Screenshots go to GATE_SHOTS. The cookie arrives in
// GATE_COOKIE and is never printed: every error below is reduced to its first
// line, and no call that takes the cookie reports its arguments.
import { chromium } from "@playwright/test";

const env = (key) => {
  const value = process.env[key];
  if (!value) {
    console.log(`FAIL  console: ${key} is not set`);
    process.exit(2);
  }
  return value;
};

const origin = env("GATE_ORIGIN").replace(/\/$/, "");
const phase = env("GATE_PHASE");
const shots = env("GATE_SHOTS");
const step = env("GATE_STEP");
const cookie = env("GATE_COOKIE");
const ws = env("GATE_WS");
const TIMEOUT = 20_000;

let failed = 0;
const firstLine = (e) => String(e?.message ?? e).split("\n")[0].slice(0, 240);
async function check(what, fn) {
  try {
    await fn();
    console.log(`PASS  ${step}: ${what}`);
  } catch (e) {
    failed += 1;
    console.log(`FAIL  ${step}: ${what}: ${firstLine(e)}`);
  }
}
const must = (ok, why) => {
  if (!ok) throw new Error(why);
};

const browser = await chromium.launch({ headless: true });
const context = await browser.newContext({
  viewport: { width: 1440, height: 900 },
  // The console's service worker would take the page's requests; nothing
  // here tests it, and ui-tests blocks it for the same reason.
  serviceWorkers: "block",
});
try {
  await context.addCookies([
    { name: "maidan_session", value: cookie, url: origin, httpOnly: true, sameSite: "Lax", secure: origin.startsWith("https:") },
  ]);
} catch {
  console.log(`FAIL  ${step}: console: the session cookie was not accepted by the browser`);
  await browser.close();
  process.exit(1);
}
const page = await context.newPage();
page.setDefaultTimeout(TIMEOUT);

async function open(hash, channel) {
  if (channel) {
    // The board opens on the channel the console last showed, kept here.
    await page.addInitScript((id) => window.localStorage.setItem("maidan_console_channel", id), channel);
  }
  await page.goto(`${origin}/ui/${hash}`);
  await page.locator(".rail:visible, .tabbar:visible").first().waitFor({ state: "visible" });
}
async function shot(name) {
  await page.screenshot({ path: `${shots}/${name}.png`, fullPage: false });
}
async function getJson(path) {
  const res = await page.request.get(`${origin}${path}`);
  must(res.ok(), `${path} answered ${res.status()}`);
  return res.json();
}

if (phase === "channel") {
  await check("the console opens on this tenant's session", async () => {
    await open("#/tools/workspace");
    const session = await getJson("/auth/session");
    must(session.workspace_id === ws, `the session is for ${session.workspace_id}`);
  });
  await check("Tools, Workspace, New channel creates launch", async () => {
    const form = page.locator("form", { has: page.getByLabel("New channel") });
    await form.getByLabel("New channel").fill("launch");
    await form.getByRole("button", { name: "Create" }).click();
    await page.getByText("Channel created").first().waitFor({ state: "visible" });
    await shot("2-a-channel-created");
    const channels = await getJson(`/ui/api/workspaces/${ws}/channels`);
    const launch = channels.find((c) => c.name === "launch");
    must(launch, `no launch channel in ${JSON.stringify(channels.map((c) => c.name))}`);
    console.log(`SET CH=${launch.id}`);
  });
} else if (phase === "people") {
  const person = env("GATE_PERSON");
  const agent = env("GATE_AGENT");
  await check(`People lists two members, ${person} and the agent ${agent}`, async () => {
    await open("#/people");
    const rows = page.locator(".roster-list .person-row");
    await rows.filter({ hasText: person }).filter({ hasText: "Human" }).first().waitFor({ state: "visible" });
    await rows.filter({ hasText: agent }).filter({ hasText: "Agent" }).first().waitFor({ state: "visible" });
    must((await rows.count()) === 2, `the roster has ${await rows.count()} rows`);
    await shot("3b-a-people");
  });
} else if (phase === "review") {
  const channel = env("GATE_CH");
  const thread = env("GATE_T");
  const title = env("GATE_TITLE");
  const sheet = page.getByRole("dialog");
  await check("the Board shows the task under Needs review", async () => {
    await open(`#/board?channel=${encodeURIComponent(channel)}`, channel);
    await page.locator("#lane-review .task", { hasText: title }).first().waitFor({ state: "visible" });
    await shot("4-a-board-in-review");
  });
  await check("the task sheet's Approve signs the review packet", async () => {
    await page.locator(".lane .task", { hasText: title }).first().click();
    await sheet.getByRole("button", { name: "Approve" }).click();
    await sheet.locator(".sheet-note", { hasText: "Approved." }).waitFor({ state: "visible" });
    const reviews = await getJson(`/ui/api/threads/${thread}/reviews`);
    must(JSON.stringify(reviews).includes("approve"), "no approving review recorded");
  });
  await check("More actions, Close task closes it", async () => {
    await sheet.getByRole("button", { name: "More actions" }).click();
    await sheet.getByRole("menu").getByRole("menuitem", { name: "Close task" }).click();
    await sheet.locator(".sheet-note", { hasText: "Closed." }).waitFor({ state: "visible" });
    // The card moves lanes with an animation; shoot once it has landed.
    await page.locator("#lane-done .task", { hasText: title }).first().waitFor({ state: "visible" });
    await page.locator("#lane-review .task", { hasText: title }).first().waitFor({ state: "detached" });
    await shot("6-a-task-closed");
    const shown = await getJson(`/ui/api/threads/${thread}`);
    must(shown.state === "closed", `the thread is ${shown.state}`);
  });
} else if (phase === "tenant") {
  const forbidden = env("GATE_FORBIDDEN").split(",").filter(Boolean);
  const own = env("GATE_OWN").split(",").filter(Boolean);
  await check("B's Board and People show B's own and nothing of workspace A", async () => {
    await open("#/board");
    await page.waitForLoadState("networkidle");
    await shot("8-b-board");
    for (const text of forbidden) must((await page.getByText(text).count()) === 0, `the Board shows ${text}`);
    await open("#/people");
    // B's own members first, so an empty page cannot pass for a refusal.
    const rows = page.locator(".roster-list .person-row");
    for (const text of own) await rows.filter({ hasText: text }).first().waitFor({ state: "visible" });
    await shot("8-b-people");
    for (const text of forbidden) must((await page.getByText(text).count()) === 0, `People shows ${text}`);
  });
} else {
  console.log(`FAIL  ${step}: console: unknown phase ${phase}`);
  failed += 1;
}

await browser.close();
process.exit(failed ? 1 : 0);
