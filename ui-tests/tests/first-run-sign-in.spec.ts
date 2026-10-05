import { test, expect, Request } from "@playwright/test";
import { fixtures } from "./_fixtures";

// Signing in is something a person can see and do: Enter in the token field
// or a Sign in button. Until a credential exists the page asks the server for
// nothing it would refuse, so nobody is told their token was "not accepted"
// before they have pasted one.
const fx = fixtures();

test("pasting a token and pressing Enter signs in", async ({ page }) => {
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toBeHidden();
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  const exchanged = page.waitForResponse((r) => r.url().endsWith("/auth/session/from-token"));
  await page.locator("#token").press("Enter");
  expect((await exchanged).status()).toBe(201);
  await expect(page.locator("#first-run")).toBeHidden();
  await expect(page.locator("#identity-who")).toContainText("Operator");
  await expect(page.locator(`#channel-list li[data-id="${fx.desk_channel_id}"]`)).toBeVisible();
});

test("the Sign in button signs in, and spends the token once", async ({ page }) => {
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toBeHidden();
  const signIn = page.getByRole("button", { name: "Sign in", exact: true });
  await expect(signIn).toBeVisible();
  await expect(signIn).not.toHaveClass(/\bprimary\b/);

  // With nothing pasted the button says what to do and asks nothing.
  await signIn.click();
  await expect(page.locator("#token-hint")).toHaveText("Paste your token to connect.");
  await expect(page.locator("#token")).toBeFocused();

  const exchanges: Request[] = [];
  page.on("request", (r) => {
    if (r.url().endsWith("/auth/session/from-token")) exchanges.push(r);
  });
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  await signIn.click();
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#first-run")).toBeHidden();
  expect(exchanges).toHaveLength(1);
});

test("a workspace id before any token asks the server for nothing and blames no one", async ({ page }) => {
  const channelReads: string[] = [];
  page.on("request", (r) => {
    if (/\/channels(\?|$)/.test(r.url())) channelReads.push(r.url());
  });
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toBeHidden();
  await page.fill("#workspace", fx.workspace_id);
  // Leaving the field fires its change, which is what used to load channels.
  await page.locator("#workspace").press("Tab");

  await expect(page.locator("#channel-list")).toHaveText("Paste your token to connect.");
  await page.click("#refresh-channels");
  await expect(page.locator("#channel-list")).toHaveText("Paste your token to connect.");
  expect(channelReads).toEqual([]);
  await expect(page.locator("body")).not.toContainText("not accepted");
  await expect(page.locator("#toasts .toast")).toHaveCount(0);
  await expect(page.locator("#first-run")).toBeVisible();
});
