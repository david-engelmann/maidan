import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

for (const touch of [false, true]) {
  test(`on a phone a review row's three buttons share one line (touch=${touch})`, async ({ browser }) => {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: touch, isMobile: touch });
    const page = await ctx.newPage();
    await signIn(page, fx.workspace_id, fx.review_token);
    const row = page.locator('#needs-you-list .ny-item[data-kind="review_request"]').first();
    await expect(row.getByRole("button", { name: "Approve with note" })).toBeVisible();
    const tops = await row.locator(".ny-actions > button").evaluateAll((bs) => bs.map((b) => Math.round(b.getBoundingClientRect().top)));
    expect(tops.length).toBe(3);
    expect(new Set(tops).size).toBe(1);
    await ctx.close();
  });
}

test("on a phone a teammate's status wraps instead of being cut", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await signIn(page, fx.workspace_id, fx.review_token);
  await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
  const small = page.locator("#team .mate .mate-text small").first();
  await expect(small).toBeVisible();
  await expect(small).toHaveCSS("white-space", "normal");
  const fits = await page.locator("#team .mate .mate-text small").evaluateAll((els) => els.every((e) => e.scrollWidth <= e.clientWidth + 1));
  expect(fits).toBe(true);
});
