import { test, expect, type Locator, type Page, type TestInfo } from "@playwright/test";
import { mkdirSync, writeFileSync } from "fs";
import { join } from "path";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Tools, Integrations against the real server: a workspace's own GitHub,
// Slack and Linear connections and its outbound webhooks. The harness's
// connection factory refuses any token that does not start with `good-` with
// the reason "Bad credentials", and names the account otherwise, so the
// provider is never called. The console's own logic is unit tested with
// mocked requests; this is the one place it meets the server.
const fx = fixtures();
test.describe.configure({ mode: "serial", retries: 0 });

const GOOD_TOKEN = "good-token-1";
const WEBHOOK_SECRET = "whsec-ui-test-0123456789abcdef";
const BAD_TOKEN = "bad-token-xyz-999";
const HOOK_URL = "https://hooks.example.com/maidan-ui-test";
const HOOK_LABEL = "ui-test hook";

type Seen = { url: string; status: number; body: string };

/**
 * Everything that must not happen on a happy path: a console error, a page
 * error, a failed request. `allowRefusal` lets the one deliberate 400 for a
 * bad token through, and the browser's own line about it.
 */
function watch(page: Page, allowRefusal = false) {
  const problems: string[] = [];
  const seen: Seen[] = [];
  const reading: Promise<void>[] = [];
  page.on("pageerror", (err) => problems.push(`page error: ${err.message}`));
  page.on("console", (msg) => {
    if (msg.type() !== "error") return;
    if (allowRefusal && /status of 400/.test(msg.text())) return;
    problems.push(`console error: ${msg.text()}`);
  });
  page.on("response", (res) => {
    const url = res.url();
    const status = res.status();
    const refusal = allowRefusal && status === 400 && res.request().method() === "PUT" && /\/connections\/github$/.test(url);
    if (status >= 400 && !refusal) problems.push(`${status} ${res.request().method()} ${url}`);
    if (!url.includes("/ui/api")) return;
    reading.push(
      res.text().then(
        (body) => void seen.push({ url, status, body }),
        () => void seen.push({ url, status, body: "" }),
      ),
    );
  });
  return {
    problems,
    seen,
    /** The bodies of every /ui/api response so far, once they have all been read. */
    async bodies(): Promise<string[]> {
      await Promise.all(reading);
      return seen.map((s) => s.body);
    },
  };
}

async function open(page: Page, theme: "dark" | "light" = "dark", viewport = { width: 1280, height: 800 }) {
  await page.setViewportSize(viewport);
  await page.addInitScript((t) => window.localStorage.setItem("maidan_console_theme", t), theme);
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/tools");
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
  await page.getByRole("tablist", { name: "Tools" }).getByRole("tab", { name: "Integrations", exact: true }).click();
  await expect(page).toHaveURL(/#\/tools\/integrations$/);
  await expect(page.getByRole("heading", { name: "GitHub, Slack and Linear", exact: true })).toBeVisible();
  await expect(connections(page).getByRole("listitem")).toHaveCount(3);
}

const connections = (page: Page) => page.getByRole("list", { name: "Connections", exact: true });
const surface = (page: Page, name: string): Locator =>
  connections(page).getByRole("listitem").filter({ has: page.getByRole("heading", { name, exact: true }) });
const webhooks = (page: Page) => page.getByRole("region", { name: "Webhooks", exact: true });

async function connectGithub(page: Page, token: string, secret?: string) {
  const row = surface(page, "GitHub");
  await row.getByLabel("Fine-grained personal access token", { exact: true }).fill(token);
  if (secret) await row.getByLabel("Webhook secret (optional)").fill(secret);
  await row.getByRole("button", { name: "Connect GitHub", exact: true }).click();
}

async function expectConnectedGithub(page: Page) {
  const row = surface(page, "GitHub");
  await expect(row.getByText("Connected", { exact: true })).toBeVisible();
  await expect(row).toContainText("octo-team");
  const url = row.getByText(/^https?:\/\/.*\/connections\/github\/webhook$/);
  await expect(url).toBeVisible();
  await expect(url).toContainText(fx.workspace_id);
  await expect(row).toContainText("A webhook secret is set.");
  return row;
}

async function disconnectGithub(page: Page) {
  const row = surface(page, "GitHub");
  await row.getByRole("button", { name: "Remove the GitHub connection…", exact: true }).click();
  await row.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(row.getByText("Not connected", { exact: true })).toBeVisible();
}

/** What a person could read or copy off the page: its text and every field's value. */
async function pageText(page: Page): Promise<string> {
  const text = await page.locator("body").innerText();
  const fields = await page.evaluate(() => Array.from(document.querySelectorAll("input")).map((i) => i.value).join("\n"));
  return `${text}\n${fields}`;
}

async function shot(page: Page, testInfo: TestInfo, name: string) {
  const png = await page.screenshot({ fullPage: true });
  await testInfo.attach(name, { body: png, contentType: "image/png" });
  const dir = process.env.CONN_SHOTS_DIR;
  if (dir) {
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `${name}.png`), png);
  }
}

