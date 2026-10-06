import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

// Connect an agent finishes in the product: it creates a member and mints the
// worker preset. The browser keeps the session it already exchanged, and does
// not put either secret back in the field. The snippets keep a placeholder.
// The minted token can claim.
test("creating an agent mints a token that can claim, post, and transition", async ({ page, request }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.admin_token);
  await page.locator("#token").dispatchEvent("change");
  await page.getByRole("button", { name: "Connect an agent" }).first().click();

  const handle = `coder-${Date.now()}`;
  await page.fill("#cx-handle", handle);
  await page.fill("#cx-name", "Coder");
  await page.click("#cx-create-agent");

  const secret = page.locator("#cx-secret-value");
  await expect(secret).toBeVisible();
  const token = (await secret.textContent())!.trim();
  expect(token.length).toBeGreaterThan(20);
  await expect(page.locator("#cx-json")).toContainText("REPLACE_WITH_MAIDAN_TOKEN");
  await expect(page.locator("#cx-secret")).toContainText("thread:transition");
  await expect(page.locator("#token")).toHaveValue("");
  expect(await page.evaluate(() => localStorage.getItem("maidan_token"))).toBeNull();
  expect(token).not.toBe(fx.admin_token);

  const me = await request.get(`${fx.base_url}/me`, { headers: { Authorization: `Bearer ${token}` } });
  expect(me.ok()).toBeTruthy();
  const caps: string[] = (await me.json()).capabilities;
  for (const needed of ["workspace:read", "message:post", "thread:transition"]) {
    expect(caps, needed).toContain(needed);
  }

  const channel = await request.post(`${fx.base_url}/workspaces/${fx.workspace_id}/channels`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { name: `lane-${handle}`, topic: "demo data" },
  });
  expect(channel.ok()).toBeTruthy();
  const channelId = (await channel.json()).id;
  const opened = await request.post(`${fx.base_url}/channels/${channelId}/threads`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { title: "Claim me" },
  });
  expect(opened.ok()).toBeTruthy();
  const threadId = (await opened.json()).id;

  const claim = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "claim_next_thread", arguments: { channel_id: channelId, lease_secs: 900 } },
    },
  });
  expect(claim.ok()).toBeTruthy();
  const body = await claim.json();
  expect(body.result?.isError ?? false).toBeFalsy();
  const claimed = JSON.parse(body.result.content[0].text);
  expect(claimed.id).toBe(threadId);

  // The Tokens form default is the same grant, not the read/write/post preset
  // that claim refuses.
  await page.locator("#connect-dialog").evaluate((d: HTMLDialogElement) => d.close());
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="tokens"]');
  await expect(page.locator("#token-caps")).toHaveValue(
    "workspace:read,workspace:write,message:post,thread:transition",
  );
});

// Once the member exists a retry cannot create it again: the handle is taken.
// A mint the server refuses (4xx) names the member and puts its id in Tokens.
test("a refused mint after the member is created points Tokens at that member", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.admin_token);
  await page.locator("#token").dispatchEvent("change");
  await page.route(/\/members\/[^/]+\/tokens$/, (route) =>
    route.request().method() === "POST" ? route.fulfill({ status: 403, body: "nope" }) : route.continue(),
  );
  await page.getByRole("button", { name: "Connect an agent" }).first().click();

  const handle = `nomint-${Date.now()}`;
  await page.fill("#cx-handle", handle);
  await page.click("#cx-create-agent");

  const status = page.locator("#cx-status");
  await expect(status).toContainText(`Member ${handle} was created, but the server refused to mint its token`);
  await expect(status).toContainText("Mint its token in Tokens");
  await expect(status).not.toContainText("nope");
  await expect(page.locator("#token-member")).toHaveValue(/^[0-9a-f-]{36}$/);
  await expect(page.locator("#cx-create-agent")).toBeEnabled();
});

// mint_api_token can commit the token before the quota listing fails, so a
// 500 leaves the outcome unknown: the page must not claim the server refused,
// because a token may exist that this page never saw.
test("a 500 from the mint says the outcome is unknown", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.admin_token);
  await page.locator("#token").dispatchEvent("change");
  await page.route(/\/members\/[^/]+\/tokens$/, (route) =>
    route.request().method() === "POST" ? route.fulfill({ status: 500, body: "boom" }) : route.continue(),
  );
  await page.getByRole("button", { name: "Connect an agent" }).first().click();

  const handle = `unkmint-${Date.now()}`;
  await page.fill("#cx-handle", handle);
  await page.click("#cx-create-agent");

  const status = page.locator("#cx-status");
  await expect(status).toContainText(
    `Member ${handle} was created, but the server failed while minting its token, so a token may exist that this page never saw.`,
  );
  await expect(status).toContainText("Mint its token in Tokens");
  await expect(status).not.toContainText("boom");
  await expect(page.locator("#token-member")).toHaveValue(/^[0-9a-f-]{36}$/);
  await expect(page.locator("#cx-create-agent")).toBeEnabled();
});

// A mint the server made but whose reply the page could not read: the secret
// cannot be shown again, so the page says so instead of staying on "Minting…".
test("an unreadable mint reply says the token cannot be shown", async ({ page }) => {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.admin_token);
  await page.locator("#token").dispatchEvent("change");
  await page.route(/\/members\/[^/]+\/tokens$/, (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 201, contentType: "application/json", body: "{not json" })
      : route.continue(),
  );
  await page.getByRole("button", { name: "Connect an agent" }).first().click();

  const handle = `garbled-${Date.now()}`;
  await page.fill("#cx-handle", handle);
  await page.click("#cx-create-agent");

  const status = page.locator("#cx-status");
  await expect(status).toContainText(`Member ${handle} was created`);
  await expect(status).toContainText("cannot be shown");
  await expect(page.locator("#token-member")).toHaveValue(/^[0-9a-f-]{36}$/);
  await expect(page.locator("#cx-create-agent")).toBeEnabled();
});
