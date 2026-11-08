import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// OAuth consent in a real browser: the consent page's CSP allows form-action
// to the client's redirect origin, so clicking Allow navigates to the
// redirect URI with code and state (not blocked by the browser).
test("OAuth consent Allow lands on the client redirect with code and state", async ({
  page,
  request,
}) => {
  // The playwright-consent-client is seeded via MAIDAN_OAUTH_CLIENTS in
  // playwright.config.ts (there is no POST /oauth/clients; dynamic
  // registration is not implemented by design).

  // Start the authorize flow to get a pending request. Use PKCE S256.
  // (The verifier/challenge are fixed for the test.)
  const verifier = "playwright-pkce-verifier-1234567890";
  const challenge = "7ZJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJx"; // placeholder
  const authUrl =
    `${fx.base_url}/oauth/authorize?response_type=code` +
    `&client_id=playwright-consent-client` +
    `&redirect_uri=${encodeURIComponent("https://client.example/callback")}` +
    `&code_challenge=${challenge}&code_challenge_method=S256` +
    `&scope=workspace%3Aread&state=test-state-123` +
    // The resource is required, and must be the MCP endpoint (RFC 8707).
    `&resource=${encodeURIComponent(`${fx.base_url}/mcp/streamable`)}`;

  // Sign in via the session cookie, then open the authorize URL.
  await page.goto("/ui/");
  await page.context().addCookies([
    {
      name: "maidan_session",
      value: fx.session_cookie,
      domain: new URL(fx.base_url).hostname,
      path: "/",
    },
  ]);

  // The authorize endpoint redirects to the consent page.
  // We use the API to get the pending request id, since the browser
  // would need a bearer token for /oauth/authorize.
  const authRes = await request.get(authUrl, {
    headers: { Authorization: `Bearer ${fx.token}` },
    maxRedirects: 0,
  });
  // 307 to the consent page.
  expect(authRes.status()).toBe(307);
  const location = authRes.headers()["location"];
  expect(location).toMatch(/\/ui\/oauth\/consent\?request=/);
  const requestId = location.split("request=")[1];

  // Open the consent page in the browser.
  await page.goto(`${fx.base_url}/ui/oauth/consent?request=${requestId}`);
  await expect(page.locator("h1")).toContainText("Connect");
  await expect(page.locator("ul.caps")).toContainText("Read channels");
  // The inline stylesheet runs only if the CSP hash admits it: unstyled,
  // the Allow button would keep the browser's square default shape.
  const allow = page.locator('button[name="approved"][value="true"]');
  await expect(allow).toHaveCSS("border-radius", "999px");

  // The page's CSP allows form-action to the client's origin.
  const csp = await page.evaluate(() => {
    const meta = document.querySelector('meta[http-equiv="Content-Security-Policy"]');
    return meta?.getAttribute("content") || "header";
  });
  // CSP is sent as a header, not a meta tag; check via response.
  // (We verify the header in the API test; here we verify navigation works.)

  // Click Allow and wait for navigation to the redirect URI.
  // The redirect goes to https://client.example, which won't resolve;
  // we intercept to verify the URL has code and state.
  const [navRequest] = await Promise.all([
    page.waitForRequest(
      (req) => req.url().startsWith("https://client.example/callback"),
      { timeout: 10000 }
    ).catch(() => null),
    page.click('button[name="approved"][value="true"]'),
  ]);

  // If the navigation was blocked by CSP, navRequest would be null
  // and the page would stay. Check we left the consent page.
  // (In a real browser with network, we'd land on client.example.)
  const url = page.url();
  // The form POST returns 303 to the redirect; the browser follows it.
  // We assert the navigation was attempted (not blocked by CSP).
  expect(navRequest !== null || url.includes("client.example")).toBeTruthy();
  if (navRequest) {
    const navUrl = new URL(navRequest.url());
    expect(navUrl.searchParams.get("code")).toBeTruthy();
    expect(navUrl.searchParams.get("state")).toBe("test-state-123");
  }
});

