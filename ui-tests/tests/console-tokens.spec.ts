import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

// Tokens from the member page: a minted secret is shown once, revoke ends a
// token, rotate retires the old secret, a rotation of the page's own token
// moves the page onto the new one, and a grant cannot be widened. Each test
// mints its own throwaway token so a retry never touches a shared one.
const fx = fixtures();

async function mint(request: APIRequestContext, label: string, capabilities: string[]): Promise<{ id: string; secret: string }> {
  const res = await request.post(`/workspaces/${fx.workspace_id}/members/${fx.member_id}/tokens`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { label, capabilities },
  });
  expect(res.ok()).toBeTruthy();
  return res.json();
}

async function meStatus(request: APIRequestContext, secret: string): Promise<number> {
  return (await request.get("/me", { headers: { Authorization: `Bearer ${secret}` } })).status();
}

async function pasteIn(page: Page, secret: string, hash: string): Promise<void> {
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(secret);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
}

const tokenRow = (page: Page, label: string) => page.locator("li.token", { has: page.locator(".token-head b", { hasText: label }) });

test("a minted secret is shown once, masked, and gone after I have it", async ({ page, request }) => {
  await pasteIn(page, fx.admin_token, `#/people/m/${fx.member_id}`);
  const label = `mint-once-${Date.now()}`;
  await page.getByRole("checkbox", { name: "Agent worker preset" }).uncheck();
  await page.getByLabel("Label").fill(label);
  await page.getByRole("textbox", { name: "Capabilities" }).fill("workspace:read");
  await page.getByRole("button", { name: "Mint" }).click();
  const once = page.locator(".secret-once");
  await expect(once).toBeVisible();
  await expect(once.getByLabel("The secret, hidden")).toHaveText(/^•+$/);
  await once.getByRole("button", { name: "Show" }).click();
  const secret = (await once.getByLabel("The secret", { exact: true }).textContent()) ?? "";
  expect(secret.length).toBeGreaterThan(16);
  expect(await meStatus(request, secret)).toBe(200);
  await once.getByRole("button", { name: "I have it" }).click();
  await expect(once).toHaveCount(0);
  await expect(tokenRow(page, label)).toBeVisible();
  expect(await page.content()).not.toContain(secret);
  await page.reload();
  await expect(page.locator(".secret-once")).toHaveCount(0);
  expect(await page.evaluate(() => JSON.stringify({ ...localStorage }) + JSON.stringify({ ...sessionStorage }))).not.toContain(secret);
});

test("revoke asks once more, then the token stops working", async ({ page, request }) => {
  const label = `revoke-${Date.now()}`;
  const t = await mint(request, label, ["workspace:read"]);
  await pasteIn(page, fx.admin_token, `#/people/m/${fx.member_id}`);
  const row = tokenRow(page, label);
  await row.getByRole("button", { name: "Revoke" }).click();
  await expect(row.getByRole("button", { name: "Revoke: sure?" })).toBeVisible();
  expect(await meStatus(request, t.secret), "one press does not revoke").toBe(200);
  await row.getByRole("button", { name: "Revoke: sure?" }).click();
  await expect(row.locator(".state-chip.revoked")).toHaveText("Revoked");
  expect(await meStatus(request, t.secret)).toBe(401);
});

test("rotate gives a new secret once and retires the old one", async ({ page, request }) => {
  const label = `rotate-${Date.now()}`;
  const t = await mint(request, label, ["workspace:read"]);
  await pasteIn(page, fx.admin_token, `#/people/m/${fx.member_id}`);
  const row = tokenRow(page, label);
  await row.getByRole("button", { name: "Rotate" }).click();
  await row.getByRole("button", { name: "Rotate: sure?" }).click();
  const once = page.locator(".secret-once");
  await expect(once).toContainText("The old one no longer works");
  await once.getByRole("button", { name: "Show" }).click();
  const fresh = (await once.getByLabel("The secret", { exact: true }).textContent()) ?? "";
  expect(fresh).not.toBe(t.secret);
  expect(await meStatus(request, t.secret), "the old secret stopped working").toBe(401);
  expect(await meStatus(request, fresh)).toBe(200);
  expect(await page.evaluate(async () => (await fetch("/me", { credentials: "include" })).status), "the admin's own page kept working").toBe(200);
});

