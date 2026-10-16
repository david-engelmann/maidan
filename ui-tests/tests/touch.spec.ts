import { test, expect, devices } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();
// A describe group may not switch the browser, only the viewport and input.
const { defaultBrowserType: _browser, ...phone } = devices["Pixel 5"];

// On a phone every control is a 44px target, the size WCAG 2.5.5 asks of a
// fingertip, and the Ctrl K hint is gone: there is no keyboard to press it on.
test.describe("on a touch screen", () => {
  test.use(phone);

  test("every visible button is at least 44px and no shortcut hint shows", async ({ page }) => {
    await signIn(page, fx.workspace_id, fx.token);
    await page.click("#refresh-channels");
    await page.click(`#channel-list li[data-id="${fx.board_channel_id}"]`);
    await expect(page.locator("#board .card").first()).toBeVisible();

    await expect(page.locator("#palette-open .kbd.mod-k")).toBeHidden();
    const create = await page.locator("#create-channel").boundingBox();
    expect(create).not.toBeNull();
    expect(create!.height).toBeGreaterThanOrEqual(44);
    expect(create!.width).toBeGreaterThanOrEqual(44);

    const sizes = await page.locator("button:visible, select:visible, #channel-list li:visible").evaluateAll((els) =>
      els.map((el) => {
        const r = el.getBoundingClientRect();
        return { id: el.id || el.className || el.textContent?.trim().slice(0, 20), h: Math.round(r.height) };
      }),
    );
    expect(sizes.length).toBeGreaterThan(5);
    expect(sizes.filter((b) => b.h > 0 && b.h < 44)).toEqual([]);
    // Bigger targets must not push the page wider than the phone.
    const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(overflow).toBeLessThanOrEqual(0);
  });
});

test("with a mouse the shortcut hint shows", async ({ page }) => {
  await page.goto("/ui/");
  await expect(page.locator("#palette-open .kbd.mod-k")).toBeVisible();
});
