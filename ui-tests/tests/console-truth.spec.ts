import { test, expect, type APIRequestContext, type Locator, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// What the console says must be what the server holds: a refused close shows
// on its card, Needs you keeps its rows and says why when it cannot refresh,
// a review nobody owns still reaches someone, an approval is accepted only
// through its confirmation link, a decline is recorded, and evidence carries
// its tier.
const fx = fixtures();
test.describe.configure({ retries: 0 });

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
const needRow = (page: Page, threadId: string) => page.locator(`li.need[data-thread-id="${threadId}"]`);

async function mcp(request: APIRequestContext, token: string, name: string, args: object) {
  const res = await request.post("/mcp", {
    headers: { ...bearer(token), "Content-Type": "application/json" },
    data: { jsonrpc: "2.0", id: 1, method: "tools/call", params: { name, arguments: args } },
  });
  expect(res.ok()).toBeTruthy();
  const body = await res.json();
  expect(body.error, JSON.stringify(body.error)).toBeUndefined();
  return JSON.parse(body.result.content[0].text);
}

/** Make a live event in the operator's workspace, so Needs you refreshes. Returns a cleanup. */
async function poke(request: APIRequestContext): Promise<() => Promise<void>> {
  const res = await request.post(`/threads/${fx.thread_id}/messages`, { headers: bearer(fx.token), data: { body: `poke ${Date.now()}` } });
  expect(res.ok(), `poke: ${res.status()}`).toBeTruthy();
  return async () => {};
}

/** The live socket subscribes a moment after the page loads, so poke until the page reacts. */
async function pokeUntil(request: APIRequestContext, shown: Locator): Promise<() => Promise<void>> {
  await expect
    .poll(async () => {
      await poke(request);
      await shown.waitFor({ timeout: 1_500 }).catch(() => undefined);
      return shown.isVisible();
    }, { timeout: 20_000, intervals: [0] })
    .toBe(true);
  return async () => {};
}

async function gateState(request: APIRequestContext, gateId: string): Promise<{ state: string; content?: unknown }> {
  return mcp(request, fx.token, "get_approval_gate", { gate_id: gateId });
}

test("a refused close is visible on its card, with the server's sentence", async ({ page, request }) => {
  const res = await request.post(`/threads/${fx.desk_waiting_thread_id}`, { headers: bearer(fx.review_token), data: { action: "close" } });
  expect(res.status()).toBe(409);
  expect((await res.json()).detail).toContain("review requirement not met");
  const list = await (await request.get(`/threads/${fx.desk_waiting_thread_id}/messages?limit=20`, { headers: bearer(fx.review_token) })).json();
  const note = list.find((m: { metadata?: { notice?: string } }) => m.metadata?.notice === "transition_refused");
  expect(note.body).toContain("review requirement not met");

  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.desk_channel_id);
  const card = page.locator("button.task", { has: page.locator(".chip-refused") });
  await expect(card).toHaveCount(1);
  await expect(card.locator(".chip-refused")).toHaveText("Close refused");
  await expect(card.locator(".chip-refused")).toHaveAttribute("title", note.body);
  await expect(card).toHaveAttribute("aria-label", /close refused$/);
  const after = await (await request.get(`/threads/${fx.desk_waiting_thread_id}`, { headers: bearer(fx.review_token) })).json();
  expect(after.state).toBe("in_review");
});

test("Needs you keeps its rows and says it is stale when the server stops answering", async ({ page, request }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  const rows = page.locator("li.need");
  await expect(rows.first()).toBeVisible();
  const before = await rows.count();
  await page.route("**/waiting*", (route) => route.abort("connectionrefused"));
  const done = await pokeUntil(request, page.locator(".needs-stale"));
  await expect(page.locator(".needs-stale")).toContainText(/Stale since .*Reconnecting/);
  expect(await rows.count(), "the rows stay while it reconnects").toBe(before);
  await page.unroute("**/waiting*");
  await done();
});

test("a revoked token keeps the rows and names the fix", async ({ page, request }) => {
  const minted = await (await request.post(`/workspaces/${fx.workspace_id}/members/${fx.member_id}/tokens`, {
    headers: bearer(fx.admin_token),
    data: { label: "needs-truth", capabilities: ["workspace:read", "thread:transition"] },
  })).json();
  await page.goto("/ui/#/needs");
  await page.getByLabel(/paste a token/i).fill(minted.secret);
  await page.getByRole("button", { name: "Sign in with token" }).click();
  const rows = page.locator("li.need");
  await expect(rows.first()).toBeVisible();
  const before = await rows.count();
  expect((await request.delete(`/tokens/${minted.id}`, { headers: bearer(fx.admin_token) })).ok()).toBeTruthy();
  const stale = page.locator(".needs-stale");
  const done = await pokeUntil(request, stale);
  await expect(stale).not.toHaveText(/^Stale since/);
  await expect(stale).toContainText(/token|sign in/i);
  expect(await rows.count()).toBe(before);
  await done();
});

