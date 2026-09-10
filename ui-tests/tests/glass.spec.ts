import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Cluster 367.3 (Wave 2 #15): the looking-glass explorer. A deterministic live
// fetch→render: an unknown sha resolves to not-found (a real 404 the static
// ui_js_contract check can't exercise).
test("the looking glass looks up an unknown artifact and shows not-found", async ({
  page,
}) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);

  await page.click('.tabs button[data-tab="glass"]');
  await expect(page.locator("#panel-glass")).toBeVisible();

  await page.fill("#glass-sha", "0".repeat(64));
  await page.click("#glass-sha-btn");
  await expect(page.locator("#glass-artifact")).toContainText("Not found");
});
