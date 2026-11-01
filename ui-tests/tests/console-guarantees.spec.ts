import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// The promises the classic specs held that must hold in a real browser, not
// only in unit tests: hostile names stay text, a pasted token is never kept or
// echoed, a token cannot reach another workspace, and the board works from the
// keyboard with visible focus and 44px touch targets.

test("a hostile attachment filename renders as text", async ({ page }) => {
  const name = `<img src=x onerror="window.__pwned=1">.txt`;
  const params = new URLSearchParams({ kind: "attachment", mime_type: "text/plain", filename: name });
  const up = await page.request.post(`/artifacts?${params}`, {
    headers: { Authorization: `Bearer ${fx.token}` },
    data: "hello",
  });
  expect(up.status()).toBe(201);
  const sha = (await up.json()).sha256 as string;

  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/artifacts");
  await page.getByLabel("sha256").fill(sha);
  await page.getByRole("button", { name: "Look up" }).click();
  await expect(page.locator("dd", { hasText: "onerror" })).toHaveText(name);
  await expect(page.locator('img[src="x"]')).toHaveCount(0);
  expect(await page.evaluate(() => (window as unknown as { __pwned?: number }).__pwned)).toBeUndefined();
});

test("a pasted token is never stored, put in a URL, or echoed on the page", async ({ page }) => {
  const urls: string[] = [];
  page.on("request", (r) => urls.push(r.url()));
  await page.addInitScript((id) => window.localStorage.setItem("maidan_console_channel", id), fx.board_channel_id);
  await page.goto("/ui/#/board");
  await page.getByLabel(/paste a token/i).fill(fx.token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".lane").first()).toBeVisible();

  const kept = await page.evaluate(() => [
    ...Object.keys(localStorage).map((k) => localStorage.getItem(k) ?? ""),
    ...Object.keys(sessionStorage).map((k) => sessionStorage.getItem(k) ?? ""),
    document.cookie,
  ].join("\n"));
  // Never print the secret: compare as booleans only.
  expect(kept.includes(fx.token), "token found in storage or cookies").toBe(false);
  expect((await page.content()).includes(fx.token), "token echoed in the page").toBe(false);
  expect(urls.some((u) => u.includes(fx.token)), "token carried in a URL").toBe(false);
});

test("a token for another workspace does not open this workspace's board", async ({ page }) => {
  await page.addInitScript((id) => window.localStorage.setItem("maidan_console_channel", id), fx.board_channel_id);
  await page.goto("/ui/#/board");
  await page.getByLabel(/paste a token/i).fill(fx.second_token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeHidden();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
  await page.waitForTimeout(1500);
  await expect(page.locator("button.task", { hasText: "Open: nobody holds this" })).toHaveCount(0);
});

test("the board opens a task from the keyboard with a visible focus ring", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  const task = page.locator("button.task", { hasText: "Open: nobody holds this" });
  await task.focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(task).toBeFocused();
  await expect(task).not.toHaveCSS("outline-style", "none");
  await page.keyboard.press("Enter");
  const sheet = page.getByRole("dialog");
  await expect(sheet).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(sheet).toBeHidden();
  await expect(task).toBeFocused();
});

test("the board and Needs you have no serious axe violations", async ({ page }) => {
  await signInWithCookie(page, fx);
  for (const hash of ["#/board", "#/needs"]) {
    await openOnChannel(page, fx.board_channel_id, hash);
    await page.waitForTimeout(1500);
    const r = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    const bad = r.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
    expect(bad.map((v) => `${hash} ${v.id}: ${v.nodes.length}`)).toEqual([]);
  }
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("tab bar and task targets are at least 44px", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.board_channel_id);
    await expect(page.locator("button.task").first()).toBeVisible();
    const small = await page.locator(".tabbar a:visible, .tabbar button:visible, button.task:visible").evaluateAll((els) =>
      els
        .map((e) => ({ t: (e.textContent ?? "").trim().slice(0, 30), r: e.getBoundingClientRect() }))
        .filter(({ r }) => r.height < 44 || r.width < 44)
        .map(({ t, r }) => `${t} ${Math.round(r.width)}x${Math.round(r.height)}`),
    );
    expect(small).toEqual([]);
  });
});
