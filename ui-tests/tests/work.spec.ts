import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Cluster 367.1 (Wave 2 #15): the Work tab. Proves the tab loads its channel
// selector from the seeded workspace and renders queue depth + threads for the
// selected channel — the live fetch→render the static ui_js_contract can't catch.
test("the Work tab shows queue depth and threads for a channel", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);

  // Open the Work tab (loads channels + schedules).
  await page.click('.tabs button[data-tab="work"]');
  await expect(page.locator("#panel-work")).toBeVisible();

  // The seeded channel appears as an option; select it.
  const channel = page.locator("#work-channel");
  await expect(channel.locator(`option[value="${fx.channel_id}"]`)).toBeAttached();
  await channel.selectOption(fx.channel_id);

  // Queue depth renders for the selected channel (a live fetch → render).
  await expect(page.locator("#work-depth")).toContainText("Queue —");

  // The seeded thread renders in the thread list, and Inspect loads its detail.
  const threadItem = page.locator("#work-thread-list li").first();
  await expect(threadItem).toBeVisible();
  await threadItem.getByRole("button", { name: "Inspect" }).click();
  await expect(page.locator("#work-thread-detail")).toContainText("Thread");
});
