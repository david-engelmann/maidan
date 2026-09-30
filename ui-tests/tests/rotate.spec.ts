import { test, expect, APIRequestContext } from "@playwright/test";
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
  await expect(page.locator("#token")).toHaveValue(fresh);
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBe(fresh);

  expect(await meStatus(request, old.secret), "the old secret stopped working").toBe(401);
  expect(await meStatus(request, fresh)).toBe(200);
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
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBe(fx.admin_token);
  expect(await meStatus(request, agent.secret)).toBe(401);
  expect(await meStatus(request, fresh)).toBe(200);
});
