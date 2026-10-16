import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Hold the token exchange until the test lets it go, as a slow link would.
async function holdExchange(page: Page): Promise<() => void> {
  let release: () => void = () => {};
  const held = new Promise<void>((resolve) => (release = resolve));
  await page.route("**/auth/session/from-token", async (route) => {
    await held;
    await route.continue();
  });
  return release;
}

async function pasteToken(page: Page, token: string) {
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toBeHidden();
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", token);
  await page.locator("#token").dispatchEvent("change");
}

// The audit timed a slow-3G sign-in at 21 s with nothing on screen but a
// disabled field. While it runs, the button and the status line say so.
test("a sign-in in progress says it is signing in, then clears", async ({ page }) => {
  const release = await holdExchange(page);
  await pasteToken(page, fx.token);

  const button = page.locator("#token-signin");
  await expect(button).toHaveText("Signing in…");
  await expect(button).toHaveAttribute("aria-busy", "true");
  await expect(page.locator("#session-status")).toHaveText("Signing in…");

  release();
  await expect(page.locator("#logout")).toBeVisible();
  await expect(button).toHaveText("Sign in");
  await expect(button).not.toHaveAttribute("aria-busy", /.*/);
  await expect(page.locator("#session-status")).not.toHaveText("Signing in…");
});

// A refused token replaces "Signing in…" with the reason, not a blank line.
test("a refused token replaces the progress line with the reason", async ({ page }) => {
  const release = await holdExchange(page);
  await pasteToken(page, "mdn_not_a_real_token");
  await expect(page.locator("#session-status")).toHaveText("Signing in…");
  release();
  const status = page.locator("#session-status");
  await expect(status).toHaveClass(/err/);
  await expect(status).not.toHaveText("Signing in…");
  await expect(status).not.toHaveText("");
  await expect(page.locator("#token-signin")).toHaveText("Sign in");
});
