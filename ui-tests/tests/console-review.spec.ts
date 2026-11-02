import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Needs you on a wide screen: a queue on the left, the picked item's evidence
// and actions on the right, and a and r acting on the picked item.
const fx = fixtures();

test("a wide screen reviews from a queue, and r sends back the picked item", async ({ page }) => {
  await page.setViewportSize({ width: 1680, height: 1000 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  const picks = page.locator(".need-pick");
  await expect(picks.nth(1)).toBeVisible();
  await expect(page.locator(".need-card:visible")).toHaveCount(1);
  const title = (await picks.nth(1).locator(".need-pick-title").textContent()) ?? "";
  await picks.nth(1).click();
  await expect(picks.nth(1)).toHaveAttribute("aria-current", "true");
  const pane = page.locator(".need.selected .need-card");
  await expect(pane.locator(".need-title")).toHaveText(title);
  const pickBox = await picks.nth(1).boundingBox();
  const paneBox = await pane.boundingBox();
  expect(paneBox!.x, "the detail sits to the right of the queue").toBeGreaterThan(pickBox!.x + pickBox!.width);
  await page.locator("body").press("r");
  await expect(pane.locator("textarea")).toBeFocused();
  await pane.getByRole("button", { name: "Cancel" }).click();
});

test("a narrow screen keeps one column of full cards", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.tiers_channel_id, "#/needs");
  await expect(page.locator(".need-card").first()).toBeVisible();
  await expect(page.locator(".need-pick:visible")).toHaveCount(0);
  expect(await page.locator(".need-card:visible").count()).toBeGreaterThan(1);
});
