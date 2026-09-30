import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

// A blank browser is walked from server and workspace to a signed-in board:
// the connection inputs sit in a first-run card until a credential works,
// then fold into the header. The identity-provider button appears only when
// the server's discovery document says it has one.
const fx = fixtures();

test("a first visit shows the connection card, not header paste fields", async ({ page }) => {
  await page.goto("/ui/");
  const card = page.locator("#first-run");
  await expect(card).toBeVisible();
  await expect(card.locator("#workspace")).toBeVisible();
  await expect(card.locator("#token")).toBeVisible();
  await expect(page.locator("header #workspace")).toHaveCount(0);
  await expect(card).toContainText("maidan init");
  await expect(card).toContainText("This browser exchanges it for a session and does not keep the token");
  await expect(page.locator("#first-run-oidc"), "this server has no identity provider").toBeHidden();
  await expect(page.locator("#logout")).toBeHidden();
});

test("a token connects, folds the inputs into the header, and Sign out forgets it", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  await page.locator("#token").dispatchEvent("change");

  await expect(page.locator("#first-run")).toBeHidden();
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#identity-who")).toContainText("Operator");
  await page.click("#conn-edit");
  await expect(page.locator("header #conn-fields")).toBeVisible();
  await expect(page.locator("header #token")).toHaveValue(fx.review_token);

  const signOut = page.locator("#logout");
  await expect(signOut).toHaveText("Sign out");
  await signOut.click();
  await expect(page.locator("#first-run")).toBeVisible();
  await expect(page.locator("#token")).toHaveValue("");
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeFalsy();
});

test("the identity-provider button appears only when the server has one", async ({ page }) => {
  await page.route("**/.well-known/maidan.json", async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    body.auth = { bearer: true, oidc: true, oidc_login: "/auth/oidc/login" };
    await route.fulfill({ response: res, json: body });
  });
  let loginUrl = "";
  await page.route(/\/auth\/oidc\/login\?/, async (route) => {
    loginUrl = route.request().url();
    await route.fulfill({ status: 200, contentType: "text/plain", body: "identity provider" });
  });
  await page.goto("/ui/");
  const oidc = page.getByRole("button", { name: "Sign in with your identity provider" });
  await expect(oidc).toBeVisible();
  await page.fill("#workspace", fx.workspace_id);
  await oidc.click();
  await expect.poll(() => loginUrl).toContain(`workspace_id=${fx.workspace_id}`);
});

test("a saved token the server refuses says so and keeps the card open", async ({ page }) => {
  await page.addInitScript(
    ([ws]) => {
      localStorage.setItem("maidan_workspace", ws);
      localStorage.setItem("maidan_token", "revoked-long-ago");
    },
    [fx.workspace_id],
  );
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toContainText("did not sign in");
  await expect(page.locator("#first-run")).toBeVisible();
  await expect(page.locator("#identity-pill")).toBeHidden();
});
