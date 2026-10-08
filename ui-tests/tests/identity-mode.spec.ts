import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport, signIn } from "./_tools";

const fx = fixtures();

// The header says which credential the page acts with (a session, a bearer
// token, or a delegated token), and a refusal names that credential: a
// session is never told "your token".

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
const mode = (page: Page) => page.locator("#identity-mode");

interface Provisioned {
  workspace: { id: string; name: string };
  member: { id: string };
  token: { secret: string };
}

// A fresh workspace with its own first admin, so the delegated grant below
// lives somewhere no other spec reads, and the page can show that one
// workspace sees nothing of another.
async function provisionWorkspace(request: APIRequestContext, name: string): Promise<Provisioned> {
  const res = await request.post("/operator/workspaces", {
    headers: bearer(fx.admin_token),
    data: { name, admin_handle: "keeper" },
  });
  expect(res.status()).toBe(201);
  return res.json();
}

// A real delegated token: the admin lends an agent workspace:read as itself,
// and the agent exchanges the grant. /me then names the grant.
async function delegatedToken(request: APIRequestContext, ws: Provisioned): Promise<string> {
  const wid = ws.workspace.id;
  const admin = bearer(ws.token.secret);
  const agentRes = await request.post(`/workspaces/${wid}/members`, {
    headers: admin,
    data: { handle: "helper", display_name: "Helper", kind: "agent" },
  });
  expect(agentRes.status()).toBe(201);
  const agent = await agentRes.json();
  const minted = await request.post(`/workspaces/${wid}/members/${agent.id}/tokens`, {
    headers: admin,
    data: { label: "identity-mode-spec", capabilities: ["workspace:read"] },
  });
  expect(minted.status()).toBe(201);
  const agentToken = (await minted.json()).secret;
  const grant = await request.post(`/workspaces/${wid}/delegation-grants`, {
    headers: admin,
    data: {
      subject_id: ws.member.id,
      delegate_id: agent.id,
      capabilities: ["workspace:read"],
      purpose: "identity mode spec",
      expires_at: new Date(Date.now() + 3600_000).toISOString(),
    },
  });
  expect(grant.status()).toBe(201);
  const exchanged = await request.post("/tokens/delegate", {
    headers: bearer(agentToken),
    data: { grant_id: (await grant.json()).id },
  });
  expect(exchanged.status()).toBe(201);
  return (await exchanged.json()).token.secret;
}

// A refused channel list, so each mode's 403 sentence can be read on the page.
async function refuseChannels(page: Page) {
  await page.route(/\/workspaces\/[^/]+\/channels$/, (route) =>
    route.fulfill({
      status: 403,
      contentType: "application/problem+json",
      body: JSON.stringify({ title: "Forbidden", detail: "needs capability channel:admin" }),
    }),
  );
  await page.click("#refresh-channels");
}

test("no badge until a credential works", async ({ page }) => {
  await page.goto("/ui/");
  await expect(page.locator("#session-status")).toBeHidden();
  await expect(mode(page)).toBeHidden();
  await expect(mode(page)).toHaveText("");
});

test("a bearer token shows bearer token, and a refusal says token", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.token);
  await expect(mode(page)).toBeVisible();
  await expect(mode(page)).toHaveText("bearer token");
  await expect(mode(page)).toHaveAttribute("data-mode", "bearer");
  await expect(mode(page)).toHaveAttribute("title", /^Bearer token/);
  // Type, like the name beside it: no chip.
  await expect(mode(page)).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(mode(page)).toHaveCSS("border-top-width", "0px");

  await refuseChannels(page);
  await expect(page.locator("#channel-list")).toContainText(
    "Could not load channels: Your token is not allowed to do this; it needs channel:admin. Mint a token with it in Tokens",
  );
});

test("a session shows session, and its refusals never say token", async ({ page }) => {
  // The harness has no identity provider. A session the page did not make
  // from a token answers GET /auth/session with no token_id; that answer is
  // all the page reads to tell the two apart.
  await signIn(page, fx.workspace_id, fx.token);
  await page.route(/\/auth\/session$/, async (route) => {
    if (route.request().method() !== "GET") return route.continue();
    const res = await route.fetch();
    await route.fulfill({ response: res, json: { ...(await res.json()), token_id: null } });
  });
  await page.reload();
  await expect(mode(page)).toHaveText("session");
  await expect(mode(page)).toHaveAttribute("data-mode", "session");

  await refuseChannels(page);
  const list = page.locator("#channel-list");
  await expect(list).toContainText(
    "Could not load channels: Your session is not allowed to do this; it needs channel:admin. A session cannot mint tokens, so use Change to paste a token that has it",
  );
  await expect(list).not.toContainText(/your token/i);
  await expect(list).not.toContainText("Mint a token");

  await page.route("**/waiting*", (route) => route.fulfill({ status: 401, body: "" }));
  await callUiExport(page, "needs.js", "loadNeedsYou");
  const state = page.locator("#needs-you-state");
  await expect(state).toHaveText(
    "Could not load what is waiting on you: Your session was not accepted; it may have ended. Sign in again, or use Change to paste a token",
  );
  await expect(state).not.toContainText(/your token|token or session/i);
});

test("a delegated token shows delegated token, and a second workspace sees nothing of the first", async ({ page, request }) => {
  const other = await provisionWorkspace(request, `Identity mode elsewhere ${Date.now()}`);
  const delegated = await delegatedToken(request, other);

  // Through the API: the delegated token is the second workspace's, and the
  // first workspace's channels and identity are not its to read.
  const me = await request.get("/me", { headers: bearer(delegated) });
  expect(me.ok()).toBeTruthy();
  const who = await me.json();
  expect(who.workspace_id).toBe(other.workspace.id);
  expect(who.member_id).toBe(other.member.id);
  expect(who.delegation_grant_id).toBeTruthy();
  for (const token of [delegated, other.token.secret]) {
    const peek = await request.get(`/workspaces/${fx.workspace_id}/channels`, { headers: bearer(token) });
    expect([403, 404]).toContain(peek.status());
    const peekWs = await request.get(`/workspaces/${fx.workspace_id}`, { headers: bearer(token) });
    expect([403, 404]).toContain(peekWs.status());
  }

  // On the page: the badge says delegated, the header names the second
  // workspace, and nothing of the first shows.
  await signIn(page, other.workspace.id, delegated);
  await expect(mode(page)).toHaveText("delegated token");
  await expect(mode(page)).toHaveAttribute("data-mode", "delegated");
  await expect(page.locator("#identity-ws")).toHaveText(other.workspace.name);
  await expect(page.locator("header")).not.toContainText("UI Test Workspace");
  await expect(page.locator("#channel-list")).not.toContainText("general");
  await expect(page.locator("#channel-list")).not.toContainText("build");

  await refuseChannels(page);
  await expect(page.locator("#channel-list")).toContainText(
    "Could not load channels: This delegated token is not allowed to do this; it needs channel:admin. It holds only what its grant lends: ask for a grant that includes it",
  );
  await page.unroute(/\/workspaces\/[^/]+\/channels$/);

  // And the first workspace's page shows its own bearer badge and nothing
  // of the second.
  await page.context().clearCookies();
  await signIn(page, fx.workspace_id, fx.token);
  await expect(mode(page)).toHaveText("bearer token");
  await expect(page.locator("#identity-ws")).toHaveText("UI Test Workspace");
  await expect(page.locator("header")).not.toContainText(other.workspace.name);
});
