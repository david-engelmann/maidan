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
  await expect(page.locator(".needs-count")).toHaveText(/waiting on you|Nothing waiting/);
  await expect(page.locator(".needs-group").first()).toBeVisible();
});
