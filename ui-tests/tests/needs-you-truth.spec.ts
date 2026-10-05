import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport, signIn } from "./_tools";

const fx = fixtures();

// A failed Needs you load is never an empty or hidden queue. The rows and the
// count in the tab title stay as last seen; a refusal names the fix, and a
// dropped connection is marked stale and retries on its own.

const waitingRow = (page: Page) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${fx.desk_waiting_thread_id}"]`);

async function reloadQueue(page: Page) {
  await callUiExport(page, "needs.js", "loadNeedsYou");
}

// A throwaway token for the operator, so revoking it breaks no other spec.
async function mintThrowaway(request: APIRequestContext): Promise<{ id: string; secret: string }> {
  const res = await request.post(`/workspaces/${fx.workspace_id}/members/${fx.member_id}/tokens`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
    data: { label: "needs-you-truth-spec", capabilities: ["workspace:read"] },
  });
  expect(res.ok()).toBeTruthy();
  return res.json();
}

test("a revoked token is told what to fix, and the queue and tab count stay", async ({ page, request }) => {
  const throwaway = await mintThrowaway(request);
  await signIn(page, fx.workspace_id, throwaway.secret);
  await expect(waitingRow(page)).toBeVisible();
  const title = await page.title();
  expect(title).toMatch(/^\(\d+\) Maidan$/);

  const revoked = await request.delete(`/tokens/${throwaway.id}`, {
    headers: { Authorization: `Bearer ${fx.admin_token}` },
  });
  expect(revoked.ok()).toBeTruthy();
  await reloadQueue(page);

  const state = page.locator("#needs-you-state");
  await expect(state).toHaveText(
    "Could not load what is waiting on you: Your token or session was not accepted. Use Change to set a working one",
  );
  await expect(state).toHaveClass(/\berr\b/);
  await expect(page.locator("#needs-you")).toBeVisible();
  await expect(page.locator("#needs-you-quiet")).toBeHidden();
  await expect(waitingRow(page)).toBeVisible();
  await expect(page).toHaveTitle(title);
});

test("a dropped connection marks the queue stale, and the next good load clears it", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.token);
  await expect(waitingRow(page)).toBeVisible();
  const title = await page.title();
  const state = page.locator("#needs-you-state");
  await expect(state).toBeHidden();

  // A server error is a sentence, then the next good load clears it.
  await page.route("**/waiting*", (route) => route.fulfill({ status: 503, body: "" }));
  await reloadQueue(page);
  await expect(state).toHaveText(
    "Could not load what is waiting on you: The server hit an error. Try again; if it keeps failing, check the server log",
  );
  await expect(page).toHaveTitle(title);
  await page.unroute("**/waiting*");
  await reloadQueue(page);
  await expect(state).toBeHidden();

  // No answer at all: the rows stay under a stale line that says when they
  // were last true. The load retries by itself once the server answers again.
  await page.route("**/waiting*", (route) => route.abort("internetdisconnected"));
  await reloadQueue(page);
  await expect(state).toHaveText(/^Stale since \d{1,2}:\d{2}.*: could not reach the server\. Reconnecting…$/);
  await expect(state).not.toHaveClass(/\berr\b/);
  await expect(waitingRow(page)).toBeVisible();
  await expect(page).toHaveTitle(title);
  await page.unroute("**/waiting*");
  await expect(state).toBeHidden({ timeout: 15_000 });
  await expect(waitingRow(page)).toBeVisible();
  await expect(page).toHaveTitle(title);
});
