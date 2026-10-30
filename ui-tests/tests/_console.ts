import { expect, type Page } from "@playwright/test";
import type { Fixtures } from "./_fixtures";

// The console keeps the board's channel here; setting it before load opens
// that channel the way a returning person would see it.
const CHANNEL_KEY = "maidan_console_channel";

/** Sign in as the operator through the session cookie, a person's sign-in. */
export async function signInWithCookie(page: Page, fx: Fixtures): Promise<void> {
  await page.context().addCookies([
    { name: "maidan_session", value: fx.session_cookie, domain: new URL(fx.base_url).hostname, path: "/" },
  ]);
}

/** Open the console with a channel already picked for the board. */
export async function openOnChannel(page: Page, channelId: string, hash = "#/board"): Promise<void> {
  await page.addInitScript(([key, id]) => window.localStorage.setItem(key, id), [CHANNEL_KEY, channelId]);
  await page.goto(`/ui/${hash}`);
  await expect(page.locator(".rail")).toBeVisible();
}
