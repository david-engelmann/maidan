import { test, expect, type Page, type ConsoleMessage } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

function watchConsole(page: Page): string[] {
  const notes: string[] = [];
  page.on("console", (msg: ConsoleMessage) => {
    if (msg.type() === "warning" || msg.type() === "error") notes.push(`${msg.location().url} :: ${msg.text()}`);
  });
  return notes;
}

async function sideways(page: Page): Promise<number> {
  return page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
}

test("an existing token includes Codex, and a filtered board keeps its channel", async ({ page }) => {
  const notes = watchConsole(page);
  await signInWithCookie(page, fx);
  await page.goto("/ui/#/tools/connect");
  await expect(page.getByRole("heading", { name: "Connect an agent" })).toBeVisible();
  await page.getByRole("button", { name: "Already have a token?" }).click();
  const codex = page.locator(".snippet", { hasText: "Codex (~/.codex/config.toml, with a token)" });
  await expect(codex.locator("pre.snippet-body")).toContainText("tool_timeout_sec = 330");
  await expect(codex.locator("pre.snippet-body")).toContainText("bearer_token_env_var = \"MAIDAN_TOKEN\"");
  const bodies = page.locator("#connect-token-templates pre.snippet-body");
  await expect(bodies.first()).toContainText("mcpServers");
  await expect(bodies.nth(1)).toContainText("claude mcp add");
  await expect(page.locator("#main > section")).toHaveCSS("opacity", "1");
  const connectScan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(connectScan.violations.map((v) => v.id)).toEqual([]);

  await page.goto(`/ui/#/board?channel=${fx.board_channel_id}`);
  await expect(page.locator(".board, .lanes, button.task").first()).toBeVisible();
  await page.getByLabel("State").selectOption("open");
  await expect(page).toHaveURL(new RegExp(`channel=${fx.board_channel_id}`));
  await expect(page).toHaveURL(/state=open/);
  await page.getByPlaceholder("Search tasks").fill("hello");
  await expect(page).toHaveURL(new RegExp(`channel=${fx.board_channel_id}`));
  await expect(page).toHaveURL(/q=hello/);
  await expect(page.getByText("Nothing here matches.")).toBeVisible();
  await expect(page.locator(".task")).toHaveCount(0);
  await expect(page.locator("#main > section")).toHaveCSS("opacity", "1");
  const boardScan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(boardScan.violations.map((v) => v.id)).toEqual([]);
  // Chromium logs a resource error for the first-run HEAD /support probe when
  // that route is absent. That probe lives outside this change. The suite
  // blocks the service worker (playwright.config.ts), and Playwright says so
  // when the console registers it.
  const unexpected = notes.filter((n) => !/\/support :: Failed to load resource/.test(n) && !/Service Worker registration blocked by Playwright/.test(n));
  expect(unexpected.filter((n) => /apple-mobile-web-app-capable/i.test(n))).toEqual([]);
  expect(unexpected).toEqual([]);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("connect stays on the screen and its controls are at least 44px", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id, "#/tools/connect");
    await page.getByRole("button", { name: "Already have a token?" }).click();
    await expect(page.getByText("tool_timeout_sec = 330")).toBeVisible();
    expect(await sideways(page)).toBeLessThanOrEqual(1);
    const small = await page.locator(".tabbar a:visible, .tabbar summary:visible, .top .icon-btn:visible, #connect-token-templates .copy, .button:visible").evaluateAll((els) =>
      els
        .map((e) => {
          const r = e.getBoundingClientRect();
          return { t: (e.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 24), w: Math.round(r.width), h: Math.round(r.height) };
        })
        .filter(({ w, h }) => w > 0 && h > 0 && (w < 44 || h < 44))
        .map(({ t, w, h }) => `${t} ${w}x${h}`),
    );
    expect(small).toEqual([]);
  });
});
