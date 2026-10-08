import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport, signIn } from "./_tools";

const fx = fixtures();

// A task blocked until a person clears it waits on its owner in Needs you,
// with an Unblock button. The panel's refused and stale states hold for it as
// they do for reviews and gates, and another workspace never sees it.

const NOTE = "rotate the signing key by hand";
const HOLD_TITLE = "Held up: the signing key needs a person";
const SECOND_TITLE = "Afar: the second workspace's blocked task";

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });

async function reloadQueue(page: Page) {
  await callUiExport(page, "needs.js", "loadNeedsYou");
}

// The spec blocks the operator's task itself, so a retry starts from the
// same place. The admin grant holds thread:transition.
async function blockHold(request: APIRequestContext) {
  const res = await request.put(`/threads/${fx.hold_thread_id}/block`, {
    headers: bearer(fx.admin_token),
    data: { reason: "human", note: NOTE },
  });
  expect(res.ok()).toBeTruthy();
}

async function holdBlockStatus(request: APIRequestContext): Promise<number> {
  const res = await request.get(`/threads/${fx.hold_thread_id}/block`, {
    headers: bearer(fx.admin_token),
  });
  return res.status();
}

async function waitingThreadIds(
  request: APIRequestContext,
  token: string,
  memberId: string,
): Promise<string[]> {
  const res = await request.get(`/members/${memberId}/waiting`, { headers: bearer(token) });
  expect(res.ok()).toBeTruthy();
  const body = await res.json();
  return body.items.map((i: { thread_id?: string }) => i.thread_id).filter(Boolean);
}

test.afterEach(async ({ request }) => {
  // Leave the task unblocked for the next spec. 404 means already clear.
  const res = await request.delete(`/threads/${fx.hold_thread_id}/block`, {
    headers: bearer(fx.admin_token),
  });
  expect([204, 404]).toContain(res.status());
});

test("a blocked task waits on its owner in Needs you, and Unblock clears it", async ({ page, request }) => {
  await blockHold(request);
  await signIn(page, fx.workspace_id, fx.review_token);

  const blocked = row(page, fx.hold_thread_id);
  await expect(blocked).toBeVisible();
  await expect(blocked).toHaveAttribute("data-kind", "blocked");
  await expect(blocked.locator(".ny-kind")).toHaveText("Blocked");
  await expect(blocked.locator(".ny-title")).toHaveText(HOLD_TITLE);
  // The reason and note follow the title once, not the title again.
  await expect(blocked.locator(".ny-sub")).toContainText(`blocked (human): ${NOTE}`);
  await expect(blocked.locator(".ny-sub")).not.toContainText(HOLD_TITLE);
  await expect(page).toHaveTitle(/^\(\d+\) Maidan$/);
  const before = Number(await page.locator("#needs-you-count").textContent());

  await blocked.getByRole("button", { name: "Unblock" }).click();
  await expect(blocked).toHaveCount(0);
  expect(await holdBlockStatus(request)).toBe(404);
  await expect(page.locator("#needs-you-count")).toHaveText(before > 1 ? String(before - 1) : "");

  // A fresh load agrees: the block is gone on the server, not just the row.
  await reloadQueue(page);
  await expect(page.locator("#needs-you-state")).toBeHidden();
  await expect(blocked).toHaveCount(0);
});

test("a refused or unanswered Unblock keeps the row and says why, and the next try clears it", async ({ page, request }) => {
  await blockHold(request);
  await signIn(page, fx.workspace_id, fx.review_token);
  const blocked = row(page, fx.hold_thread_id);
  const unblock = blocked.getByRole("button", { name: "Unblock" });
  await expect(unblock).toBeVisible();

  await page.route("**/block", (route) =>
    route.request().method() === "DELETE" ? route.fulfill({ status: 503, body: "" }) : route.continue(),
  );
  await unblock.click();
  await expect(blocked.locator(".ny-err")).toHaveText(/^Could not unblock: The server hit an error\./);
  expect(await holdBlockStatus(request)).toBe(200);
  await page.unroute("**/block");

  await page.route("**/block", (route) =>
    route.request().method() === "DELETE" ? route.abort("internetdisconnected") : route.continue(),
  );
  await unblock.click();
  await expect(blocked.locator(".ny-err")).toHaveText("Could not unblock: the server did not answer. Try again.");
  // A reload keeps the row that is showing the error.
  await reloadQueue(page);
  await expect(blocked.locator(".ny-err")).toHaveText("Could not unblock: the server did not answer. Try again.");
  await page.unroute("**/block");

  // A row that showed an error still leaves once the block is cleared.
  await blocked.getByRole("button", { name: "Unblock" }).click();
  await expect(blocked).toHaveCount(0);
  expect(await holdBlockStatus(request)).toBe(404);
});

