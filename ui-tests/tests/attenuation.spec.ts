import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Attenuation chrome in a real browser (Cluster 353.3): a minted token can only
// be a subset of the caller's own grant. The fixture token holds
// workspace:read + workspace:write + message:post, so requesting token:admin is
// a widening attempt — flagged client-side before any request is sent (the
// server enforces the same rule via validate_subset).
test("minting cannot widen the caller's grant", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click('.tabs button[data-tab="tokens"]');

  // The ceiling reflects the fixture token's real grant.
  await expect(page.locator("#attenuation-ceiling")).toContainText("workspace:read");

  // A capability outside the grant is blocked before any POST.
  await page.fill("#token-caps", "token:admin");
  await page.click("#mint-member-token");
  const warn = page.locator("#attenuation-warning");
  await expect(warn).toBeVisible();
  await expect(warn).toContainText("token:admin");
});
