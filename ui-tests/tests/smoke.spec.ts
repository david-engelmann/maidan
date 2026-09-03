import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Proves the harness end-to-end: the real /ui loads, its single <script> runs,
// and a bearer-authed fetch → render cycle works. This is the class of bug the
// static ui_js_contract check cannot catch (a broken handler, a wrong endpoint,
// a render that never paints).
test("the /ui console loads its JS and renders seeded data", async ({ page }) => {
  await page.goto("/ui/");

  // The page + its script are present (version marker + the tab bar).
  await expect(page.locator('[data-ui-version="7"]')).toBeAttached();
  await expect(page.locator('.tabs button[data-tab="notifications"]')).toBeVisible();

  // Authenticate (bearer) + point at the seeded workspace, then load channels.
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");

  // The seeded "general" channel renders into the list — a live fetch → render.
  await expect(page.locator("#channel-list li").filter({ hasText: "general" })).toBeVisible();
});
