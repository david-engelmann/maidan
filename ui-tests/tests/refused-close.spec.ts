import { test, expect, Page } from "@playwright/test";
import { fixtures } from "./_fixtures";

const fx = fixtures();

async function openDesk(page: Page) {
  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.review_token);
  await page.locator("#token").dispatchEvent("change");
  await page.click(`#channel-list li[data-id="${fx.desk_channel_id}"]`);
}

// The agent's close is refused by the server. A person watching the board
// sees that sentence, not a toast we made up.
test("a refused close is visible on the board", async ({ page, request }) => {
  const res = await request.post(`${fx.base_url}/threads/${fx.desk_waiting_thread_id}`, {
    headers: { Authorization: `Bearer ${fx.review_token}`, "Content-Type": "application/json" },
    data: { action: "close" },
  });
  expect(res.status()).toBe(409);
  const problem = await res.json();
  expect(problem.detail).toContain("review requirement not met");

  const messages = await request.get(`${fx.base_url}/threads/${fx.desk_waiting_thread_id}/messages?limit=20`, {
    headers: { Authorization: `Bearer ${fx.review_token}` },
  });
  expect(messages.ok()).toBeTruthy();
  const list = await messages.json();
  const note = list.find((m: { metadata?: { notice?: string }; body: string }) => m.metadata?.notice === "transition_refused");
  expect(note.body).toContain("review requirement not met");
  expect(note.body).toContain("Next:");

  await openDesk(page);
  const banner = page.locator("#board-refusal");
  await expect(banner).toBeVisible();
  await expect(banner.locator(".refusal-text")).toContainText("review requirement not met");
  await expect(banner.locator(".refusal-text")).toContainText(note.body.slice(0, 40));
  await expect(
    page.locator(`#board .card[data-id="${fx.desk_waiting_thread_id}"] .card-refusal`),
  ).toHaveText("Close refused");
  const after = await (await request.get(`${fx.base_url}/threads/${fx.desk_waiting_thread_id}`, {
    headers: { Authorization: `Bearer ${fx.review_token}` },
  })).json();
  expect(after.state).toBe("in_review");
});
