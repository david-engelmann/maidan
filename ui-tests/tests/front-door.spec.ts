import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

// The front door (docs/Hosted Console.md, open question 4). Signing in with
// the identity provider needs no workspace id: the server signs the identity
// in to its most recent workspace and leaves a hint on the return path. With
// more than one workspace this page opens the list as a chooser; with none it
// says so, since no session was made. The workspace-id sign-in is unchanged.
//
// The harness has no identity provider, so discovery is answered as a server
// with one, and the provider round trip itself is covered server-side
// (oidc_front_door_e2e.rs). Here the page's half: where the button goes, and
// what the page does with each hint.
const fx = fixtures();

async function offerIdentityProvider(page: Page) {
  await page.route("**/.well-known/maidan.json", async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    body.auth = { ...(body.auth || {}), oidc: true, oidc_login: "/auth/oidc/login", sessions: true };
    await route.fulfill({ response: res, json: body });
  });
}

async function catchLogins(page: Page): Promise<string[]> {
  const logins: string[] = [];
  await page.route("**/auth/oidc/login**", async (route) => {
    logins.push(route.request().url());
    await route.fulfill({ status: 200, contentType: "text/plain", body: "signing in" });
  });
  return logins;
}

async function signIn(page: Page) {
  await page.context().addCookies([
    { name: "maidan_session", value: fx.session_cookie, url: fx.base_url, httpOnly: true, sameSite: "Lax" },
  ]);
}

test("signing in with no workspace id goes to the front door", async ({ page }) => {
  await offerIdentityProvider(page);
  const logins = await catchLogins(page);
  await page.goto("/ui/");
  await expect(page.locator("#login")).toBeVisible();
  await expect(page.locator("#workspace")).toHaveValue("");
  const nav = page.waitForURL(/\/auth\/oidc\/login/);
  await page.locator("#login").click();
  await nav;
  expect(logins).toHaveLength(1);
  const url = new URL(logins[0]);
  expect(url.searchParams.has("workspace_id")).toBe(false);
  expect(url.searchParams.get("return_to")).toBe("/ui/");
});

test("a workspace id still signs in to that workspace", async ({ page }) => {
  await offerIdentityProvider(page);
  const logins = await catchLogins(page);
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  const nav = page.waitForURL(/\/auth\/oidc\/login/);
  await page.locator("#login").click();
  await nav;
  const url = new URL(logins[0]);
  expect(url.searchParams.get("workspace_id")).toBe(fx.workspace_id);
  expect(url.searchParams.get("return_to")).toBe("/ui/");
});

test("landing with several workspaces opens the chooser, and continuing stays put", async ({ page }) => {
  await signIn(page);
  const logins = await catchLogins(page);
  await page.goto("/ui/?choose_workspace=1");
  const chooser = page.locator("#ws-switcher");
  await expect(chooser).toBeVisible();
  await expect(chooser).toHaveAttribute("aria-label", "Choose a workspace");
  await expect(page.locator("#ws-switch-note")).toContainText("your most recent workspace");
  // Only this identity's two workspaces, never another identity's.
  await expect(page.locator("#ws-switch-list li")).toHaveCount(2);
  await expect(page.locator("#ws-switch-list")).not.toContainText("Someone else's desk");
  // The hint is read once and taken off the address bar.
  expect(new URL(page.url()).search).toBe("");

  const here = page.locator(`#ws-switch-list li[data-id="${fx.workspace_id}"] button`);
  await expect(here).toBeEnabled();
  await expect(here).toContainText("continue here");
  await here.click();
  await expect(chooser).toBeHidden();
  await expect(page.locator("#ws-switch")).toBeFocused();
  expect(logins).toHaveLength(0);

  // Opened later from the header, it is the ordinary switcher again.
  await page.locator("#ws-switch").click();
  await expect(chooser).toHaveAttribute("aria-label", "Switch workspace");
  await expect(page.locator("#ws-switch-note")).toBeHidden();
  await expect(here).toBeDisabled();
});

test("choosing another workspace signs in there", async ({ page }) => {
  await signIn(page);
  const logins = await catchLogins(page);
  await page.goto("/ui/?choose_workspace=1");
  await expect(page.locator("#ws-switcher")).toBeVisible();
  const nav = page.waitForURL(/\/auth\/oidc\/login/);
  await page.locator(`#ws-switch-list li[data-id="${fx.switch_workspace_id}"] button`).click();
  await nav;
  expect(new URL(logins[0]).searchParams.get("workspace_id")).toBe(fx.switch_workspace_id);
});

test("without a session, no chooser opens even if the hint says so", async ({ page }) => {
  await page.goto("/ui/?choose_workspace=1");
  await expect(page.locator("#first-run")).toBeVisible();
  await expect(page.locator("#ws-switcher")).toBeHidden();
  expect(new URL(page.url()).search).toBe("");
});

test("landing with no workspace says so and opens nothing", async ({ page }) => {
  await offerIdentityProvider(page);
  await page.goto("/ui/?no_workspace=1");
  const notice = page.locator("#no-workspace");
  await expect(notice).toBeVisible();
  await expect(notice).toHaveAttribute("role", "alert");
  await expect(notice).toContainText("isn't a member of any workspace on this server");
  await expect(page.locator("#first-run")).toBeVisible();
  await expect(page.locator("#login")).toBeVisible();
  expect(new URL(page.url()).search).toBe("");

  // A reload does not replay it.
  await page.reload();
  await expect(page.locator("#first-run")).toBeVisible();
  await expect(notice).toBeHidden();
});

test("a stale no-workspace hint is ignored when a session exists", async ({ page }) => {
  await signIn(page);
  await page.goto("/ui/?no_workspace=1");
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#no-workspace")).toBeHidden();
});
