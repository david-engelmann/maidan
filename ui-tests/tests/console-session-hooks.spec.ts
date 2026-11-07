import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";

// A token for one workspace must not keep another workspace's channel,
// board, or roster, and the first-run snippets have to be keyboard-scrollable.
const fx = fixtures();

async function sideways(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
}

async function signIn(page: Page, secret: string, hash = "#/board"): Promise<void> {
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(secret);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
}

async function signOut(page: Page): Promise<void> {
  await page.goto("/ui/#/tools/prefs");
  await page.getByRole("button", { name: "Sign out" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();
}

test("a bad token's error clears as the token is edited", async ({ page }) => {
  await page.goto("/ui/#/board");
  await page.locator("#sign-in-token").fill("not-a-real-token");
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("alert")).toContainText(/not accepted/i);
  await page.locator("#sign-in-token").fill("not-a-real-token-edited");
  await expect(page.getByRole("alert")).toHaveCount(0);
});

test("each workspace remembers its channel, and the other room's board and people stay there", async ({ browser }) => {
  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await signIn(page, fx.admin_token);
  await page.getByLabel("Channel").selectOption({ label: "quiet" });
  await expect(page.locator(".channel-name, .channel-pick option:checked").first()).toContainText("quiet");
  const stored = await page.evaluate((id) => ({
    scoped: localStorage.getItem(`maidan_console_channel:${id}`),
    legacy: localStorage.getItem("maidan_console_channel"),
  }), fx.workspace_id);
  expect(stored.scoped).toBe(fx.quiet_channel_id);
  expect(stored.legacy).toBe(fx.quiet_channel_id);
  await expect(page.getByRole("heading", { name: "Connect your first agent" })).toBeVisible();

  await signOut(page);
  await signIn(page, fx.second_token);
  await expect(page.getByText("Afar: the second workspace's blocked task")).toBeVisible();
  await expect(page.getByText("Deploy v9")).toHaveCount(0);
  await page.goto("/ui/#/people");
  await expect(page.getByRole("link", { name: /Stranger/ })).toBeVisible();
  await expect(page.getByRole("link", { name: /Operator/ })).toHaveCount(0);

  await signOut(page);
  await signIn(page, fx.admin_token, "#/board");
  await expect(page.locator(".channel-name, .channel-pick option:checked").first()).toContainText("quiet");
  await expect(page.getByRole("heading", { name: "Connect your first agent" })).toBeVisible();
  await ctx.close();
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("first-run snippets scroll by keyboard, and another token does not keep this room", async ({ browser }) => {
    // A route change fades the page in, and new board cards fade in after it.
    // An axe pass that lands mid-fade reads text at part opacity and reports
    // color-contrast; with reduced motion nothing fades, so the scan sees what
    // a reader sees. The failure names its nodes, should a real one appear.
    // (A context made here does not inherit test.use, so it is set here.)
    const ctx = await browser.newContext({ reducedMotion: "reduce" });
    const page = await ctx.newPage();
    await signIn(page, fx.admin_token);
    await page.getByLabel("Channel").selectOption({ label: "quiet" });
    const snippets = page.locator(".first-run pre.snippet-body");
    await expect(snippets).toHaveCount(2);
    await expect(snippets.nth(0)).toHaveAttribute("tabindex", "0");
    await expect(snippets.nth(1)).toHaveAttribute("tabindex", "0");
    await expect(page.locator("main section").first()).toHaveCSS("opacity", "1");
    const scan = await new AxeBuilder({ page }).analyze();
    expect(scan.violations.map((v) => `${v.id}: ${v.help} ${v.nodes.map((n) => n.target.join(" ")).join(", ")}`)).toEqual([]);
    expect(await sideways(page)).toBeLessThanOrEqual(1);

    await signOut(page);
    await signIn(page, fx.second_token);
    await expect(page.getByText("Afar: the second workspace's blocked task")).toBeVisible();
    await expect(page.getByRole("heading", { name: "Connect your first agent" })).toHaveCount(0);
    await ctx.close();
  });
});
