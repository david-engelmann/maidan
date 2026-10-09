import { test, expect, type Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

// Two rules from docs/UI Design.md that the screenshot capture (#1353) found
// broken: "Could not load" is a sentence plus Try again, not a red panel; and
// a screen has one filled button, so an open thread whose actions draw
// Approve has Post as a ghost and no second filled Approve in Needs you.

// Wait for start()'s session read, sign in by leaving the token field, and
// open the channel only after that sign-in's channel response (the pattern
// board.spec.ts uses, so a busy run does not race two channel loads).
async function openChannel(page: Page, token: string, channelId: string) {
  const session = page.waitForResponse(
    (r) => r.request().method() === "GET" && r.url().endsWith("/auth/session"),
  );
  await page.goto("/ui/");
  await session;
  await page.fill("#workspace", fx.workspace_id);
  const channels = page.waitForResponse(
    (r) =>
      r.request().method() === "GET" &&
      r.url().includes(`/workspaces/${fx.workspace_id}/channels`) &&
      r.ok(),
  );
  await page.fill("#token", token);
  await page.locator("#token").blur();
  await channels;
  await page.click(`#channel-list li[data-id="${channelId}"]`);
}

test("a board that could not load is a sentence and Try again, not a red panel", async ({ page }) => {
  await page.route(`**/channels/${fx.board_channel_id}/threads**`, (route) => route.abort("connectionrefused"));
  await openChannel(page, fx.token, fx.board_channel_id);
  const panel = page.locator("#board .onboard.board-error");
  await expect(panel).toBeVisible();
  await expect(panel.locator("button.primary")).toHaveText("Try again");
  const look = await panel.evaluate((el) => {
    const s = getComputedStyle(el);
    return { background: s.backgroundColor, border: s.borderTopColor };
  });
  // The panel is the plain onboard box: white, with its neutral border.
  expect(look.background).toBe("rgb(255, 255, 255)");
  expect(look.border).not.toBe("rgb(254, 202, 202)");
  expect(look.border).toBe("rgb(203, 213, 225)");
  // The error is the word: the heading keeps --err.
  const colors = await panel.locator("h3").evaluate((el) => {
    const probe = document.createElement("span");
    probe.style.color = "var(--err)";
    document.body.appendChild(probe);
    const err = getComputedStyle(probe).color;
    probe.remove();
    return { heading: getComputedStyle(el).color, err };
  });
  expect(colors.heading).toBe(colors.err);
});

test("an open thread under review has one filled Approve: the thread's", async ({ page }) => {
  await openChannel(page, fx.review_token, fx.desk_channel_id);
  const row = page.locator(`#needs-you-list .ny-item[data-thread-id="${fx.desk_waiting_thread_id}"]`);
  const rowApprove = row.locator(".ny-actions button", { hasText: /^Approve$/ });
  // Before the card is open, the row's Approve is the filled one.
  await expect(rowApprove).toHaveClass(/\bprimary\b/);

  await page.locator(`#board .card[data-id="${fx.desk_waiting_thread_id}"]`).click();
  const threadApprove = page.locator("#thread-actions button", { hasText: "Approve" });
  await expect(threadApprove).toHaveClass(/\bprimary\b/);
  // The thread's Approve is the decision: Post and the same task's row step back.
  await expect(page.locator("#post-message")).not.toHaveClass(/\bprimary\b/);
  await expect(page.locator("#post-message")).toHaveClass(/\bghost\b/);
  await expect(rowApprove).not.toHaveClass(/\bprimary\b/);
  await expect(rowApprove).toHaveClass(/\bghost\b/);
  // Another task's row is not this thread's decision and keeps its own.
  const other = page.locator(`#needs-you-list .ny-item[data-thread-id="${fx.desk_send_back_thread_id}"] .ny-actions button`, {
    hasText: /^Approve$/,
  });
  await expect(other).toHaveClass(/\bprimary\b/);

  // A thread with no decision for me gives Post back its fill, and the
  // first task's row gets its Approve back.
  await page.locator(`#board .card[data-id="${fx.desk_send_back_thread_id}"]`).click();
  await expect(page.locator("#thread-actions button", { hasText: "Approve" })).toHaveClass(/\bprimary\b/);
  await expect(rowApprove).toHaveClass(/\bprimary\b/);
  await expect(other).toHaveClass(/\bghost\b/);
});

test("Post is filled when the open thread has no decision for me", async ({ page }) => {
  await openChannel(page, fx.token, fx.board_channel_id);
  await page.locator(`#board .card[data-id="${fx.board_open_thread_id}"]`).click();
  await expect(page.locator("#collab-panel")).toBeVisible();
  await expect(page.locator("#thread-actions button")).toHaveCount(0);
  await expect(page.locator("#post-message")).toHaveClass(/\bprimary\b/);
  await expect(page.locator("#post-message")).not.toHaveClass(/\bghost\b/);
});
