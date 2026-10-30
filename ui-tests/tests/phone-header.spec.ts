import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

async function headerRows(page: import("@playwright/test").Page) {
  return page.evaluate(() => {
    const h = document.querySelector("header") as HTMLElement;
    const shown = [...h.children].filter((e) => (e as HTMLElement).offsetParent !== null);
    // Items of different heights centre on one line, so a new row starts only
    // where an item begins below the bottom of the row before it.
    const boxes = shown.map((e) => e.getBoundingClientRect()).sort((a, b) => a.top - b.top);
    let rows = 0;
    let bottom = -Infinity;
    for (const b of boxes) {
      if (b.top >= bottom - 2) { rows += 1; bottom = b.bottom; } else bottom = Math.max(bottom, b.bottom);
    }
    return { rows, height: h.getBoundingClientRect().height };
  });
}

for (const touch of [false, true]) {
  test(`a signed-in phone header is two rows (touch=${touch})`, async ({ browser }) => {
    const ctx = await browser.newContext({ viewport: { width: 390, height: 844 }, hasTouch: touch, isMobile: touch });
    const page = await ctx.newPage();
    await signIn(page, fx.workspace_id, fx.review_token);
    await expect(page.locator("#identity-pill")).toBeVisible();
    const { rows, height } = await headerRows(page);
    expect(rows).toBe(2);
    expect(height).toBeLessThan(touch ? 120 : 100);
    // The short labels keep their full names for assistive tech.
    await expect(page.getByRole("button", { name: /Search or jump/ })).toBeVisible();
    await expect(page.getByRole("button", { name: "Connect an agent" }).first()).toBeVisible();
    await expect(page.getByRole("heading", { level: 1, name: "Maidan" })).toBeVisible();
    // Nothing in the header runs off the side of the screen.
    const overflow = await page.evaluate(() => document.querySelector("header")!.scrollWidth - document.querySelector("header")!.clientWidth);
    expect(overflow).toBeLessThanOrEqual(0);
    await ctx.close();
  });
}

test("the desktop header keeps its full labels", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await signIn(page, fx.workspace_id, fx.review_token);
  await expect(page.locator("#palette-open .hdr-long")).toBeVisible();
  await expect(page.locator("#palette-open .hdr-icon")).toBeHidden();
  await expect(page.locator("header h1 span")).toHaveText("Maidan");
  expect((await headerRows(page)).rows).toBe(1);
});
