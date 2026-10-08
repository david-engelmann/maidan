import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport } from "./_tools";

const fx = fixtures();

// These tests approve and close seeded tasks on the shared server, so a retry
// would find its row already gone and fail for the wrong reason. A failure
// here fails once, with its real cause.
test.describe.configure({ retries: 0 });

async function reloadQueue(page: Page) {
  const done = page.waitForResponse((r) => r.url().includes("/waiting"));
  await callUiExport(page, "needs.js", "loadNeedsYou");
  await done;
  await page.waitForTimeout(100);
}

async function signIn(page: Page, token: string) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", token);
  await page.locator("#token").dispatchEvent("change");
  await page.click("#refresh-channels");
  await page.click(`#channel-list li[data-id="${fx.desk_channel_id}"]`);
}

async function threadState(page: Page, id: string): Promise<string> {
  const res = await page.request.get(`${fx.base_url}/threads/${id}`, {
    headers: { Authorization: `Bearer ${fx.review_token}` },
  });
  expect(res.ok()).toBeTruthy();
  return (await res.json()).state;
}

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

// The reviews agents asked this member for are the first thing on the page,
// each naming who handed the work off and what they reported, with the
// decision buttons in the row. The waiting count reaches the tab title.
test("needs you lists the reviews waiting on me, with who and what", async ({ page }) => {
  await signIn(page, fx.review_token);

  const box = page.locator("#needs-you");
  await expect(box).toBeVisible();
  await expect(box.locator(".ny-head h2")).toHaveText("Needs you");
  const approve = row(page, fx.desk_approve_thread_id);
  await expect(approve.locator(".ny-kind")).toHaveText("Review");
  await expect(approve.locator(".ny-title")).toHaveText("Approve me: the flaky login test is fixed");
  await expect(approve.locator(".ny-sub .person .name")).toHaveText("Deployer");
  const failures = approve.locator(".result .kv", { hasText: "failures" });
  await expect(failures.locator(".v")).toHaveText("0");
  await expect(failures.locator(".v")).toHaveClass(/good/);
  await expect(approve.locator(".result .kv", { hasText: "runs" }).locator(".v")).toHaveText("500");
  await expect(approve.getByRole("button", { name: "Approve", exact: true })).toBeVisible();
  await expect(approve.getByRole("button", { name: "Request changes" })).toBeVisible();

  const count = Number(await page.locator("#needs-you-count").textContent());
  expect(count).toBeGreaterThanOrEqual(2);
  await expect(page).toHaveTitle(`(${count}) Maidan`);
  await expect(page.locator("#favicon")).toHaveAttribute("href", /^data:image\/svg\+xml,/);
});

// Approve in the row with a note, then close once the requirement is met: the
// task is done on the server, the note is on the review, and the row leaves
// the queue.
test("approving from needs you meets the review, and close finishes the task", async ({ page }) => {
  await signIn(page, fx.review_token);
  const approve = row(page, fx.desk_approve_thread_id);
  await approve.getByRole("button", { name: "Approve with note" }).click();
  await approve.locator(".ny-note input").fill("Nice handling of the empty list.");
  await approve.locator(".ny-note").getByRole("button", { name: "Approve", exact: true }).click();
  await expect(approve.locator(".done-note")).toHaveText("Approved ✓ 1/1");
  const reviews = await page.request.get(`${fx.base_url}/threads/${fx.desk_approve_thread_id}/reviews`, {
    headers: { Authorization: `Bearer ${fx.review_token}` },
  });
  expect((await reviews.json()).some((r: { decision: string; note: string }) =>
    r.decision === "approve" && r.note === "Nice handling of the empty list.")).toBeTruthy();
  // A live update reloads the queue; the approved row keeps its Close task.
  await reloadQueue(page);
  await expect(approve.getByRole("button", { name: "Close task" })).toBeVisible();
  await approve.getByRole("button", { name: "Close task" }).click();
  await expect(approve).toHaveCount(0);
  expect(await threadState(page, fx.desk_approve_thread_id)).toBe("closed");
  await expect(
    page.locator(`#board .board-col[data-column="done"] .card[data-id="${fx.desk_approve_thread_id}"]`),
  ).toBeVisible();
});

