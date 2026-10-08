import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

// These tests close a seeded task on the shared server, so a retry would find
// its row already gone. A failure here fails once, with its real cause.
test.describe.configure({ retries: 0 });

// A review that names nobody still reaches someone: its owner, or, when it has
// none, the workspace's admins. The owner's own approval does not count, so
// the owner's row offers what the owner can do. A thread with no review gate
// can close without an approval, and the board says so.

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

test("a review nobody owns reaches an admin, and says no result was posted", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  const ownerless = row(page, fx.triage_ownerless_thread_id);
  await expect(ownerless).toHaveAttribute("data-kind", "unassigned_review");
  await expect(ownerless.locator(".ny-kind")).toHaveText("Review");
  await expect(ownerless.locator(".ny-sub")).toContainText("no reviewer named");
  await expect(ownerless.locator(".ny-warn")).toHaveText("No result was posted");
  await expect(ownerless.getByRole("button", { name: "Approve", exact: true })).toHaveClass(/primary/);
  await expect(ownerless.locator("button.primary")).toHaveCount(1);
});

test("the owner of a review nobody was named for can close it, and the board says closed without review", async ({
  page,
}) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  const owned = row(page, fx.triage_owned_thread_id);
  await expect(owned).toHaveAttribute("data-kind", "unassigned_review");
  await expect(owned.locator(".ny-sub")).toContainText("no reviewer named");
  const close = owned.getByRole("button", { name: "Close without review" });
  await expect(close).toHaveClass(/primary/);
  await expect(owned.getByRole("button", { name: "Approve", exact: true })).toHaveCount(0);
  await expect(owned.getByRole("button", { name: "Approve with note" })).toHaveCount(0);
  await expect(owned.locator("button.primary")).toHaveCount(1);
  await expect(owned.locator(".ny-warn")).toHaveCount(0);

  await close.click();
  await expect(owned).toHaveCount(0);
  const res = await page.request.get(`${fx.base_url}/threads/${fx.triage_owned_thread_id}`, {
    headers: { Authorization: `Bearer ${fx.review_token}` },
  });
  const thread = await res.json();
  expect(thread.state).toBe("closed");
  expect(thread.closed_without_review).toBe(true);

  await page.click(`#channel-list li[data-id="${fx.triage_channel_id}"]`);
  const card = page.locator(`#board .board-col[data-column="done"] .card[data-id="${fx.triage_owned_thread_id}"]`);
  await expect(card.locator(".card-state")).toHaveText("closed without review");
  await expect(card).toHaveAttribute("aria-label", /closed without review$/);
});

test("an approved close stays plain done on the board", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await expect(page.locator(`#board .card[data-id="${fx.board_done_thread_id}"] .card-state`)).toHaveText("done");
});
