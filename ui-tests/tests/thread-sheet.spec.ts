import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

test.describe("phone", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("a selected task opens as a bottom sheet with a way back to the board", async ({ page }) => {
    await signIn(page, fx.workspace_id, fx.review_token);
    await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
    const card = page.locator(`#board .card[data-id="${fx.board_claimed_thread_id}"]`);
    await card.click();
    const panel = page.locator("#collab-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toHaveCSS("position", "fixed");
    const box = await panel.boundingBox();
    expect(box && Math.round(box.y + box.height)).toBeGreaterThanOrEqual(843);
    const back = page.getByRole("button", { name: "Back to the board", exact: true });
    await expect(back).toBeVisible();
    await back.click();
    await expect(panel).toBeHidden();
    await expect(card).toBeFocused();
  });
});

test("on a wide screen the panel stays in the layout and the back button is hidden", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 });
  await signIn(page, fx.workspace_id, fx.review_token);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  await page.click(`#board .card[data-id="${fx.board_claimed_thread_id}"]`);
  await expect(page.locator("#collab-panel")).toBeVisible();
  await expect(page.locator("#collab-panel")).not.toHaveCSS("position", "fixed");
  await expect(page.locator("#thread-close")).toBeHidden();
});
