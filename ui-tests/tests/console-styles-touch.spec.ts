import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

async function fits(page: Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);
}

async function shortTargets(page: Page): Promise<string[]> {
  return page.locator("button:visible, a[href]:visible, select:visible, summary:visible, [role='tab']:visible").evaluateAll((els) =>
    els
      .map((el) => {
        const r = el.getBoundingClientRect();
        const t = (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 28) || el.tagName.toLowerCase();
        return { t, w: Math.round(r.width), h: Math.round(r.height) };
      })
      .filter(({ w, h }) => w > 0 && h > 0 && (w < 44 || h < 44))
      .map(({ t, w, h }) => `${t} ${w}x${h}`),
  );
}

async function focusRing(page: Page): Promise<string> {
  const select = page.locator(".channel-pick select");
  await select.focus();
  return select.evaluate((el) => getComputedStyle(el).outlineStyle);
}

test.describe("on a desktop", () => {
  test.use({ viewport: { width: 1280, height: 800 } });

  test("a multi-channel board keeps a 44px channel select", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    const select = page.locator(".channel-pick select");
    await expect(select).toBeVisible();
    expect(await select.evaluate((el) => (el as HTMLSelectElement).options.length)).toBeGreaterThanOrEqual(2);
    const box = await select.boundingBox();
    expect(box).not.toBeNull();
    expect(Math.round(box!.width)).toBeGreaterThanOrEqual(44);
    expect(Math.round(box!.height)).toBeGreaterThanOrEqual(44);
    expect(await focusRing(page)).not.toBe("none");
    expect(await fits(page)).toBe(true);
  });
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, reducedMotion: "reduce" });

  test("board, tools and people keep a 44px target", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    const select = page.locator(".channel-pick select");
    await expect(select).toBeVisible();
    await expect.poll(() => select.evaluate((el) => (el as HTMLSelectElement).options.length)).toBeGreaterThanOrEqual(2);
    const next = await select.evaluate((el) => {
      const menu = el as HTMLSelectElement;
      return [...menu.options].map((o) => o.value).find((id) => id && id !== menu.value) ?? "";
    });
    expect(next).not.toBe("");
    await select.selectOption(next);
    await expect(select).toHaveValue(next);
    await expect(page.locator(".lane, .lanes").first()).toBeVisible();
    const box = await select.boundingBox();
    expect(Math.round(box!.width)).toBeGreaterThanOrEqual(44);
    expect(Math.round(box!.height)).toBeGreaterThanOrEqual(44);
    expect(await focusRing(page)).not.toBe("none");
    await expect(page.locator("#main > section")).toHaveCSS("opacity", "1");
    await expect(select).toHaveCSS("transition-duration", "0s");
    expect(await fits(page)).toBe(true);
    expect(await shortTargets(page)).toEqual([]);
    const board = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(board.violations.map((v) => v.id)).toEqual([]);

    await page.goto("/ui/#/tools");
    await expect(page.getByRole("tab", { name: "Inbox" })).toBeVisible();
    await expect(page.locator("#main > section")).toHaveCSS("opacity", "1");
    await expect(page.locator("button, a[role='tab']").first()).toHaveCSS("transition-duration", "0s");
    expect(await fits(page)).toBe(true);
    expect(await shortTargets(page)).toEqual([]);
    const tools = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(tools.violations.map((v) => v.id)).toEqual([]);

    await page.goto("/ui/#/people");
    await expect(page.getByRole("tab", { name: "People" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Add agent" })).toBeVisible();
    await expect(page.locator("#main > section")).toHaveCSS("opacity", "1");
    expect(await fits(page)).toBe(true);
    expect(await shortTargets(page)).toEqual([]);
    const people = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    expect(people.violations.map((v) => v.id)).toEqual([]);
  });
});
