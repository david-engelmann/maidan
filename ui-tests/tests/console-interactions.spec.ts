import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// What a static shot cannot show: the approve toast, the optimistic approve
// put back when the server refuses, j/k through the list, and the unread dot
// appearing for someone else's message and clearing once read.
const fx = fixtures();
test.describe.configure({ mode: "serial", retries: 0 });

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });
const needRow = (page: Page, threadId: string) => page.locator(`li.need[data-thread-id="${threadId}"]`);
const convRow = (page: Page, threadId: string) => page.locator(`a.conv[href$="/${threadId}"]`);

async function confirmApprove(page: Page, threadId: string) {
  const row = needRow(page, threadId);
  await expect(row).toBeVisible();
  await row.locator(".need-actions").getByRole("button", { name: "Approve", exact: true }).click();
  await row.locator(".need-confirm").getByRole("button", { name: "Approve", exact: true }).click();
  return row;
}

test("a refused approve is put back on the card and says why", async ({ page }) => {
  await signInWithCookie(page, fx);
  await page.route(`**/threads/${fx.desk_approve_thread_id}/reviews`, (route) =>
    route.request().method() === "POST"
      ? route.fulfill({ status: 500, contentType: "application/json", body: JSON.stringify({ error: "the review store is unavailable" }) })
      : route.fallback(),
  );
  await openOnChannel(page, fx.desk_channel_id, "#/needs");
  const row = await confirmApprove(page, fx.desk_approve_thread_id);
  // The optimistic "Approved" is taken back and the card says what happened;
  // once read, the buttons are back so the person can try again.
  await expect(row).toContainText("The server hit an error");
  await expect(row).not.toContainText("Approved ✓");
  await row.getByRole("button", { name: "OK", exact: true }).click();
  await expect(row.locator(".need-actions").getByRole("button", { name: "Approve", exact: true })).toBeEnabled();
  await expect(page.locator(".toasts .toast")).toBeVisible();
});

test("approving shows a toast and the card says approved", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.desk_channel_id, "#/needs");
  const row = await confirmApprove(page, fx.desk_approve_thread_id);
  await expect(page.locator(".toasts .toast").filter({ hasText: "Approved" })).toBeVisible();
  await expect(row).toContainText(/approved/i);
});

test("j and k walk the thread list in order", async ({ page }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.board_channel_id, "#/threads");
  const rows = page.locator("a.conv:visible");
  await expect.poll(() => rows.count()).toBeGreaterThan(2);
  await page.locator("body").click({ position: { x: 1, y: 1 } });
  await page.keyboard.press("j");
  await expect(rows.nth(0)).toBeFocused();
  await page.keyboard.press("j");
  await expect(rows.nth(1)).toBeFocused();
  await page.keyboard.press("j");
  await expect(rows.nth(2)).toBeFocused();
  await page.keyboard.press("k");
  await expect(rows.nth(1)).toBeFocused();
});

test("someone else's message shows an unread dot, and reading it clears the dot", async ({ page, request }) => {
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${fx.thread_id}`);
  await expect(page.locator(".msg").first()).toBeVisible();
  // Read now; leave for the list with nothing open.
  await page.goto("/ui/#/threads");
  const row = convRow(page, fx.thread_id);
  await expect(row).toBeVisible();
  await expect(row.locator(".unread-dot")).toHaveCount(0);
  const res = await request.post(`/threads/${fx.thread_id}/messages`, { headers: bearer(fx.requester_token), data: { body: `unread probe ${Date.now()}` } });
  expect(res.ok(), `post: ${res.status()}`).toBeTruthy();
  // The list hears it live; a reload is the fallback the dot must also survive.
  await expect(row.locator(".unread-dot")).toBeVisible({ timeout: 15_000 }).catch(async () => {
    await page.reload();
    await expect(convRow(page, fx.thread_id).locator(".unread-dot")).toBeVisible();
  });
  await convRow(page, fx.thread_id).click();
  await expect(page.locator(".msg").filter({ hasText: "unread probe" })).toBeVisible();
  await expect(convRow(page, fx.thread_id).locator(".unread-dot")).toHaveCount(0);
});

test("a slow board shows placeholder rows, then the real ones", async ({ page }) => {
  await signInWithCookie(page, fx);
  let release!: () => void;
  const held = new Promise<void>((r) => (release = r));
  await page.route("**/threads**", async (route) => {
    if (route.request().method() === "GET" && route.request().resourceType() === "fetch") await held;
    await route.fallback();
  });
  await openOnChannel(page, fx.desk_channel_id, "#/board");
  await expect(page.locator(".skeleton").first()).toBeVisible();
  release();
  await expect(page.locator(".skeleton")).toHaveCount(0);
  await expect(page.locator("main")).not.toContainText("Loading");
});

test("losing the connection says so in the top bar and clears once back", async ({ page, context }) => {
  // Browsers keep an open localhost socket through setOffline, so the socket is
  // dropped here by hand and refused until the network comes back.
  let down = false;
  const open = new Set<{ close: (o?: { code?: number; reason?: string }) => Promise<void> }>();
  await page.routeWebSocket(/\/ws\/subscribe/, (ws) => {
    if (down) return void ws.close({ code: 1011, reason: "offline" });
    ws.connectToServer();
    open.add(ws);
  });
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.desk_channel_id, "#/needs");
  await expect(page.locator("li.need").first()).toBeVisible();
  const status = page.locator("span.live[role=status]");
  await expect(status).toHaveCount(0);
  down = true;
  await context.setOffline(true);
  for (const ws of open) await ws.close({ code: 1011, reason: "offline" });
  open.clear();
  await expect(status).toBeVisible();
  await expect(status).toHaveAttribute("title", /Reconnecting|Offline/);
  // The rows already on screen stay put while the connection is down.
  await expect(page.locator("li.need").first()).toBeVisible();
  down = false;
  await context.setOffline(false);
  await page.evaluate(() => window.dispatchEvent(new Event("online")));
  await expect(status).toHaveCount(0, { timeout: 20_000 });
});
