import { test, expect, APIRequestContext, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

// Rotating a token gives a new secret for the same authority and ends the old
// one. Each test mints its own throwaway token with the admin fixture, so a
// retry never rotates a token another spec depends on.
const fx = fixtures();

async function mintThrowaway(request: APIRequestContext): Promise<{ id: string; secret: string }> {
  const res = await request.post(`/workspaces/${fx.workspace_id}/members/${fx.member_id}/tokens`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { label: "rotate-spec", capabilities: ["workspace:read"] },
  });
  expect(res.ok()).toBeTruthy();
  return res.json();
}

async function meStatus(request: APIRequestContext, secret: string): Promise<number> {
  const res = await request.get("/me", { headers: { Authorization: `Bearer ${secret}` } });
  return res.status();
}

// What the page itself can do now, over its session cookie and no bearer.
async function pageMe(page: Page): Promise<{ status: number; tokenId: string | null }> {
  return page.evaluate(async () => {
    const res = await fetch("/me");
    const body = res.ok ? await res.json() : {};
    return { status: res.status, tokenId: body.token_id ?? null };
  });
}

test("the Session tab rotates the token this page runs on, and keeps working", async ({ page, request }) => {
  const old = await mintThrowaway(request);
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", old.secret);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator("#identity-pill")).toBeVisible();

  await page.click('.tabs button[data-tab="session"]');
  await expect(page.locator("#session-credential")).toHaveText("bearer token — acts as this member");
  await page.getByRole("button", { name: "Rotate this token" }).click();

  const banner = page.locator("#mint-banner");
  await expect(banner).toBeVisible();
  await expect(page.locator("#mint-title")).toContainText("New token (shown once)");
  const fresh = (await page.locator("#mint-secret").textContent()) ?? "";
  expect(fresh).not.toBe("");
  expect(fresh).not.toBe(old.secret);
  await page.click("#conn-edit");
  // The successor is exchanged for a new session. The page does not keep it.
  await expect(page.locator("#token")).toHaveValue("");
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeNull();
  expect(await page.evaluate(async () => (await fetch("/me")).status)).toBe(200);

  expect(await meStatus(request, old.secret), "the old secret stopped working").toBe(401);
  expect(await meStatus(request, fresh)).toBe(200);
});

// The Tokens tab rotating the page's own token, with the Session tab never
// opened, exchanges the successor too.
test("rotating the page's own token from the Tokens tab keeps the page working", async ({ page, request }) => {
  const own = await mintThrowaway(request);
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", own.secret);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator("#identity-pill")).toBeVisible();
  expect((await pageMe(page)).tokenId).toBe(own.id);

  await page.click('.tabs button[data-tab="tokens"]');
  await page.fill("#token-revoke-id", own.id);
  const exchanged = page.waitForResponse((r) => r.url().endsWith("/auth/session/from-token"));
  await page.getByRole("button", { name: "Rotate token" }).click();
  expect((await exchanged).status()).toBe(201);
  await expect(page.locator("#mint-title")).toContainText("New token (shown once)");

  const me = await pageMe(page);
  expect(me.status, "the page keeps working after rotating its own token").toBe(200);
  expect(me.tokenId).not.toBe(own.id);
  expect(await meStatus(request, own.secret)).toBe(401);
  const channels = await page.evaluate(
    async (ws) => (await fetch(`/ui/api/workspaces/${ws}/channels`)).status,
    fx.workspace_id,
  );
  expect(channels, "and the board still reads over the new session").toBe(200);
});

test("an admin rotates another token from the Tokens tab without losing their own", async ({ page, request }) => {
  const agent = await mintThrowaway(request);
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.admin_token);
  await page.locator("#token").dispatchEvent("change");
  await expect(page.locator("#identity-pill")).toBeVisible();

  await page.click('.tabs button[data-tab="tokens"]');
  await page.fill("#token-revoke-id", agent.id);
  await page.getByRole("button", { name: "Rotate token" }).click();

  await expect(page.locator("#mint-title")).toContainText("New token (shown once)");
  const fresh = (await page.locator("#mint-secret").textContent()) ?? "";
  expect(fresh).not.toBe(agent.secret);
  // Rotating someone else's token does not replace this browser's session,
  // and the admin secret was never stored.
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeNull();
  expect(await page.evaluate(async () => (await fetch("/me")).status)).toBe(200);
  expect(await meStatus(request, agent.secret)).toBe(401);
  expect(await meStatus(request, fresh)).toBe(200);
});
