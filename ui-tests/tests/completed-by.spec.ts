import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

// Closing clears the holder, so the header reads who finished the task from
// the thread's transition record.
test("a closed task's header says who completed it, and when", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.board_done_thread_id}"]`);
  const fact = page.locator("#thread-facts .fact", { hasText: "completed by" });
  await expect(fact).toHaveCount(1);
  await expect(fact.locator("time")).toHaveAttribute("datetime", /^\d{4}-\d{2}-\d{2}T/);
  await expect(page.locator("#thread-facts .fact", { hasText: "held by" })).toHaveCount(0);
});

test("an open task's header has no completed-by line", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.board_claimed_thread_id}"]`);
  await expect(page.locator("#thread-facts .fact", { hasText: "held by" })).toHaveCount(1);
  await expect(page.locator("#thread-facts .fact", { hasText: /completed by|closed by/ })).toHaveCount(0);
});