test("Integrations lists GitHub, Slack and Linear, each not connected", async ({ page }) => {
  const w = watch(page);
  await open(page);
  await expect(connections(page).getByRole("listitem").getByRole("heading")).toHaveText(["GitHub", "Slack", "Linear"]);
  for (const name of ["GitHub", "Slack", "Linear"]) {
    const row = surface(page, name);
    await expect(row.getByText("Not connected", { exact: true })).toBeVisible();
    await expect(row).toContainText(`Nothing in this workspace can send to or read from ${name} until it is connected.`);
    await expect(row.getByRole("button", { name: `Connect ${name}`, exact: true })).toBeDisabled();
  }
  expect(w.problems).toEqual([]);
});

test("a bad GitHub token is refused with the provider's reason and the surface stays not connected", async ({ page }) => {
  const w = watch(page, true);
  await open(page);
  await connectGithub(page, BAD_TOKEN);
  const row = surface(page, "GitHub");
  await expect(row.getByRole("alert")).toContainText("Bad credentials");
  await expect(row.getByText("Not connected", { exact: true })).toBeVisible();
  await expect(row.getByRole("button", { name: "Remove the GitHub connection…" })).toHaveCount(0);
  // Nothing the person typed comes back, even in a refusal.
  expect(await pageText(page)).not.toContain(BAD_TOKEN);
  expect((await w.bodies()).join("\n")).not.toContain(BAD_TOKEN);
  expect(w.problems).toEqual([]);
});

test("a good token connects GitHub, shows the account and the inbound URL, and the token is shown nowhere", async ({ page }) => {
  const w = watch(page);
  await open(page);
  await connectGithub(page, GOOD_TOKEN, WEBHOOK_SECRET);
  const row = await expectConnectedGithub(page);
  await expect(row.getByRole("button", { name: "Replace the GitHub connection", exact: true })).toBeVisible();
  // Slack and Linear are untouched.
  await expect(surface(page, "Slack").getByText("Not connected", { exact: true })).toBeVisible();
  await expect(surface(page, "Linear").getByText("Not connected", { exact: true })).toBeVisible();
  const text = await pageText(page);
  for (const secret of [GOOD_TOKEN, WEBHOOK_SECRET]) {
    expect(text).not.toContain(secret);
    expect((await w.bodies()).join("\n")).not.toContain(secret);
  }
  expect(w.problems).toEqual([]);
});

test("after a reload the connection is still shown, from the server", async ({ page }) => {
  const w = watch(page);
  await open(page);
  await expectConnectedGithub(page);
  await page.reload();
  await expect(connections(page).getByRole("listitem")).toHaveCount(3);
  await expectConnectedGithub(page);
  expect(await pageText(page)).not.toContain(GOOD_TOKEN);
  expect((await w.bodies()).join("\n")).not.toContain(GOOD_TOKEN);
  expect(w.problems).toEqual([]);
});

test("disconnecting GitHub returns it to not connected, and a reload agrees", async ({ page }) => {
  const w = watch(page);
  await open(page);
  await disconnectGithub(page);
  await expect(surface(page, "GitHub")).not.toContainText("octo-team");
  await expect(surface(page, "GitHub").getByText(/\/connections\/github\/webhook$/)).toHaveCount(0);
  await page.reload();
  await expect(surface(page, "GitHub").getByText("Not connected", { exact: true })).toBeVisible();
  expect(w.problems).toEqual([]);
});

