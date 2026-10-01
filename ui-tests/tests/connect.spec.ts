import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Connect an agent finishes in the product: it creates a member and mints the
// worker preset. The browser's own token stays put, and the snippets keep a
// placeholder. The minted token can claim.
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
  await expect(page.locator("#token")).toHaveValue(fx.admin_token);

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
  await page.click('.tabs button[data-tab="tokens"]');
  await expect(page.locator("#token-caps")).toHaveValue(
    "workspace:read,workspace:write,message:post,thread:transition",
  );
});
