import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { callUiExport, signIn } from "./_tools";

const fx = fixtures();

// An agent that asks a question waits on the task's owner in Needs you, under
// its own heading. Answer opens the thread at the composer, and the reply takes
// the question off the queue.

const QUESTION = "Which region should the replica run in?";
const TITLE = "Asked: where the replica runs";

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });

// The deployer holds the task, so it may declare its status.
async function ask(request: APIRequestContext) {
  const res = await request.put(`/threads/${fx.ask_thread_id}/status`, {
    headers: bearer(fx.requester_token),
    data: { status: "needs_input", note: QUESTION },
  });
  expect(res.ok()).toBeTruthy();
}

test("an agent's question waits on the owner, and answering in the thread clears it", async ({ page, request }) => {
  await ask(request);
  await signIn(page, fx.workspace_id, fx.review_token);

  const asked = row(page, fx.ask_thread_id);
  await expect(asked).toBeVisible();
  await expect(asked).toHaveAttribute("data-kind", "question");
  await expect(asked.locator(".ny-kind")).toHaveText("Question");
  await expect(asked.locator(".ny-title")).toHaveText(TITLE);
  await expect(asked.locator(".ny-question")).toHaveText(QUESTION);
  // Reviews and gates also wait on the operator, so the queue is split, and
  // the question sits under its own heading after the decisions.
  const heads = page.locator("#needs-you-list .ny-group");
  await expect(heads.first()).toHaveText("Needs your decision");
  await expect(heads.last()).toHaveText("An agent asked");

  await asked.getByRole("button", { name: "Answer" }).click();
  await expect(page.locator("#compose-body")).toBeFocused();
  await page.locator("#compose-body").fill("us-east-1, beside the primary.");
  await page.locator("#post-message").click();
  await expect(page.locator("#compose-body")).toHaveValue("");

  const status = await request.get(`/threads/${fx.ask_thread_id}/status`, {
    headers: bearer(fx.requester_token),
  });
  expect(status.status()).toBe(404);
  await callUiExport(page, "needs.js", "loadNeedsYou");
  await expect(asked).toHaveCount(0);
});
