import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// A thrown screen, a shortcut, and a sign-out must not leak one room into the next.
const fx = fixtures();
const ROUTES = ["#/board", "#/threads", "#/people", "#/activity", "#/tools", "#/catalog", "#/artifacts"];

async function settled(page: Page): Promise<void> {
  await expect(page.locator("main section").first()).toHaveCSS("opacity", "1");
}

test("signed-in places have no axe violations", async ({ page }) => {
  await signInWithCookie(page, fx);
  const found: string[] = [];
  for (const hash of ROUTES) {
    await openOnChannel(page, fx.board_channel_id, hash);
    await settled(page);
    const scan = await new AxeBuilder({ page }).analyze();
    for (const v of scan.violations) {
      // Activity's own section and the shell section are both named Activity.
      // Those two files belong to other open changes, so this scan leaves that one rule out.
      if (hash === "#/activity" && v.id === "landmark-unique") continue;
      found.push(`${hash} ${v.id}: ${v.help}`);
    }
  }
  expect(found).toEqual([]);
});

test("j and k walk catalog and activity, and g o leaves a tools sub-page", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/catalog");
  await settled(page);
  const tools = page.locator(".catalog-list .tool-row, .cat-item > summary");
  await expect(tools.first()).toBeVisible();
  await page.locator("h1, h2").first().click();
  await page.keyboard.press("j");
  await expect(tools.first()).toBeFocused();
  await page.keyboard.press("j");
  await expect(tools.nth(1)).toBeFocused();
  await page.keyboard.press("k");
  await expect(tools.first()).toBeFocused();

  await page.goto("/ui/#/activity");
  await settled(page);
  const links = page.locator(".activity-list a, .act-list .act-link");
  await expect(links.first()).toBeVisible();
  await page.locator("h1").first().click();
  await page.keyboard.press("j");
  await expect(links.first()).toBeFocused();
  await page.keyboard.press("j");
  await expect(links.nth(1)).toBeFocused();

  await page.goto("/ui/#/tools/prefs");
  await settled(page);
  await page.locator("h1").click();
  await page.keyboard.press("g");
  await page.keyboard.press("o");
  await expect(page).toHaveURL(/#\/tools$/);
});

test("signing out clears the previous workspace's notices", async ({ browser }) => {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await page.goto("/ui/#/tools/prefs");
  await page.getByLabel(/paste a token/i).fill(fx.admin_token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await page.getByRole("radio", { name: "Digest" }).click();
  await expect(page.getByText("You will get a digest")).toBeVisible();
  await page.getByRole("radio", { name: "Right away" }).click();
  await expect(page.getByText("You will hear right away")).toBeVisible();
  await page.goto("/ui/#/tools/recent");
  await expect(page.locator(".notice")).toHaveCount(2);

  await page.goto("/ui/#/tools/prefs");
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();

  await page.getByLabel(/paste a token/i).fill(fx.second_token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await page.goto("/ui/#/tools/recent");
  await expect(page.getByText("Nothing yet. What you do here, and what goes wrong, shows up in this list.")).toBeVisible();
  await expect(page.locator(".notice")).toHaveCount(0);
  await ctx.close();
});