// A change request carries a note for the agent and sends the task back to
// open, so it is claimable again.
test("request changes sends the task back with a note", async ({ page }) => {
  await signIn(page, fx.review_token);
  const back = row(page, fx.desk_send_back_thread_id);
  await back.getByRole("button", { name: "Request changes" }).click();
  await back.locator(".ny-note input").fill("Make the limit configurable per workspace.");
  // A reload while the note is being written keeps what was typed.
  await reloadQueue(page);
  await expect(back.locator(".ny-note input")).toHaveValue("Make the limit configurable per workspace.");
  await back.getByRole("button", { name: "Send back" }).click();
  await expect(back).toHaveCount(0);
  expect(await threadState(page, fx.desk_send_back_thread_id)).toBe("open");

  const reviews = await page.request.get(`${fx.base_url}/threads/${fx.desk_send_back_thread_id}/reviews`, {
    headers: { Authorization: `Bearer ${fx.review_token}` },
  });
  const list = await reviews.json();
  expect(list.some((r: { decision: string; note: string }) =>
    r.decision === "request_changes" && r.note === "Make the limit configurable per workspace.")).toBeTruthy();
});

// A token that cannot transition threads is told what it lacks and how to
// fix it, in words, not with a bare status code.
test("approving without thread:transition says what is missing and how to fix it", async ({ page }) => {
  await signIn(page, fx.token);
  const waiting = row(page, fx.desk_waiting_thread_id);
  await waiting.getByRole("button", { name: "Approve", exact: true }).click();
  const err = waiting.locator(".ny-err");
  await expect(err).toContainText("Review not recorded: Your token is not allowed to do this; it needs thread:transition");
  await expect(err).toContainText("Mint a token with it in Tokens");
  expect(await threadState(page, fx.desk_waiting_thread_id)).toBe("in_review");
});

// A dropped connection is reported in the row and the buttons come back, so
// the reviewer can retry without reloading the page.
test("a network failure while approving says so and leaves the row usable", async ({ page }) => {
  await signIn(page, fx.token);
  await page.route(`**/threads/${fx.desk_waiting_thread_id}/reviews`, (route) =>
    route.request().method() === "POST" ? route.abort("internetdisconnected") : route.continue(),
  );
  const waiting = row(page, fx.desk_waiting_thread_id);
  const approveBtn = waiting.getByRole("button", { name: "Approve", exact: true });
  await approveBtn.click();
  await expect(waiting.locator(".ny-err")).toContainText("Review not recorded: could not reach the server");
  await expect(approveBtn).toBeEnabled();
  expect(await threadState(page, fx.desk_waiting_thread_id)).toBe("in_review");
});

// On a first visit nothing has opened a channel, so the board has not loaded
// the gate views yet. A gate row still names who asked and answers the gate.
test("a gate in needs you names its requester and answers without a channel open", async ({ page, request }) => {
  const prompt = `Deploy the fix ${Date.now()}?`;
  const mcp = await request.post(`${fx.base_url}/mcp`, {
    headers: { Authorization: `Bearer ${fx.requester_token}`, "Content-Type": "application/json" },
    data: {
      jsonrpc: "2.0",
      id: 1,
      method: "tools/call",
      params: { name: "request_approval", arguments: { prompt, thread_id: fx.thread_id } },
    },
  });
  expect(mcp.ok()).toBeTruthy();

  // A returning visitor: signed in as a person (accepting a gate needs a
  // sign-in, or a token holding approval:grant), the workspace already
  // stored, and no channel. Seed that before the document runs, so the boot
  // reads it.
  await page.context().addCookies([
    { name: "maidan_session", value: fx.session_cookie, url: fx.base_url, httpOnly: true, sameSite: "Lax" },
  ]);
  await page.addInitScript((w) => {
    localStorage.setItem("maidan_workspace", w);
    localStorage.removeItem("maidan_channel");
  }, fx.workspace_id);
  await page.goto("/ui/");
  await expect(page.locator("#identity-pill")).toBeVisible();
  await expect(page.locator("#channel-list li[data-id]").first()).toBeVisible();
  await expect(page.locator("#channel-list li.selected")).toHaveCount(0);
  const gateRow = page.locator("#needs-you-list .ny-item").filter({ hasText: prompt });
  await expect(gateRow).toBeVisible();
  await expect(gateRow.locator(".ny-sub")).toContainText("asked by");
  await gateRow.getByRole("button", { name: "Approve", exact: true }).click();
  await expect(gateRow).toHaveCount(0);
});