test("an outbound webhook is added, listed with its secret shown once, and revoked", async ({ page }) => {
  const w = watch(page);
  await open(page);
  const panel = webhooks(page);
  const list = panel.getByRole("list", { name: "Webhooks", exact: true });
  const mine = list.getByRole("listitem").filter({ has: page.getByText(HOOK_LABEL, { exact: true }) });
  await expect(mine).toHaveCount(0);

  await panel.getByLabel("URL", { exact: true }).fill(HOOK_URL);
  await panel.getByLabel("Label (optional)").fill(HOOK_LABEL);
  await panel.locator("summary").filter({ hasText: /^Messages/ }).click();
  await panel.getByRole("checkbox", { name: "Message posted", exact: true }).check();
  await panel.getByRole("button", { name: "Add webhook", exact: true }).click();

  const secretBox = panel.getByRole("region", { name: `Signing secret for ${HOOK_LABEL}`, exact: true });
  await expect(secretBox).toBeVisible();
  await expect(secretBox).toContainText("Shown once.");
  await expect(mine).toHaveCount(1);
  await expect(mine).toContainText("Message posted");
  const created = w.seen.find((s) => s.status === 201 && /\/webhooks$/.test(s.url));
  await w.bodies();
  const minted = JSON.parse((w.seen.find((s) => /\/webhooks$/.test(s.url) && s.body.includes('"secret"')) ?? created)?.body ?? "{}") as { secret?: string };
  expect(minted.secret, "the create answer carries the signing secret").toBeTruthy();
  // Hidden until asked for, then the real value.
  await expect(secretBox).not.toContainText(minted.secret ?? "");
  await secretBox.getByRole("button", { name: "Show", exact: true }).click();
  await expect(secretBox).toContainText(minted.secret ?? "");
  await secretBox.getByRole("button", { name: "I have it", exact: true }).click();
  await expect(secretBox).toHaveCount(0);

  // Gone for good: neither the page nor a fresh read of the list has it.
  await page.reload();
  await expect(mine).toHaveCount(1);
  expect(await pageText(page)).not.toContain(minted.secret ?? "");
  const afterReload = w.seen.filter((s) => s.status === 200 && /\/webhooks$/.test(s.url)).map((s) => s.body);
  expect(afterReload.length).toBeGreaterThan(0);
  expect(afterReload.join("\n")).not.toContain(minted.secret ?? "");

  await mine.getByRole("button", { name: `Revoke the webhook for ${HOOK_LABEL}…`, exact: true }).click();
  await mine.getByRole("button", { name: "Revoke", exact: true }).click();
  await expect(mine).toHaveCount(0);
  await page.reload();
  await expect(panel.getByRole("heading", { name: "Webhooks", exact: true })).toBeVisible();
  await expect(mine).toHaveCount(0);
  expect(w.problems).toEqual([]);
});

const VIEWS = [
  { name: "connections-1280x800-dark", theme: "dark", viewport: { width: 1280, height: 800 } },
  { name: "connections-1280x800-light", theme: "light", viewport: { width: 1280, height: 800 } },
  { name: "connections-390x844-light", theme: "light", viewport: { width: 390, height: 844 } },
] as const;

for (const view of VIEWS) {
  test(`connect and disconnect GitHub at ${view.viewport.width}x${view.viewport.height}, ${view.theme}`, async ({ page }, testInfo) => {
    const w = watch(page);
    await open(page, view.theme, view.viewport);
    await connectGithub(page, GOOD_TOKEN, WEBHOOK_SECRET);
    const row = await expectConnectedGithub(page);
    await row.scrollIntoViewIfNeeded();
    // The inbound URL is the longest unbroken thing on the screen; it has to
    // wrap inside the card rather than push the page sideways.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow, "the page scrolls sideways").toBeLessThanOrEqual(0);
    const url = row.getByText(/^https?:\/\/.*\/connections\/github\/webhook$/);
    const box = await url.boundingBox();
    const card = await row.boundingBox();
    expect(box && card && box.x + box.width <= card.x + card.width + 1, "the inbound URL runs out of its row").toBe(true);
    expect(await pageText(page)).not.toContain(GOOD_TOKEN);
    await shot(page, testInfo, view.name);
    await disconnectGithub(page);
    expect((await w.bodies()).join("\n")).not.toContain(GOOD_TOKEN);
    expect(w.problems).toEqual([]);
  });
}
