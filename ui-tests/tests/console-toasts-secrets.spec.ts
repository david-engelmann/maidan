import { test, expect, type Locator, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Toasts stay while the pointer or focus is on them, and a secret card
// shown once starts over when a second secret replaces it.
const fx = fixtures();

async function pasteIn(page: Page, secret: string, hash: string): Promise<void> {
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(secret);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.locator(".rail:visible, .tabbar:visible").first()).toBeVisible();
}

async function pinToast(page: Page): Promise<Locator> {
  if (!page.url().includes(`/threads/t/${fx.board_open_thread_id}`)) {
    await page.goto(`/ui/#/threads/t/${fx.board_open_thread_id}`);
  }
  const body = `pin probe ${Date.now()}`;
  await page.locator("#compose").fill(body);
  await page.getByRole("button", { name: "Send" }).click();
  const msg = page.locator(".msg", { hasText: body });
  await expect(msg).toBeVisible();
  await msg.hover();
  await msg.getByRole("button", { name: "Add a reaction" }).click();
  await msg.getByRole("button", { name: "Pin" }).click();
  const toast = page.locator(".toasts .toast").last();
  await expect(toast).toBeVisible();
  return toast;
}

test("a toast pauses under the pointer and focus, and Escape or a new page clears it", async ({ page }) => {
  test.setTimeout(60_000);
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, `#/threads/t/${fx.board_open_thread_id}`);
  const toast = await pinToast(page);
  await expect(toast).toHaveAttribute("role", "status");
  await toast.hover();
  await page.waitForTimeout(5500);
  await expect(toast).toBeVisible();
  await page.mouse.move(2, 2);
  await page.waitForTimeout(1000);
  await expect(toast).toBeVisible();
  await toast.getByRole("button", { name: "Dismiss" }).focus();
  await page.waitForTimeout(5500);
  await expect(toast).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(toast).toHaveCount(0);

  const again = await pinToast(page);
  await page.evaluate(() => {
    window.location.hash = "#/board";
  });
  await expect(again).toHaveCount(0);

  await page.route("**/threads/*/pins", (route) => (route.request().method() === "POST" ? route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "pin failed" }) }) : route.fallback()));
  const failed = await pinToast(page);
  await expect(failed).toHaveClass(/error/);
  await expect(failed).toHaveAttribute("role", "alert");
});

test("a second secret resets Show and Copied, and the card stays in the phone width", async ({ page }) => {
  test.setTimeout(45_000);
  await page.addInitScript(() => {
    const clip = {
      writeText: (value: string) => {
        (globalThis as unknown as { __copied?: string }).__copied = value;
        return Promise.resolve();
      },
      readText: () => Promise.resolve(""),
    };
    Object.defineProperty(Navigator.prototype, "clipboard", { configurable: true, get: () => clip });
  });
  await pasteIn(page, fx.admin_token, `#/people/m/${fx.member_id}`);
  const mint = async (label: string) => {
    await page.getByRole("checkbox", { name: "Agent worker preset" }).uncheck();
    await page.getByLabel("Label").fill(label);
    await page.getByRole("textbox", { name: "Capabilities" }).fill("workspace:read");
    await page.getByRole("button", { name: "Mint" }).click();
  };
  await mint(`once-a-${Date.now()}`);
  const once = page.locator(".secret-once");
  await expect(once).toBeVisible();
  await expect(once).toHaveAttribute("role", "region");
  const copy = once.locator("button.steel");
  await copy.click();
  await expect(copy).toHaveText("Copied");
  await page.waitForTimeout(2600);
  await expect(copy).toHaveText("Copy");
  await once.getByRole("button", { name: "Show" }).click();
  await copy.click();
  await expect(copy).toHaveText("Copied");
  await expect(once.getByRole("button", { name: "Hide" })).toBeVisible();
  await mint(`once-b-${Date.now()}`);
  await expect(once.getByRole("button", { name: "Show" })).toBeVisible();
  await expect(copy).toHaveText("Copy");
  await expect(once.getByLabel("The secret, hidden")).toBeVisible();

  const axe = await new AxeBuilder({ page }).include(".secret-once").analyze();
  expect(axe.violations.map((v) => v.id)).toEqual([]);

  await page.setViewportSize({ width: 390, height: 844 });
  const extra = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(extra).toBeLessThanOrEqual(0);
  const small = await once.locator("button").evaluateAll((els) =>
    els
      .map((e) => {
        const r = e.getBoundingClientRect();
        return { name: (e.textContent ?? "").trim(), w: r.width, h: r.height };
      })
      .filter(({ w, h }) => w < 44 || h < 44)
      .map(({ name, w, h }) => `${name} ${Math.round(w)}x${Math.round(h)}`),
  );
  expect(small).toEqual([]);
});