test("a review nobody owns reaches the admins and says nothing was handed over", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.triage_channel_id, "#/needs");
  const row = needRow(page, fx.triage_ownerless_thread_id);
  await expect(row).toHaveAttribute("data-kind", "unassigned_review");
  await expect(row.locator(".need-kind")).toHaveText("Review");
  await expect(row.locator(".need-line").first()).toHaveText("no reviewer named");
  await expect(row.locator(".need-evidence")).toContainText(/no result|Nothing was handed/i);
});

test("the owner closes a review nobody was named for, and the board says closed without review", async ({ page, request }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.triage_channel_id, "#/needs");
  const row = needRow(page, fx.triage_owned_thread_id);
  await expect(row).toHaveAttribute("data-kind", "unassigned_review");
  await expect(row.getByRole("button", { name: "Approve", exact: true })).toHaveCount(0);
  await row.getByRole("button", { name: "Close without review" }).click();
  await row.getByRole("button", { name: "Close task" }).click();
  await expect.poll(async () => (await (await request.get(`/threads/${fx.triage_owned_thread_id}`, { headers: bearer(fx.review_token) })).json()).state).toBe("closed");
  const thread = await (await request.get(`/threads/${fx.triage_owned_thread_id}`, { headers: bearer(fx.review_token) })).json();
  expect(thread.closed_without_review).toBe(true);
  await page.goto("/ui/#/board");
  const card = page.locator("button.task", { has: page.locator(".task-title", { hasText: thread.title }) });
  await expect(card.locator(".state")).toHaveText("closed without review");
  await card.click();
  await expect(page.getByRole("dialog")).toContainText("Closed by");
  await expect(page.getByRole("dialog")).not.toContainText("Completed by");
});

test("a model's accept waits for the person to confirm through the link", async ({ page, request }) => {
  const prompt = `Ship build ${Date.now()}?`;
  const { gate_id } = await mcp(request, fx.requester_token, "request_approval", { prompt, thread_id: fx.thread_id });
  const asked = await mcp(request, fx.token, "approval_decide", { gate_id, decision: "accept" });
  expect((await gateState(request, gate_id)).state, "the model's accept alone decides nothing").toBe("pending");
  const hash = new URL(asked.confirmation_url as string, fx.base_url).hash;
  expect(hash).toContain(`#confirm-approval=${gate_id}.`);

  await signInWithCookie(page, fx);
  await page.goto(`/ui/${hash}`);
  const panel = page.getByRole("region", { name: "Confirm an approval" });
  await expect(panel).toContainText(prompt);
  expect(page.url(), "the link token leaves the address bar").not.toContain(hash.slice(hash.indexOf(".") + 1));
  await panel.getByRole("button", { name: "Confirm" }).click();
  await expect(panel).toHaveCount(0);
  await expect.poll(async () => (await gateState(request, gate_id)).state).toBe("accepted");
});

test("a decline from Needs you is recorded with its reason", async ({ page, request }) => {
  const prompt = `Rotate keys ${Date.now()}?`;
  const { gate_id } = await mcp(request, fx.requester_token, "request_approval", { prompt, thread_id: fx.thread_id });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  const row = page.locator("li.need.need-open_gate", { hasText: prompt });
  await row.getByRole("button", { name: "Decline" }).click();
  await row.getByLabel(/Why decline/).fill("not this week");
  await row.locator("form.need-note").getByRole("button", { name: "Decline" }).click();
  await expect.poll(async () => (await gateState(request, gate_id)).state).toBe("declined");
  expect((await gateState(request, gate_id)).content).toEqual({ note: "not this week" });
});

test("evidence shows each item's tier, and warns when it is all self-reported", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/needs");
  const attached = needRow(page, fx.tiers_attached_thread_id).locator(".need-evidence");
  await expect(attached).toHaveAttribute("data-state", "ready");
  await expect(attached.locator('.ev-item[data-ev="result"] .ev-tier')).toHaveAttribute("data-tier", "self_reported");
  await expect(attached.locator(".ev-tier[data-tier=attached]")).toHaveCount(1);
  await expect(attached.locator(".ev-warn")).toHaveCount(0);

  const verified = needRow(page, fx.tiers_verified_thread_id).locator(".need-evidence");
  await expect(verified.locator('.ev-item[data-ev="land_gate"] .ev-kind')).toHaveText("land-gate pass");
  await expect(verified.locator('.ev-item[data-ev="land_gate"] .ev-tier')).toHaveAttribute("data-tier", "verified");

  const self = needRow(page, fx.tiers_self_thread_id).locator(".need-evidence");
  await expect(self.locator(".ev-warn")).toContainText("Self-reported only");
  expect(await self.locator(".ev-tier").evaluateAll((els) => els.map((e) => (e as HTMLElement).dataset.tier))).toEqual(
    expect.arrayContaining(["self_reported"]),
  );
  await expect(self.locator(".ev-tier:not([data-tier=self_reported])")).toHaveCount(0);
  // Approve stays behind the evidence: the buttons come after it in the row.
  const row = needRow(page, fx.tiers_attached_thread_id);
  const order = await row.evaluate((li) => {
    const ev = li.querySelector(".need-evidence");
    const ok = [...li.querySelectorAll("button")].find((b) => b.textContent === "Approve");
    return ev && ok ? ev.compareDocumentPosition(ok) & Node.DOCUMENT_POSITION_FOLLOWING : 0;
  });
  expect(order).toBeTruthy();
});
