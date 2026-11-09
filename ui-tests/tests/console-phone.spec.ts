import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";
import { laidOutBox } from "./_layout";

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
    const actions = rows.first().locator(".btn");
    await laidOutBox(actions.first());
    await expect.poll(async () => {
      const tops = await actions.evaluateAll((els) =>
        els.map((e) => {
          const r = e.getBoundingClientRect();
          return r.width > 0 && r.height > 0 ? Math.round(r.top) : null;
        }),
      );
      return tops.length > 0 && tops.every((t) => t !== null) && new Set(tops).size === 1;
    }).toBe(true);
  });
});

// The 2026-11-09 phone-width pass on maidan.world: a field under 16px makes
// iOS zoom the page when it takes focus, and at 360px the header's title,
// live chip and Search overran one row and the sticky header doubled.
async function smallFields(page: Page): Promise<string[]> {
  return page.locator("input:not([type=checkbox]):not([type=radio]):not([type=hidden]), select, textarea").evaluateAll((els) =>
    els
      .filter((e) => {
        const r = e.getBoundingClientRect();
        return r.width > 0 && r.height > 0;
      })
      .map((e) => ({
        name: e.getAttribute("placeholder") ?? e.getAttribute("aria-label") ?? e.closest("label")?.textContent?.trim().slice(0, 24) ?? e.tagName,
        size: parseFloat(getComputedStyle(e).fontSize),
      }))
      .filter((f) => f.size < 16)
      .map((f) => `${f.name} ${f.size}px`),
  );
}

test.describe("on a phone, fields", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("the board's filters, channel picker and new-task field are at least 16px", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    await expect(page.locator(".filter-q input")).toBeVisible();
    await page.locator(".board-bar .new-task").click();
    await expect(page.locator(".composer-field input")).toBeVisible();
    expect(await smallFields(page)).toEqual([]);
  });
});

test.describe("on a 360px phone", () => {
  test.use({ viewport: { width: 360, height: 780 }, isMobile: true, hasTouch: true });

  test("the signed-out header keeps its title and actions on one row", async ({ page }) => {
    await page.goto("/ui/");
    const h1 = page.locator(".top h1");
    const actions = page.locator(".top .top-actions");
    await expect(page.locator(".top .live")).toBeVisible();
    await expect.poll(async () => {
      const [a, b] = await Promise.all([h1.boundingBox(), actions.boundingBox()]);
      if (!a || !b) return false;
      return Math.abs(a.y + a.height / 2 - (b.y + b.height / 2)) < 12;
    }).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(0);
  });
});
