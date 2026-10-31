import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Sign-in as a person sees it: the session card names the credential and the
// real grant, the badge says which credential the page acts with, the front
// door's hints open the chooser or say there is no workspace, and the
// switcher lists only this identity's workspaces.
const fx = fixtures();

async function signInWithToken(page: Page, secret: string, hash = "#/tools/prefs"): Promise<void> {
  await page.goto(`/ui/${hash}`);
  await page.getByLabel(/paste a token/i).fill(secret);
  await page.getByRole("button", { name: "Sign in with token" }).click();
}

async function mint(request: APIRequestContext, label: string, capabilities: string[]): Promise<{ id: string; secret: string }> {
  const res = await request.post(`/workspaces/${fx.workspace_id}/members/${fx.member_id}/tokens`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { label, capabilities },
  });
  expect(res.ok(), `mint: ${res.status()}`).toBeTruthy();
  return res.json();
}

test("a token's session card shows its real grant, and the badge says bearer", async ({ page, request }) => {
  const t = await mint(request, "signin-card", ["workspace:read", "thread:transition"]);
  await signInWithToken(page, t.secret);
  await expect(page.locator(".mode-chip:visible").first()).toHaveAttribute("data-mode", "bearer");
  await expect(page.locator(".mode-chip:visible").first()).toHaveText("Bearer token");
  await expect(page.locator(".session-credential")).toHaveText("Bearer token: acts as this member");
  await expect(page.locator(".session-member")).toHaveText(fx.member_id);
  const can = page.locator("ul.cap-can li");
  await expect(can).toHaveText(["workspace:read", "thread:transition"].sort().map((c) => new RegExp(c)), { useInnerText: true }).catch(async () => {
    // The card may order its rows; the set is what matters.
    expect((await can.allInnerTexts()).map((s) => s.trim()).sort()).toEqual(["thread:transition", "workspace:read"]);
  });
  await expect(page.locator("ul.cap-cant li", { hasText: "token:admin" })).toHaveCount(1);
  await request.delete(`/tokens/${t.id}`, { headers: { Authorization: `Bearer ${fx.admin_token}` } });
});

test("a person's sign-in says session on the badge and the card", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/tools/prefs");
  await expect(page.locator(".mode-chip:visible").first()).toHaveAttribute("data-mode", "session");
  await expect(page.locator(".mode-chip:visible").first()).toHaveText("Session");
  await expect(page.locator(".session-credential")).toHaveText("Session: your own sign-in, acting as you");
  await expect(page.locator(".session-member")).toHaveText(fx.member_id);
});

// Its own session, so logging out never ends the shared fixture cookie.
test("Sign out ends the page's session, and a reload stays signed out", async ({ page, request }) => {
  const t = await mint(request, "signin-out", ["workspace:read"]);
  await signInWithToken(page, t.secret);
  await expect(page.locator(".session-signout")).toBeVisible();
  const logout = page.waitForResponse((r) => r.url().endsWith("/auth/logout") && r.request().method() === "POST");
  await page.locator(".session-signout").click();
  const out = await logout;
  expect(out.status(), "logout answers with a redirect or a 2xx").toBeLessThan(400);
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();
  await page.reload();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" }), "the cookie was logged out too").toBeVisible();
  await request.delete(`/tokens/${t.id}`, { headers: { Authorization: `Bearer ${fx.admin_token}` } });
});

test("the front door's choose hint opens the chooser with only this identity's workspaces", async ({ page }) => {
  await signInWithCookie(page, fx);
  await page.goto("/ui/?choose_workspace=1#/board");
  const chooser = page.getByRole("dialog", { name: "Choose a workspace" });
  await expect(chooser).toBeVisible();
  await expect(page, "the hint is taken off the address bar").not.toHaveURL(/choose_workspace/);
  const ids = await chooser.locator("li[data-id]").evaluateAll((els) => els.map((e) => e.getAttribute("data-id")));
  expect(ids.sort()).toEqual([fx.workspace_id, fx.switch_workspace_id].sort());
  expect(ids).not.toContain(fx.foreign_workspace_id);
  const sw = chooser.locator(`li[data-id="${fx.switch_workspace_id}"] a`, { hasText: "Switch" });
  await expect(sw).toHaveAttribute("href", new RegExp(`workspace_id=${fx.switch_workspace_id}.*return_to=/ui/`));
  await page.keyboard.press("Escape");
  await expect(chooser).toBeHidden();
  await page.reload();
  await expect(chooser, "a reload does not replay the hint").toBeHidden();
});

test("the front door's no-workspace hint says so on the sign-in card", async ({ page }) => {
  await page.goto("/ui/?no_workspace=1#/board");
  await expect(page.locator(".no-workspace")).toContainText("isn't a member of any workspace");
  await expect(page).not.toHaveURL(/no_workspace/);
});

test("the workspace switcher lists this identity's workspaces and marks the current one", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/tools/workspace");
  const rows = page.locator(".tool-row", { has: page.locator(".state-chip, .chip-btn", { hasText: /Current|Switch/ }) });
  await expect(rows).toHaveCount(2);
  await expect(rows.locator(".state-chip", { hasText: "Current" })).toHaveCount(1);
  const href = await rows.locator("a.chip-btn", { hasText: "Switch" }).getAttribute("href");
  expect(href).toContain(`workspace_id=${fx.switch_workspace_id}`);
  expect(href).not.toContain(fx.foreign_workspace_id);
});
