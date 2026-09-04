import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Session-chrome badges in a real browser (Cluster 353.2): every thread in the
// list carries a state badge. The seeded fixture thread has a pending, schemaless
// approval gate, so it must render "needs-approval" (the gate path takes
// precedence over running/idle). The gate_id fixture is never resolved by another
// spec, so this is deterministic.
test("the thread list badges the gated fixture thread as needs-approval", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");

  // Selecting the seeded channel loads its threads (with chrome badges).
  await page.click(`#channel-list li[data-id="${fx.channel_id}"]`);

  const row = page.locator(`#thread-list li[data-id="${fx.thread_id}"]`);
  await expect(row).toBeVisible();
  const badge = row.locator(".chrome-badge");
  await expect(badge).toHaveText("needs-approval");
  // The state also rides the row dataset for scripting/assertions.
  await expect(row).toHaveAttribute("data-chrome", "needs-approval");

  // The legend explains all five states.
  await expect(page.locator(".chrome-legend")).toContainText("running");
  await expect(page.locator(".chrome-legend")).toContainText("done");
});
