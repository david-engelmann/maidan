import { test, expect } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { openMoreTools } from "./_tools";

const fx = fixtures();

interface WaitingItem {
  kind: string;
  thread_id: string | null;
  summary: string;
}

// The waiting-on-you inbox in the Work tab lists what the server says is
// waiting on the member. The summary line prints "0 waiting" for an empty
// inbox too, so the check is the list itself: one row per item the server
// returns, including the fixture's review request that no spec answers
// ("Still waiting: the upload path", desk_waiting_thread_id). Other specs
// approve and answer seeded items, so the expected count is read from the
// server at test time rather than fixed.
test("the Work tab lists the items waiting on the member", async ({ page }) => {
  const res = await page.request.get(`${fx.base_url}/members/${fx.member_id}/waiting?sla_secs=86400`, {
    headers: { Authorization: `Bearer ${fx.token}` },
  });
  expect(res.ok()).toBeTruthy();
  const inbox: { total: number; items: WaitingItem[] } = await res.json();
  const review = inbox.items.find((it) => it.thread_id === fx.desk_waiting_thread_id);
  expect(review, "the seeded review request is waiting on the member").toBeDefined();
  expect(review!.kind).toBe("review_request");
  expect(inbox.total).toBe(inbox.items.length);

  await page.goto("/ui/");
  await page.fill("#workspace", fx.workspace_id);
  await page.fill("#token", fx.token);
  const identity = page.waitForResponse((response) => response.url().endsWith("/me"));
  await page.locator("#token").press("Tab");
  await identity;

  await openMoreTools(page);
  await page.click('.tabs button[data-tab="work"]');
  await page.click("#waiting-refresh");

  await expect(page.locator("#waiting-summary")).toContainText(`${inbox.total} waiting`);
  const rows = page.locator("#waiting-list li");
  await expect(rows).toHaveCount(inbox.total);
  await expect(rows.filter({ hasText: `[review_request] ${review!.summary}` })).toHaveCount(1);
});
