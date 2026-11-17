import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

const fx = fixtures();

// The console at /ui against the seeded server: it loads under its CSP, the
// old /console path redirects, and a person can sign in and read the board.

test("the console loads with no CSP violations or page errors", async ({ page }) => {
  const problems: string[] = [];
  page.on("pageerror", (e) => problems.push(e.message));
  page.on("console", (m) => {
    if (m.type() === "error" && /Content Security Policy|Refused to/i.test(m.text())) problems.push(m.text());
  });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  await expect(page.locator(".lane").first()).toBeVisible();
  expect(problems).toEqual([]);
});

test("/console redirects to /ui/", async ({ request }) => {
  const res = await request.get("/console", { maxRedirects: 0 });
  expect(res.status()).toBe(308);
  expect(res.headers()["location"]).toBe("/ui/");
});

test("signed out, the console offers its own sign-in", async ({ page }) => {
  await page.goto("/ui/#/board");
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();
  await expect(page.getByLabel(/paste a token/i)).toBeVisible();
});

test("signed out, the header offers nothing that needs a sign-in", async ({ page }) => {
  await page.goto("/ui/#/board");
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();
  // Nothing to search and no live status before a sign-in: the card is the page.
  await expect(page.getByRole("button", { name: /search and commands/i })).toHaveCount(0);
  await expect(page.locator("header.top .live")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+k");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  // The page has one level-one heading, and it is not a second "Sign in".
  await expect(page.getByRole("heading", { level: 1 })).toHaveText(/Maidan/);
});

test("with open sign-up, creating a workspace and a token each have a section of their own", async ({ page }) => {
  await page.route("**/.well-known/maidan.json", async (route) => {
    const res = await route.fetch();
    const body = (await res.json()) as { auth?: Record<string, unknown> };
    const auth = { ...(body.auth ?? {}), oidc: true, oidc_login: "/auth/oidc/login", oidc_signup: "/auth/oidc/signup", oidc_signup_open: true, sessions: true };
    await route.fulfill({ response: res, json: { ...body, auth } });
  });
  await page.goto("/ui/#/board");
  const card = page.locator(".sign-in");
  await expect(card.getByRole("link", { name: "Sign in", exact: true })).toBeVisible();
  // Each path under its own divider, in this order: sign in, create, token.
  const order = await card.evaluate((el) =>
    [...el.querySelectorAll(".sign-in-divider, #workspace-name, #sign-in-token")].map((n) => n.textContent?.trim() || n.id),
  );
  expect(order).toEqual(["New to Maidan?", "workspace-name", "Have a token?", "sign-in-token"]);
  await expect(card.getByRole("button", { name: "Create your workspace" })).toBeVisible();
});

test("a bad token is refused in plain words", async ({ page }) => {
  await page.goto("/ui/#/board");
  await page.getByLabel(/paste a token/i).fill("not-a-real-token");
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("alert")).toContainText(/token/i);
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeVisible();
});

test("a pasted token signs in and shows the board", async ({ page }) => {
  await page.addInitScript((id) => window.localStorage.setItem("maidan_console_channel", id), fx.board_channel_id);
  await page.goto("/ui/#/board");
  await page.getByLabel(/paste a token/i).fill(fx.token);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  await expect(page.getByRole("heading", { name: "Sign in to Maidan" })).toBeHidden();
  await expect(page.locator(".lane").first()).toBeVisible();
});

test("the board on #build shows a task in each lane", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id);
  for (const lane of ["Open", "In progress", "Needs review", "Done"]) {
    await expect(page.locator("h2.lane-head", { hasText: lane })).toBeVisible();
  }
  for (const title of [
    "Open: nobody holds this",
    "Claimed: the deployer holds this",
    "Review: result waiting on a reviewer",
    "Done: closed",
  ]) {
    await expect(page.locator("button.task", { hasText: title })).toBeVisible();
  }
});

test("needs you lists what is waiting on the operator", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  await expect(page.locator(".needs-count")).toHaveText(/waiting on you/);
  await expect(page.locator(".needs-group").first()).toBeVisible();
});
