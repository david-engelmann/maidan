import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// Every signed-in screen at the two phone widths the 2026-11-09 pass on
// maidan.world measured. That pass could only reach the public pages; the
// screens behind sign-in are checked here, on the seeded workspace, for the
// three defects it found: a page that scrolls sideways, a field under 16px
// (iOS zooms the page when one takes focus), and a button under the 44px
// touch target the console promises.
const SCREENS = ["board", "needs", "threads", "people", "activity", "tools", "catalog", "artifacts"] as const;

const WIDTHS = [
  { width: 360, height: 780 },
  { width: 390, height: 844 },
] as const;

/** Wait until the screen has drawn and stopped loading. */
async function settled(page: Page): Promise<void> {
  const main = page.locator("#main");
  await expect(main).not.toBeEmpty();
  await expect(main.locator('[aria-busy="true"], .ghost-line')).toHaveCount(0);
}

async function sideways(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
}

/** Visible fields whose text is under 16px, by name and size. */
async function smallFields(page: Page): Promise<string[]> {
  return page
    .locator("input:not([type=checkbox]):not([type=radio]):not([type=hidden]), select, textarea")
    .evaluateAll((els) =>
      els
        // A visually hidden field (a file input behind its label) is 1px and
        // never takes text focus.
        .filter((e) => {
          const r = e.getBoundingClientRect();
          return r.width > 1 && r.height > 1;
        })
        .map((e) => ({
          name: e.getAttribute("placeholder") ?? e.getAttribute("aria-label") ?? e.closest("label")?.textContent?.trim().slice(0, 24) ?? e.tagName,
          size: parseFloat(getComputedStyle(e).fontSize),
        }))
        .filter((f) => f.size < 16)
        .map((f) => `${f.name} ${f.size}px`),
    );
}

/**
 * Visible, enabled buttons and tabs smaller than 44px on either side, by name
 * and size. Lists use `content-visibility: auto`, so a control below the fold
 * has no real layout until it nears the viewport: the page is scrolled a
 * screen at a time and only what is on screen is measured.
 */
async function smallTargets(page: Page): Promise<string[]> {
  const found = new Set<string>();
  const screens = await page.evaluate(() => Math.ceil(document.documentElement.scrollHeight / window.innerHeight));
  for (let i = 0; i < Math.min(screens, 12); i++) {
    await page.evaluate((n) => window.scrollTo(0, n * window.innerHeight * 0.8), i);
    await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
    const small = await page.locator("button:not([disabled]), [role=tab]").evaluateAll((els) =>
      els
        .map((e) => ({ e, r: e.getBoundingClientRect() }))
        .filter(({ e, r }) => r.width > 1 && r.height > 1 && getComputedStyle(e).visibility !== "hidden")
        .filter(({ r }) => r.top >= 0 && r.bottom <= window.innerHeight)
        .filter(({ r }) => Math.round(r.height) < 44 || Math.round(r.width) < 44)
        .map(({ e, r }) => {
          const name = (e.getAttribute("aria-label") ?? e.textContent ?? e.className).trim().slice(0, 32);
          return `${name} ${Math.round(r.width)}x${Math.round(r.height)}`;
        }),
    );
    small.forEach((s) => found.add(s));
  }
  return [...found];
}

for (const size of WIDTHS) {
  test.describe(`signed in, on a ${size.width}px phone`, () => {
    test.use({ viewport: size, isMobile: true, hasTouch: true, reducedMotion: "reduce" });

    for (const screen of SCREENS) {
      test(`${screen}: no sideways scroll, no field under 16px, no target under 44px`, async ({ page }) => {
        await signInWithCookie(page, fx);
        await openOnChannel(page, fx.board_channel_id, `#/${screen}`);
        await settled(page);
        await expect.poll(() => sideways(page), { message: "the page scrolls sideways" }).toBeLessThanOrEqual(0);
        expect(await smallFields(page), "fields under 16px").toEqual([]);
        expect(await smallTargets(page), "buttons under 44px").toEqual([]);
      });
    }
  });
}
