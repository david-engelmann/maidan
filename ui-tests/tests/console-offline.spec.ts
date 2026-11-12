import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// UI-008: offline, every place keeps what it loaded under one line saying it
// may be out of date; back online, the line goes.
const fx = fixtures();

test("offline, one line marks every place's data as possibly stale; online, it goes", async ({ page, context }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/board");
  await expect(page.locator(".lane").first()).toBeVisible();
  const banner = page.locator(".offline-banner");
  await expect(banner).toHaveCount(0);

  await context.setOffline(true);
  await expect(banner).toHaveText(/^You're offline\. What you see may be out of date/);
  // The board's own stale line would say the same thing twice.
  await expect(page.locator(".stale-banner:not(.offline-banner)")).toHaveCount(0);
  await page.evaluate(() => (window.location.hash = "#/threads"));
  await expect(banner).toBeVisible();

  await context.setOffline(false);
  await expect(banner).toHaveCount(0);
});
