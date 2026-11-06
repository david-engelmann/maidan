import { test, expect, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";

const fx = fixtures();

function watchCsp(page: Page): string[] {
  const notes: string[] = [];
  page.on("console", (msg) => {
    if (/Content Security Policy/i.test(msg.text())) notes.push(msg.text());
  });
  return notes;
}

async function fits(page: Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);
}

async function theme(page: Page): Promise<string[]> {
  return page.locator('meta[name="theme-color"]').evaluateAll((els) =>
    els.map((el) => `${el.getAttribute("media")} ${el.getAttribute("content")}`),
  );
}

async function scan(page: Page, scheme: "light" | "dark"): Promise<void> {
  await page.emulateMedia({ colorScheme: scheme });
  const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa", "best-practice"]).analyze();
  expect(result.violations.map((v) => `${scheme} ${v.id}`)).toEqual([]);
  expect(await fits(page)).toBe(true);
}

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("the route index is a landmark in both color schemes", async ({ page }) => {
    const csp = watchCsp(page);
    const response = await page.goto("/");
    expect(response?.headers()["content-security-policy"]).toBe(
      "default-src 'none'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
    );
    await expect(page.locator("main h1")).toHaveText("maidan");
    await expect(page.locator("main a[href='/ui']")).toBeVisible();
    expect(await page.locator("style, script").count()).toBe(0);
    expect(await page.locator('meta[name="theme-color"]').evaluateAll((els) => els.map((el) => el.getAttribute("content")))).toEqual(["#f2eee6"]);
    await scan(page, "light");
    await scan(page, "dark");
    expect(csp).toEqual([]);
  });

  test("operator status keeps its facts and its pinned style", async ({ page }) => {
    const csp = watchCsp(page);
    await page.setExtraHTTPHeaders({
      Authorization: `Bearer ${fx.second_token}`,
      Accept: "text/html",
    });
    const response = await page.goto("/operator/status");
    expect(response?.ok()).toBeTruthy();
    const policy = response?.headers()["content-security-policy"] ?? "";
    expect(policy.startsWith("default-src 'none'; style-src 'sha256-")).toBeTruthy();
    expect(policy).not.toContain("unsafe");
    expect(response?.headers()["referrer-policy"]).toBe("no-referrer");
    expect(response?.headers()["x-content-type-options"]).toBe("nosniff");
    await expect(page.locator("main h1")).toHaveText("Maidan status");
    await expect(page.locator("main")).toContainText("Search backfill");
    await expect(page.locator("th", { hasText: "Phase" })).toBeVisible();
    expect(await theme(page)).toEqual([
      "(prefers-color-scheme: light) #f2eee6",
      "(prefers-color-scheme: dark) #121110",
    ]);
    await scan(page, "light");
    await scan(page, "dark");
    expect(csp, csp.join("\n")).toEqual([]);
  });
});