test("a blocked row stays through a refused load and a dropped connection", async ({ page, request }) => {
  await blockHold(request);
  await signIn(page, fx.workspace_id, fx.review_token);
  const blocked = row(page, fx.hold_thread_id);
  await expect(blocked).toBeVisible();
  const title = await page.title();
  const state = page.locator("#needs-you-state");

  await page.route("**/waiting*", (route) => route.fulfill({ status: 503, body: "" }));
  await reloadQueue(page);
  await expect(state).toHaveText(/^Could not load what is waiting on you: The server hit an error\./);
  await expect(state).toHaveClass(/\berr\b/);
  await expect(blocked).toBeVisible();
  await expect(page).toHaveTitle(title);
  await page.unroute("**/waiting*");

  await page.route("**/waiting*", (route) => route.abort("internetdisconnected"));
  await reloadQueue(page);
  await expect(state).toHaveText(/^Stale since \d{1,2}:\d{2}.*: could not reach the server\. Reconnecting…$/);
  await expect(state).not.toHaveClass(/\berr\b/);
  await expect(blocked).toBeVisible();
  await expect(blocked.getByRole("button", { name: "Unblock" })).toBeVisible();
  await page.unroute("**/waiting*");
  // The load retries on its own and clears the stale line.
  await expect(state).toBeHidden({ timeout: 15_000 });
  await expect(blocked).toBeVisible();
});

test("a second workspace sees nothing of the first's blocked task, and cannot clear it", async ({ page, request }) => {
  await blockHold(request);

  // Through the API: each member's inbox lists only its own workspace's block.
  const mine = await waitingThreadIds(request, fx.review_token, fx.member_id);
  expect(mine).toContain(fx.hold_thread_id);
  expect(mine).not.toContain(fx.second_thread_id);
  const theirs = await waitingThreadIds(request, fx.second_token, fx.second_member_id);
  expect(theirs).toContain(fx.second_thread_id);
  expect(theirs).not.toContain(fx.hold_thread_id);

  // The second workspace cannot read the first's inbox or block, or clear it.
  const peek = await request.get(`/members/${fx.member_id}/waiting`, { headers: bearer(fx.second_token) });
  expect([403, 404]).toContain(peek.status());
  const peekBlock = await request.get(`/threads/${fx.hold_thread_id}/block`, { headers: bearer(fx.second_token) });
  expect([403, 404]).toContain(peekBlock.status());
  const clear = await request.delete(`/threads/${fx.hold_thread_id}/block`, { headers: bearer(fx.second_token) });
  expect([403, 404]).toContain(clear.status());
  expect(await holdBlockStatus(request)).toBe(200);
  // Nor the other way round.
  const back = await request.delete(`/threads/${fx.second_thread_id}/block`, { headers: bearer(fx.admin_token) });
  expect([403, 404]).toContain(back.status());
  const secondBlock = await request.get(`/threads/${fx.second_thread_id}/block`, { headers: bearer(fx.second_token) });
  expect(secondBlock.status()).toBe(200);

  // On the page: the stranger's Needs you shows its own blocked task and
  // nothing from the first workspace.
  await signIn(page, fx.second_workspace_id, fx.second_token);
  const own = row(page, fx.second_thread_id);
  await expect(own).toBeVisible();
  await expect(own.locator(".ny-kind")).toHaveText("Blocked");
  await expect(own.locator(".ny-title")).toHaveText(SECOND_TITLE);
  await expect(own.locator(".ny-sub")).toContainText("blocked (human): only the second workspace may see this");
  await expect(own.getByRole("button", { name: "Unblock" })).toBeVisible();
  await expect(row(page, fx.hold_thread_id)).toHaveCount(0);
  await expect(page.locator("#needs-you-list")).not.toContainText(HOLD_TITLE);
  await expect(page.locator("#needs-you-list")).not.toContainText(NOTE);

  // And the operator's page shows its own block and not the stranger's.
  await page.context().clearCookies();
  await signIn(page, fx.workspace_id, fx.review_token);
  await expect(row(page, fx.hold_thread_id)).toBeVisible();
  await expect(row(page, fx.second_thread_id)).toHaveCount(0);
  await expect(page.locator("#needs-you-list")).not.toContainText(SECOND_TITLE);
});