test("rotating the token this page runs on moves the page onto the new secret", async ({ page, request }) => {
  const own = await mint(request, `own-${Date.now()}`, ["workspace:read", "token:admin"]);
  await pasteIn(page, own.secret, `#/people/m/${fx.member_id}`);
  const row = page.locator("li.token", { has: page.locator(".state-chip", { hasText: "This page" }) });
  await row.getByRole("button", { name: "Rotate" }).click();
  await row.getByRole("button", { name: "Rotate: sure?" }).click();
  await expect(page.locator(".secret-once")).toBeVisible();
  await expect.poll(() => meStatus(request, own.secret)).toBe(401);
  await expect.poll(() => page.evaluate(async () => (await fetch("/me", { credentials: "include" })).status)).toBe(200);
  await page.reload();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await expect(page.getByLabel(/paste a token/i)).toHaveCount(0);
});

test("a rotation that lands after the page signed out does not sign it back in", async ({ page, request }) => {
  const own = await mint(request, `stale-${Date.now()}`, ["workspace:read", "token:admin"]);
  await pasteIn(page, own.secret, `#/people/m/${fx.member_id}`);
  let release: () => void = () => {};
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/tokens/*/rotate", async (route) => {
    await held;
    await route.continue();
  });
  const trades: string[] = [];
  page.on("request", (r) => {
    if (r.url().endsWith("/auth/session/from-token")) trades.push(r.url());
  });
  // Signing out may cancel the held request, or the server may refuse it
  // once the credential is gone; either way the page must stay signed out.
  const settled = new Promise<"done" | "dropped">((r) => {
    page.on("requestfinished", (q) => {
      if (q.url().includes("/rotate")) void q.response().then((res) => { r(res && res.ok() ? "done" : "dropped"); });
    });
    page.on("requestfailed", (q) => { if (q.url().includes("/rotate")) r("dropped"); });
  });
  const row = page.locator("li.token", { has: page.locator(".state-chip", { hasText: "This page" }) });
  await row.getByRole("button", { name: "Rotate" }).click();
  await row.getByRole("button", { name: "Rotate: sure?" }).click();
  // Sign out in another view while the rotation is in flight.
  await page.goto("/ui/#/tools/prefs");
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByLabel(/paste a token/i)).toBeVisible();
  release();
  if ((await settled) === "done") {
    await expect.poll(() => meStatus(request, own.secret), { message: "the rotation finished" }).toBe(401);
  }
  await page.waitForTimeout(500);
  expect(trades, "the stale rotation installed no session").toEqual([]);
  await expect(page.getByLabel(/paste a token/i)).toBeVisible();
  expect(await page.evaluate(async () => (await fetch("/me", { credentials: "include" })).status)).toBe(401);
});

test("the mint form will not widen the grant of the token acting", async ({ page, request }) => {
  // A narrow admin: it may mint, but only within workspace:read and token:admin.
  const narrow = await mint(request, `narrow-${Date.now()}`, ["workspace:read", "token:admin"]);
  await pasteIn(page, narrow.secret, `#/people/m/${fx.member_id}`);
  await page.getByRole("checkbox", { name: "Agent worker preset" }).uncheck();
  await page.getByRole("textbox", { name: "Capabilities" }).fill("workspace:read, thread:transition");
  await expect(page.getByRole("alert").filter({ hasText: "Your token does not hold thread:transition, so it cannot grant it." })).toBeVisible();
  await expect(page.getByRole("button", { name: "Mint" })).toBeDisabled();
  await page.getByRole("textbox", { name: "Capabilities" }).fill("workspace:read");
  await expect(page.getByRole("button", { name: "Mint" })).toBeEnabled();
});
