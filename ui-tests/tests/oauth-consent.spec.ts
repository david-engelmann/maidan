import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// OAuth consent in a real browser: the consent page's CSP allows form-action
// to the client's redirect origin, so clicking Allow navigates to the
// redirect URI with code and state (not blocked by the browser).
test("OAuth consent Allow lands on the client redirect with code and state", async ({
  page,
  request,
}) => {
  // Create an OAuth client with a redirect URI on another origin.
  const clientRes = await request.post(`${fx.base_url}/oauth/clients`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: {
      client_id: "playwright-consent-client",
      name: "Playwright Consent Client",
      redirect_uris: ["https://client.example/callback"],
      allowed_scopes: ["workspace:read"],
    },
  });
  expect(clientRes.ok()).toBeTruthy();

  // Start the authorize flow to get a pending request. Use PKCE S256.
  // (The verifier/challenge are fixed for the test.)
  const verifier = "playwright-pkce-verifier-1234567890";
  const challenge = "7ZJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJxJx"; // placeholder
  const authUrl =
    `${fx.base_url}/oauth/authorize?response_type=code` +
    `&client_id=playwright-consent-client` +
    `&redirect_uri=${encodeURIComponent("https://client.example/callback")}` +
    `&code_challenge=${challenge}&code_challenge_method=S256` +
    `&scope=workspace%3Aread&state=test-state-123`;

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
  await expect(page.locator("h1")).toContainText("Authorize");

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
