import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Needs you, the shell, and Connect must not keep the previous room's
// private state after a switch or a sign-out.
const fx = fixtures();

async function sideways(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
}

test("a confirmation link opens on Needs you, and another workspace does not keep this queue", async ({ browser, page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.desk_channel_id, "#/needs");
  const title = page.locator(".need-title").first();
  await expect(title).toBeVisible();
  const marker = ((await title.textContent()) ?? "").trim();
  expect(marker.length).toBeGreaterThan(0);

  await page.evaluate(() => {
    window.location.hash = "#/confirm-approval=gate-1.one-time-token";
  });
  await expect(page.getByRole("region", { name: "Confirm an approval" })).toBeVisible();
  await expect(page).toHaveURL(/#\/needs$/);
  await expect(page).not.toHaveURL(/one-time-token/);

  const away = await browser.newContext();
  const other = await away.newPage();
  await other.goto("/ui/#/needs");
  await other.getByLabel(/paste a token/i).fill(fx.other_token);
  await other.getByRole("button", { name: "Sign in with token" }).click();
  await expect(other.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await other.goto("/ui/#/needs");
  await expect(other.locator(".need-title", { hasText: marker })).toHaveCount(0);
  await away.close();
});

test("a minted client snippet can take focus, and signing out drops the admin secret", async ({ browser }) => {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await page.route("**/ui/api/workspaces/*/agents", (route) =>
    route.fulfill({
      status: 201,
      contentType: "application/json",
      body: JSON.stringify({
        member: { id: "00000000-0000-7000-8000-0000000000c1", workspace_id: fx.workspace_id, handle: "iso-bot", display_name: "Iso Bot", kind: "agent" },
        token: { id: "00000000-0000-7000-8000-0000000000c2", secret: "mdn_iso_worker", workspace_id: fx.workspace_id, member_id: "00000000-0000-7000-8000-0000000000c1", capabilities: ["workspace:read"], quotas: [] },
      }),
    }),
  );
  await page.route("**/auth/session/mint", (route) => route.fulfill({ status: 200, contentType: "application/json", body: JSON.stringify({ secret: "mdn_iso_admin" }) }));
  await page.goto("/ui/#/tools/connect");
  await page.getByLabel(/paste a token/i).fill(fx.admin_token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await page.getByLabel("Agent name").fill("Iso Bot");
  await page.getByRole("button", { name: "Add agent" }).click();
  const snippet = page.locator("pre.snippet-body").first();
  await expect(snippet).toBeVisible();
  await expect(snippet).toHaveAttribute("tabindex", "0");

  await page.getByRole("button", { name: "Mint an admin token" }).click();
  const admin = page.getByRole("region", { name: "Admin token" });
  await expect(admin).toBeVisible();
  await page.goto("/ui/#/tools/prefs");
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByLabel(/paste a token/i)).toBeVisible();
  await expect(page.getByRole("region", { name: "Admin token" })).toHaveCount(0);
  await ctx.close();
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("More closes on Escape and on a tap outside, without sideways scroll", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, "#/board");
    const more = page.locator("details.tab-more");
    const summary = more.locator("summary");
    await summary.click();
    await expect(more).toHaveJSProperty("open", true);
    await page.keyboard.press("Escape");
    await expect(more).toHaveJSProperty("open", false);
    await expect(summary).toBeFocused();

    await summary.click();
    await expect(more).toHaveJSProperty("open", true);
    await page.locator("body").click({ position: { x: 8, y: 8 } });
    await expect(more).toHaveJSProperty("open", false);

    await summary.click();
    await more.locator(".tab-more-list a").first().click();
    await expect(more).toHaveJSProperty("open", false);

    expect(await sideways(page)).toBeLessThanOrEqual(1);
    const small = await page.locator(".tabbar a:visible, .tabbar summary:visible, .top .icon-btn:visible").evaluateAll((els) =>
      els
        .map((e) => ({ t: (e.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 30), r: e.getBoundingClientRect() }))
        .filter(({ r }) => r.width > 0 && r.height > 0 && (Math.round(r.height) < 44 || Math.round(r.width) < 44))
        .map(({ t, r }) => `${t} ${Math.round(r.width)}x${Math.round(r.height)}`),
    );
    expect(small).toEqual([]);
  });
});

test("the signed-in console has no axe violations", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.desk_channel_id, "#/needs");
  await expect(page.locator(".need, .needs-empty").first()).toBeVisible();
  await expect(page.locator("main section").first()).toHaveCSS("opacity", "1");
  const scan = await new AxeBuilder({ page }).analyze();
  expect(scan.violations.map((v) => `${v.id}: ${v.help}`)).toEqual([]);
});
