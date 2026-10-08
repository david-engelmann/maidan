import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

const fx = fixtures();

// The harness seeds one quarantined webhook. Replay clears the dead letter
// and the row comes back under Pending.
test("a dead-lettered delivery can be replayed from the operator tab", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.admin_token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="operator"]');

  const list = page.locator("#op-deliv-list");
  await page.selectOption("#op-deliv-status", "quarantined");
  await page.click("#op-deliv-refresh");
  await expect(list).toContainText(fx.delivery_url);
  await expect(list).toContainText("DLQ");
  await list.getByRole("button", { name: "Replay" }).click();
  await expect(
    page.locator("#toasts .toast-success", { hasText: `Replayed webhook delivery #${fx.delivery_id}` }),
  ).toHaveAttribute("role", "status");
  await expect(list).not.toContainText(fx.delivery_url);

  await page.selectOption("#op-deliv-status", "pending");
  await page.click("#op-deliv-refresh");
  await expect(list).toContainText(fx.delivery_url);
  await expect(list).not.toContainText("DLQ");
});
