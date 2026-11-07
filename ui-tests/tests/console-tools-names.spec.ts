import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Tools should speak in task, channel and member names. A followed thread is
// picked from the workspace. Search links to the task by its channel and
// title; this seed's event log does not show the kinds Recent events lists.
const deploy = "#general \u2014 Deploy v9";
const review = "#build \u00b7 Review: result waiting on a reviewer";

function panel(page: Page, title: string) {
  return page.locator("section.tool-panel", { has: page.getByRole("heading", { name: title, exact: true }) });
}

test("threads you follow are chosen and shown by channel and title", async ({ page }) => {
  const live = fixtures();
  await signInWithCookie(page, live);
  await openOnChannel(page, live.board_channel_id, "#/tools/prefs");
  const threads = panel(page, "Threads you follow");
  const task = threads.getByRole("combobox", { name: "Task" });
  const row = threads.locator("li", { hasText: deploy });
  if (await row.count()) {
    await row.getByRole("button", { name: "Unfollow" }).click();
    await expect(row).toHaveCount(0);
  }
  await expect(task.locator("option", { hasText: deploy })).toHaveCount(1);
  await expect(threads.getByLabel("Thread id")).toHaveCount(0);
  await task.selectOption({ label: deploy });
  await threads.getByRole("button", { name: "Follow" }).click();
  await expect(row.getByRole("link", { name: deploy, exact: true })).toHaveAttribute("href", `#/threads/t/${live.thread_id}`);
  await row.getByRole("button", { name: "Unfollow" }).click();
  await expect(row).toHaveCount(0);
  await expect(task.locator("option", { hasText: deploy })).toHaveCount(1);
});

test("search names the task and who wrote it", async ({ page }) => {
  const live = fixtures();
  await signInWithCookie(page, live);
  await openOnChannel(page, live.board_channel_id, "#/tools/search");
  await page.getByLabel("Words").fill("result");
  await page.getByRole("button", { name: "Search", exact: true }).click();
  const hits = panel(page, "Search messages");
  const link = hits.getByRole("link", { name: review, exact: true });
  await expect(link).toBeVisible();
  await expect(link).toHaveAttribute("href", `#/threads/t/${live.board_review_thread_id}`);
  await expect(hits).toContainText("by Deployer");
  await expect(hits.getByRole("link", { name: "Open the thread" })).toHaveCount(0);
  await expect(link).not.toHaveText(/^thread [0-9a-f]{8}$/);
});
