import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport } from "./_tools";

const fx = fixtures();

// Rebuilding the needs-you list detaches every row, even the one being kept,
// and a detached element loses focus. A keyboard user sitting on a decision
// button landed on <body> whenever the queue reloaded. These specs only move
// focus and reload: nothing is approved, so the shared seed is unchanged.

const waiting = (url: string) => /\/members\/[^/]+\/waiting(\?|$)/.test(url);

// Sign in by leaving the token field (no Refresh race), open the desk, and
// wait for the board's needs-you load before touching the rows.
async function openDesk(page: Page) {
  const session = page.waitForResponse(
    (r) => r.request().method() === "GET" && r.url().endsWith("/auth/session"),
  );
  await page.goto("/ui/");
  await session;
  await page.fill("#workspace", fx.workspace_id);
  const signedIn = page.waitForResponse((r) => r.request().method() === "GET" && waiting(r.url()) && r.ok());
  await page.fill("#token", fx.review_token);
  await page.locator("#token").blur();
  await (await signedIn).finished();
  await page.click(`#channel-list li[data-id="${fx.desk_channel_id}"]`);
  await expect(page.locator("#board-title")).toHaveText(/desk/);
  // The board's own reload, run to the end, so no render is still in flight.
  await callUiExport(page, "needs.js", "loadNeedsYou");
}

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

test("a needs-you reload keeps focus on the decision button it was on", async ({ page }) => {
  await openDesk(page);
  const changes = row(page, fx.desk_waiting_thread_id).getByRole("button", { name: "Request changes" });
  await changes.focus();
  await expect(changes).toBeFocused();
  const before = await changes.elementHandle();

  // A refresh, the path a realtime reconnect and the live-poll fallback take.
  await callUiExport(page, "needs.js", "loadNeedsYou");
  await expect(changes).toBeFocused();
  // The same control, not a look-alike in a rebuilt row.
  expect(await page.evaluate((el) => el === document.activeElement, before)).toBe(true);

  // And again: a second reload does not lose it either.
  await callUiExport(page, "needs.js", "loadNeedsYou");
  await expect(changes).toBeFocused();
});

test("when the focused item leaves the queue, focus moves to its neighbour and says so", async ({ page }) => {
  await openDesk(page);
  const leaving = row(page, fx.desk_waiting_thread_id);
  const keys = await page.locator("#needs-you-list .ny-item").evaluateAll((rows) =>
    rows.map((r) => (r as HTMLElement).dataset.key || ""),
  );
  const at = keys.findIndex((k) => k.includes(fx.desk_waiting_thread_id));
  expect(at).toBeGreaterThanOrEqual(0);
  expect(keys.length).toBeGreaterThan(1);
  const neighbourKey = keys[at + 1] ?? keys[at - 1];

  await leaving.getByRole("button", { name: "Request changes" }).focus();
  // The server no longer lists it, and the row is on its way out, as
  // dropRow marks it after an Unblock or a decision.
  await page.route(
    (url) => waiting(url.toString()),
    async (route) => {
      const res = await route.fetch();
      const body = await res.json();
      body.items = body.items.filter((i: { thread_id?: string }) => i.thread_id !== fx.desk_waiting_thread_id);
      await route.fulfill({ response: res, json: body });
    },
  );
  await leaving.evaluate((li) => li.classList.add("leaving"));
  await callUiExport(page, "needs.js", "loadNeedsYou");

  await expect(leaving).toHaveCount(0);
  const neighbour = page.locator(`#needs-you-list .ny-item[data-key="${neighbourKey}"]`);
  const focused = page.locator("#needs-you-list .ny-item :focus");
  await expect(focused).toHaveCount(1);
  expect(await neighbour.evaluate((li) => li.contains(document.activeElement))).toBe(true);
  await expect(page.locator("#toasts")).toContainText("Focus moved to the next one");
});

// A row marked leaving can be redrawn before it is removed, while the server
// still lists its item. The redrawn review row starts with its approve
// buttons disabled until its packet loads, and the focused control (here a
// Close task button, which only the old row had) has no twin. Focus must land
// on an enabled control of that row, not on a disabled one, which would leave
// it on <body>.
test("a redrawn row never hands focus to a disabled control", async ({ page }) => {
  await openDesk(page);
  const target = row(page, fx.desk_waiting_thread_id);
  // Keep every redrawn row's packet loading, so its approve buttons stay off.
  await page.route(/\/threads\/[^/]+\/review-packet$/, () => new Promise(() => {}));
  // The control that only the old row has, in the slot where the redrawn
  // row's disabled Approve sits.
  await target.getByRole("button", { name: "Approve", exact: true }).evaluate((b: HTMLButtonElement) => {
    b.disabled = false;
    b.textContent = "Close task";
    b.focus();
  });
  await target.evaluate((li) => li.classList.add("leaving"));
  await callUiExport(page, "needs.js", "loadNeedsYou");

  const redrawn = row(page, fx.desk_waiting_thread_id);
  await expect(redrawn.getByRole("button", { name: "Approve", exact: true })).toBeDisabled();
  expect(await redrawn.evaluate((li) => li.contains(document.activeElement))).toBe(true);
  expect(await page.evaluate(() => (document.activeElement as HTMLButtonElement).disabled === true)).toBe(false);
});
