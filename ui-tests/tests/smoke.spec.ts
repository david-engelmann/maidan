import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

// Proves the harness end-to-end: the real /ui loads, its single <script> runs,
// and a bearer-authed fetch → render cycle works. This is the class of bug the
// static ui_js_contract check cannot catch (a broken handler, a wrong endpoint,
// a render that never paints).
test("the /ui console loads its JS and renders seeded data", async ({ page }) => {
  await page.goto("/ui/");

  // The page + its script are present (the tab bar).
  await openMoreTools(page);
  await expect(page.locator('.tabs button[data-tab="notifications"]')).toBeVisible();

  // Authenticate (bearer) + point at the seeded workspace, then load channels.
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");

  // The seeded "general" channel renders into the list — a live fetch → render.
  await expect(page.locator("#channel-list li").filter({ hasText: "general" })).toBeVisible();
});

test("primary lists expose loading, actionable empty, and API error detail states", async ({ page }) => {
  let response: "empty" | "error" = "empty";
  await page.route(/\/workspaces\/[^/]+\/channels$/, async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 250));
    if (response === "empty") {
      await route.fulfill({ status: 200, contentType: "application/json", body: "[]" });
    } else {
      await route.fulfill({
        status: 403,
        contentType: "application/problem+json",
        body: JSON.stringify({ title: "Forbidden", detail: "channel access was revoked" }),
      });
    }
  });

  // The test server offers sessions and would refuse "test-token", and a
  // refused exchange leaves no credential, so the channel list would never be
  // asked for. Answer the exchange as a server without sessions does (404):
  // the token stays in this tab and the stubbed channel route is what loads.
  await page.route(/\/auth\/session\/from-token$/, (route) =>
    route.fulfill({ status: 404, contentType: "application/json", body: "{}" }),
  );

  await page.goto("/ui/");
  await page.fill("#workspace", "00000000-0000-0000-0000-000000000001");
  await page.fill("#token", "test-token");
  // Since #1257 a pasted token connects only on Sign in or Enter.
  await page.click("#token-signin");
  await page.click("#refresh-channels");
  await expect(page.locator("#channel-list")).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#channel-list")).toContainText("Loading channels");
  await expect(page.locator("#channel-list")).toContainText("Create one above");
  await expect(page.locator("#channel-list")).not.toHaveAttribute("aria-busy", "true");

  response = "error";
  await page.click("#refresh-channels");
  // A 403 is the human sentence. The server detail stays off the list.
  // Showing "channel access was revoked", or an HTTP status, fails this.
  const list = page.locator("#channel-list");
  await expect(list).toContainText("Could not load channels: Your token is not allowed to do this. Mint one with the right capability in Tokens");
  await expect(list).not.toContainText("channel access was revoked");
  await expect(list).not.toContainText("403");
  await expect(list).not.toContainText("HTTP");
});
