import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

async function signIn(page: Page, token: string) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", token);
  await page.locator("#token").dispatchEvent("change");
  await page.click("#refresh-channels");
}

// The first screen is the board, not a connection panel. Preset, reconnect
// and the raw feed stay in a closed menu. A refused subscribe stays one
// line: the sentence is there for the reader, and it does not grow a banner.
test("the live controls and a refused subscribe stay off the first screen", async ({ page }) => {
  await signIn(page, fx.token);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);

  const bar = page.locator("#live-panel");
  await expect(page.locator("#ws-connect")).toBeVisible();
  await expect(page.locator("#ws-connect")).not.toHaveClass(/primary/);
  await expect(page.locator("#ws-preset")).toBeHidden();
  await expect(page.locator("#ws-auto-reconnect")).toBeHidden();
  await expect(page.locator("#live-feed")).toBeHidden();
  await expect(bar).not.toContainText("Connect to update the board");
  const before = await bar.boundingBox();
  expect(before!.height).toBeLessThan(64);

  await page.click("#ws-connect");
  const status = page.locator("#ws-status");
  await expect(status).toContainText("missing event:subscribe capability");
  await expect(status).toHaveClass(/error/);
  await expect(status).toHaveCSS("white-space", "nowrap");
  await expect(status).toHaveCSS("text-overflow", "ellipsis");
  const after = await bar.boundingBox();
  expect(after!.height).toBeLessThan(64);
  // A refused close has its own banner. A refused subscribe must not open it.
  await expect(page.locator("#board-refusal")).toBeHidden();
});

// A decision row has one filled button. The other action is a quiet control,
// and opening the change note moves the filled button onto Send back.
test("a needs-you row has one primary button", async ({ page }) => {
  await signIn(page, fx.review_token);
  const rows = page.locator("#needs-you-list .ny-item");
  await expect(rows.first()).toBeVisible();
  const n = await rows.count();
  expect(n).toBeGreaterThan(0);
  for (let i = 0; i < n; i++) {
    await expect(rows.nth(i).locator("button.primary")).toHaveCount(1);
  }

  const review = page.locator('#needs-you-list .ny-item[data-kind="review_request"]').first();
  await expect(review.getByRole("button", { name: "Approve" })).toHaveClass(/primary/);
  await expect(review.getByRole("button", { name: "Request changes" })).toHaveClass(/ghost/);
  await expect(review.getByRole("button", { name: "Request changes" })).not.toHaveClass(/primary/);

  await review.getByRole("button", { name: "Request changes" }).click();
  await expect(review.locator("button.primary")).toHaveCount(1);
  await expect(review.getByRole("button", { name: "Send back" })).toHaveClass(/primary/);
  await expect(review.getByRole("button", { name: "Approve" })).not.toHaveClass(/primary/);

  await review.locator(".ny-note input").press("Escape");
  await expect(review.locator(".ny-note")).toHaveCount(0);
  await expect(review.getByRole("button", { name: "Approve" })).toHaveClass(/primary/);
  await expect(review.locator("button.primary")).toHaveCount(1);
});

// Nothing waiting is a sentence, not the amber card the queue uses when a
// decision is actually sitting there.
test("an empty needs-you queue is a quiet line", async ({ page }) => {
  await page.route("**/waiting", (route) =>
    route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify({ items: [] }),
    }),
  );
  await signIn(page, fx.review_token);

  const box = page.locator("#needs-you");
  await expect(box).toBeVisible();
  await expect(box).toHaveClass(/clear/);
  await expect(page.locator("#needs-you-quiet")).toBeVisible();
  await expect(page.locator("#needs-you-quiet")).toHaveText("Nothing is waiting on you.");
  await expect(page.locator("#needs-you-head")).toBeHidden();
  await expect(page.locator("#needs-you-list .ny-item")).toHaveCount(0);
  await expect(box).toHaveCSS("border-top-width", "0px");
  await expect(box).toHaveCSS("box-shadow", "none");
  const bg = await box.evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(bg).toBe("rgba(0, 0, 0, 0)");
});
