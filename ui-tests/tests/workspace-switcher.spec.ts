import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

// The workspace switcher (Open Work Next 6). The seeded operator signed in
// through the identity provider and is a member of a second workspace with
// the same identity; a third workspace belongs to another identity. The
// switcher lists only the first two, filters by name, and switches by signing
// in again, never by minting a session in the page.
const fx = fixtures();

async function signIn(page: Page) {
  await page.context().addCookies([
    { name: "maidan_session", value: fx.session_cookie, url: fx.base_url, httpOnly: true, sameSite: "Lax" },
  ]);
}

test("a person in two workspaces can find the other one and switch by signing in there", async ({ page }) => {
  await signIn(page);
  const logins: string[] = [];
  await page.route("**/auth/oidc/login**", async (route) => {
    logins.push(route.request().url());
    await route.fulfill({ status: 200, contentType: "text/plain", body: "signing in" });
  });
  const listed = page.waitForResponse((r) => r.url().endsWith("/auth/session/workspaces"));
  await page.goto("/ui/");
  const body = await (await listed).json();
  const ids = body.workspaces.map((w: { workspace_id: string }) => w.workspace_id);
  expect(ids).toEqual([fx.workspace_id, fx.switch_workspace_id]);
  expect(ids).not.toContain(fx.foreign_workspace_id);

  const open = page.locator("#ws-switch");
  await expect(open).toBeVisible();
  await open.click();
  await expect(open).toHaveAttribute("aria-expanded", "true");
  const items = page.locator("#ws-switch-list li");
  await expect(items).toHaveCount(2);
  await expect(page.locator("#ws-switch-list")).toContainText("Second desk");
  await expect(page.locator("#ws-switch-list")).not.toContainText("Someone else's desk");
  await expect(page.locator(`#ws-switch-list li[data-id="${fx.workspace_id}"] button`)).toBeDisabled();
  await expect(page.locator("#ws-switch-search")).toBeFocused();

  await page.fill("#ws-switch-search", "nothing like it");
  await expect(page.locator("#ws-switch-list")).toHaveText("No workspace matches.");
  await page.fill("#ws-switch-search", "SECOND");
  await expect(items).toHaveCount(1);

  const nav = page.waitForURL(/\/auth\/oidc\/login/);
  await page.locator("#ws-switch-search").press("Enter");
  await nav;
  expect(logins).toHaveLength(1);
  const url = new URL(logins[0]);
  expect(url.searchParams.get("workspace_id")).toBe(fx.switch_workspace_id);
  expect(url.searchParams.get("return_to")).toBe("/ui/");
});

test("Escape closes the switcher and gives focus back", async ({ page }) => {
  await signIn(page);
  await page.goto("/ui/");
  await page.locator("#ws-switch").click();
  await expect(page.locator("#ws-switcher")).toBeVisible();
  await page.locator("#ws-switch-search").press("Escape");
  await expect(page.locator("#ws-switcher")).toBeHidden();
  await expect(page.locator("#ws-switch")).toBeFocused();
});

test("a session made from a pasted token offers no switcher", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  const exchanged = page.waitForResponse((r) => r.url().endsWith("/auth/session/from-token"));
  await page.locator("#token").press("Enter");
  expect((await exchanged).status()).toBe(201);
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#ws-switch")).toBeHidden();
});
