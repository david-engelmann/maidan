import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Activity: filter the live log, open a channel's board, and keep one
// workspace's events from showing up in another.
const fx = fixtures();

test("category, search and channel narrow the log, and a channel opens its board", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/activity");
  const list = page.locator(".activity-list");
  await expect(list.locator(".activity-row").first()).toBeVisible();
  const before = await list.locator(".activity-row").count();
  expect(before).toBeGreaterThan(0);

  await page.getByRole("button", { name: "Tasks", exact: true }).click();
  await expect(page.getByRole("button", { name: "Tasks", exact: true })).toHaveAttribute("aria-pressed", "true");

  await page.getByLabel("Filter activity").fill("zzz-no-such-event");
  await expect(page.getByRole("status").filter({ hasText: "No matching events" })).toBeVisible();
  await page.getByRole("button", { name: "Show all events" }).click();
  await expect(list.locator(".activity-row").first()).toBeVisible();
  await expect(page.getByLabel("Filter activity")).toHaveValue("");
  await expect(page.getByRole("button", { name: "All", exact: true })).toHaveAttribute("aria-pressed", "true");

  const link = page.locator("a.activity-channel").first();
  await expect(link).toBeVisible();
  const href = (await link.getAttribute("href")) ?? "";
  const channelId = new URLSearchParams(href.split("?")[1] ?? "").get("channel");
  expect(channelId).toBeTruthy();
  await page.getByLabel("Channel").selectOption(channelId ?? "");
  await expect(link).toBeVisible();
  await link.click();
  await expect(page).toHaveURL(new RegExp(`#/board\\?channel=${channelId}`));
});

test("switching workspace does not keep the previous room's events", async ({ browser }) => {
  const home = await browser.newContext();
  const page = await home.newPage();
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/activity");
  const title = page.locator(".activity-row a[href^='#/threads/']").first();
  await expect(title).toBeVisible();
  const marker = ((await title.textContent()) ?? "").trim();
  expect(marker.length).toBeGreaterThan(0);
  await home.close();

  const away = await browser.newContext();
  const other = await away.newPage();
  await other.goto("/ui/");
  await other.getByLabel(/paste a token/i).fill(fx.other_token);
  await other.getByRole("button", { name: "Sign in with token" }).click();
  await expect(other.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await other.goto("/ui/#/activity");
  await expect(other.locator(".activity-row", { hasText: marker })).toHaveCount(0);
  await away.close();
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("activity does not scroll sideways and its controls are at least 44px", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, "#/activity");
    await expect(page.getByLabel("Filter activity")).toBeVisible();
    const sideways = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(sideways).toBeLessThanOrEqual(1);
    const small = await page.locator(".activity button:visible, .activity input:visible, .activity select:visible, .activity a.activity-channel:visible").evaluateAll((els) =>
      els
        .map((e) => ({ t: (e.getAttribute("aria-label") || e.textContent || "").trim().slice(0, 40), r: e.getBoundingClientRect() }))
        .filter(({ r }) => r.width > 0 && r.height > 0 && (Math.round(r.height) < 44 || Math.round(r.width) < 44))
        .map(({ t, r }) => `${t} ${Math.round(r.width)}x${Math.round(r.height)}`),
    );
    expect(small).toEqual([]);
  });
});

test("activity has no axe violations", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/activity");
  await expect(page.locator(".activity-row").first()).toBeVisible();
  await expect(page.locator("main section").first()).toHaveCSS("opacity", "1");
  const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
});