async function pendingConsent(page: Page, request: APIRequestContext): Promise<void> {
  const challenge = "7ZJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJx";
  const authUrl =
    `${fx.base_url}/oauth/authorize?response_type=code` +
    `&client_id=playwright-consent-client` +
    `&redirect_uri=${encodeURIComponent("https://client.example/callback")}` +
    `&code_challenge=${challenge}&code_challenge_method=S256` +
    `&scope=workspace%3Aread&state=a11y-state` +
    `&resource=${encodeURIComponent(`${fx.base_url}/mcp/streamable`)}`;
  await page.context().addCookies([
    {
      name: "maidan_session",
      value: fx.session_cookie,
      domain: new URL(fx.base_url).hostname,
      path: "/",
    },
  ]);
  const authRes = await request.get(authUrl, {
    headers: { Authorization: `Bearer ${fx.token}` },
    maxRedirects: 0,
  });
  expect(authRes.status()).toBe(307);
  const requestId = authRes.headers()["location"].split("request=")[1];
  await page.goto(`${fx.base_url}/ui/oauth/consent?request=${requestId}`);
  await expect(page.locator("h1")).toContainText("Connect");
}

async function fits(page: Page): Promise<boolean> {
  return page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth);
}

async function shortOf44(page: Page, selector: string): Promise<string[]> {
  return page.locator(selector).evaluateAll((els) =>
    els
      .map((el) => {
        const r = el.getBoundingClientRect();
        return { t: (el.textContent ?? "").replace(/\s+/g, " ").trim().slice(0, 32), w: Math.round(r.width), h: Math.round(r.height) };
      })
      .filter(({ w, h }) => w < 44 || h < 44)
      .map(({ t, w, h }) => `${t} ${w}x${h}`),
  );
}

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true, reducedMotion: "reduce" });

  test("consent and connected apps meet contrast, heading order, and a 44px target", async ({ page, request }) => {
    const csp: string[] = [];
    page.on("console", (msg) => {
      if (/Content Security Policy/i.test(msg.text())) csp.push(msg.text());
    });
    await pendingConsent(page, request);

    // Dark by default; the light palette a page opts into passes as well.
    for (const scheme of ["dark", "light"] as const) {
      await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), scheme);
      const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
      expect(scan.violations.map((v) => `${scheme} ${v.id}`)).toEqual([]);
      const order = await new AxeBuilder({ page }).withRules(["heading-order"]).analyze();
      expect(order.violations.map((v) => v.id)).toEqual([]);
      expect(await fits(page)).toBe(true);
      expect(await shortOf44(page, ".brand")).toEqual([]);
      expect(await shortOf44(page, 'button[name="approved"]')).toEqual([]);
      await expect(page.locator("button").first()).toHaveCSS("transition-duration", "0s");
    }
    const theme = await page.locator('meta[name="theme-color"]').evaluateAll((els) => els.map((el) => el.getAttribute("content")));
    expect(theme).toEqual(["#0c0d0f"]);
    await page.evaluate(() => document.documentElement.removeAttribute("data-theme"));
    await expect(page.locator("body")).toHaveCSS("color", "rgb(236, 235, 231)");

    await page.click('button[name="approved"][value="true"]');
    await page.waitForURL(/chrome-error|chromewebdata|client\.example/);
    await page.goto("/ui/oauth/grants");
    await expect(page.locator("h1")).toHaveText("Connected apps");
    await expect(page.locator("section.app h2").first()).toBeVisible();

    // Dark by default; the light palette a page opts into passes as well.
    for (const scheme of ["dark", "light"] as const) {
      await page.evaluate((t) => document.documentElement.setAttribute("data-theme", t), scheme);
      const scan = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
      expect(scan.violations.map((v) => `${scheme} ${v.id}`)).toEqual([]);
      const order = await new AxeBuilder({ page }).withRules(["heading-order"]).analyze();
      expect(order.violations.map((v) => v.id)).toEqual([]);
      expect(await fits(page)).toBe(true);
      expect(await shortOf44(page, ".brand")).toEqual([]);
      expect(await shortOf44(page, "button.danger")).toEqual([]);
      expect(await shortOf44(page, ".foot a")).toEqual([]);
      await expect(page.locator("button.danger").first()).toHaveCSS("transition-duration", "0s");
    }

    while ((await page.locator("button.danger").count()) > 0) {
      await page.locator("button.danger").first().click();
      await expect(page).toHaveURL(/\/ui\/oauth\/grants/);
    }
    await expect(page.getByText("No apps are connected to this workspace.")).toBeVisible();
    expect(csp, csp.join("\n")).toEqual([]);
  });
});
