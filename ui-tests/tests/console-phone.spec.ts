import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// The classic board's phone polish, held on the console: a focus ring on
// fields, no sideways scroll, a task sheet that puts itself away on Escape,
// and Needs you buttons that share one line.

test("a focused sign-in field shows the 3px focus ring", async ({ page }) => {
  await page.goto("/ui/#/board");
  const field = page.getByLabel(/paste a token/i);
  await field.focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(field).toBeFocused();
  await expect(field).toHaveCSS("outline-style", "solid");
  await expect(field).toHaveCSS("outline-width", "3px");
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("the board does not scroll sideways", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    await expect(page.locator("button.task").first()).toBeVisible();
    const sideways = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(sideways).toBeLessThanOrEqual(0);
  });

  test("Escape puts the task sheet away and returns to the task", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    await page.getByRole("tab", { name: /Done/ }).click();
    const task = page.locator("button.task", { hasText: "Done: closed" });
    await task.click();
    const sheet = page.getByRole("dialog");
    await expect(sheet).toBeVisible();
    await expect(sheet).toContainText(/Completed by|Closed by/);
    await page.keyboard.press("Escape");
    await expect(sheet).toBeHidden();
    await expect(task).toBeFocused();
  });

  test("a Needs you row's buttons share one line", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, "#/needs");
    const rows = page.locator(".need-actions");
    test.skip((await rows.count()) === 0, "nothing waiting in this seed");
    const tops = await rows.first().locator(".btn").evaluateAll((els) => els.map((e) => Math.round(e.getBoundingClientRect().top)));
    expect(new Set(tops).size).toBe(1);
  });
});
