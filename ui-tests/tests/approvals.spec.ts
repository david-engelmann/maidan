import { test, expect, type APIRequestContext, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

// The agent asks; the operator answers. No one accepts their own request.
async function openGate(request: APIRequestContext, prompt: string) {
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${fx.requester_token}`, "Content-Type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "request_approval",
        arguments: { prompt, thread_id: fx.thread_id },
      },
    },
  });
  expect(mcp.ok()).toBeTruthy();
}

// The operator signed in as a person: the session cookie an identity
// provider's sign-in sets, with no token behind it. The page sends every
// write through /ui/api on it, from this origin.
async function signIn(page: Page) {
  await page.context().addCookies([
    { name: "maidan_session", value: fx.session_cookie, url: fx.base_url, httpOnly: true, sameSite: "Lax" },
  ]);
}

// The human side of the held gate, in a real browser: an agent opens a gate,
// it appears in the Approvals tab, and Accept resolves it. Accepting needs a
// person's sign-in (or a token holding approval:grant), so the operator is
// signed in, not a pasted token. The gate is created per-run with a unique
// prompt so the spec is retry-safe and independent of any other pending gate.
test("the Approvals tab lists a pending gate and resolves it on Accept", async ({ page, request }) => {
  const prompt = `Ship build ${Date.now()}?`;
  await openGate(request, prompt);

  await signIn(page);
  // Every answer goes to the session proxy, never to the bearer tree.
  const answers: string[] = [];
  page.on("request", (req) => {
    if (req.method() === "POST" && req.url().includes("/approval-gates/")) answers.push(req.url());
  });
  await page.goto("/ui/");
  await expect(page.locator("#identity-mode")).toHaveText("session");
  // Point at the seeded workspace before opening the tab (it loads on click,
  // reading #workspace).
  await page.fill("#workspace", fx.workspace_id);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="approvals"]');

  // Our pending gate renders with its three actions.
  const row = page.locator("#approval-list li.approval-row").filter({ hasText: prompt });
  await expect(row).toBeVisible();
  await expect(row.locator("button.gate-accept")).toBeVisible();
  await expect(row.locator("button.gate-decline")).toBeVisible();
  await expect(row.locator("button.gate-cancel")).toBeVisible();

  // Accept it → the gate resolves (CAS on pending) and leaves the pending list.
  await row.locator("button.gate-accept").click();
  await expect(row).toHaveCount(0);
  expect(answers.length).toBe(1);
  expect(answers[0]).toContain("/ui/api/approval-gates/");
});

// A pasted token is what a person hands an agent, so it cannot accept: the
// row stays, and the page says what accepting needs. Declining still works.
test("a pasted token cannot accept a gate, says what can, and still declines", async ({ page, request }) => {
  const prompt = `Rotate keys ${Date.now()}?`;
  await openGate(request, prompt);

  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await page.locator("#token").press("Enter");
  await expect(page.locator("#identity-mode")).toHaveText("bearer token");
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="approvals"]');

  const row = page.locator("#approval-list li.approval-row").filter({ hasText: prompt });
  await expect(row).toBeVisible();
  await row.locator("button.gate-accept").click();
  const toast = page.locator("#toasts [role=alert]");
  await expect(toast).toContainText("approval:grant");
  await expect(toast).toContainText("signed in through your identity provider");
  await expect(row).toBeVisible();

  await row.locator("button.gate-decline").click();
  await expect(row).toHaveCount(0);
});

test("the visible Approvals tab discovers a new gate without manual refresh", async ({ page, request }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="approvals"]');
  await expect(page.locator("#approval-list li.approval-row").first()).toBeVisible();

  const prompt = `Auto-refresh ${Date.now()}?`;
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${fx.token}`, "Content-Type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: {
        name: "request_approval",
        arguments: { prompt, thread_id: fx.thread_id },
      },
    },
  });
  expect(mcp.ok()).toBeTruthy();

  await expect(
    page.locator("#approval-list li.approval-row").filter({ hasText: prompt }),
  ).toBeVisible({ timeout: 7000 });
});
