import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// Mint is a real POST, not only the widening warning. The secret is shown
// once in the token field and is not written to localStorage. Revoke uses
// the session, so the new secret is taken out of the field first: while it
// sits there the page would send it as the bearer, and that token cannot
// administer tokens.
test("minting a token shows the secret once and revoke ends it", async ({ page, request }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="tokens"]');

  await expect(page.locator("#token-member")).toHaveValue(fx.member_id);
  await page.fill("#token-label", "ui-spec");
  await page.fill("#token-caps", "workspace:read");
  await page.click("#mint-member-token");
  await expect(page.locator("#toasts .toast-success", { hasText: "Token minted" })).toHaveAttribute("role", "status");

  const secret = await page.locator("#token").inputValue();
  expect(secret.length).toBeGreaterThan(20);
  expect(secret).not.toBe(fx.admin_token);
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeNull();
  await expect(page.locator("#token-revoke-id")).not.toHaveValue("");

  await page.locator("#token").evaluate((el: HTMLInputElement) => {
    el.value = "";
  });
  await page.click("#revoke-token");
  await expect(page.locator("#toasts .toast-success", { hasText: "Token revoked" })).toHaveAttribute("role", "status");

  const me = await request.get(`${fx.base_url}/me`, {
    headers: { Authorization: `Bearer ${secret}` },
  });
  expect(me.status()).toBe(401);
});
