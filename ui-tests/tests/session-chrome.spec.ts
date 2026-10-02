import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// The seeded fixture thread has a pending, schemaless approval gate, so the
// board shows it as the word needs-approval in Needs review. The gate path
// takes precedence over in review, running, claimed, and open. A pill or a
// legend means the board is wrong. The gate fixture is never resolved by
// another spec, so this is deterministic.
test("the board shows the gated thread as needs-approval", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.click("#refresh-channels");

  await page.click(`#channel-list li[data-id="${fx.channel_id}"]`);

  const row = page.locator(`#board .board-col[data-column="review"] .card[data-id="${fx.thread_id}"]`);
  await expect(row).toBeVisible();
  await expect(row.locator(".card-state")).toHaveText("needs-approval");
  await expect(row.locator(".chrome-badge")).toHaveCount(0);
  await expect(row).toHaveAttribute("data-chrome", "needs-approval");
  await expect(page.locator(".chrome-legend")).toHaveCount(0);
});
