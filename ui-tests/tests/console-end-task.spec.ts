import { test, expect, type APIRequestContext } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Ending a task early from the console: Cancel from the task sheet's More
// actions menu, Close as abandoned from the full thread view. Each test makes
// its own channel and task, so no seeded row changes under another spec.
const fx = fixtures();
const bearer = { Authorization: `Bearer ${fx.token}` };

async function freshTask(request: APIRequestContext, title: string): Promise<{ channel: string; task: string }> {
  const ch = await request.post(`/workspaces/${fx.workspace_id}/channels`, { headers: bearer, data: { name: `end-${Date.now()}-${Math.floor(Math.random() * 1e6)}` } });
  expect(ch.ok(), await ch.text()).toBeTruthy();
  const channel = (await ch.json()).id as string;
  const th = await request.post(`/channels/${channel}/threads`, { headers: bearer, data: { title } });
  expect(th.ok(), await th.text()).toBeTruthy();
  return { channel, task: (await th.json()).id as string };
}

async function stateOf(request: APIRequestContext, task: string): Promise<string> {
  const res = await request.get(`/threads/${task}`, { headers: bearer });
  expect(res.ok(), await res.text()).toBeTruthy();
  return (await res.json()).state as string;
}

test("cancelling from the task sheet asks why, says what it does, and closes the task", async ({ page, request }) => {
  const { channel, task } = await freshTask(request, "Cancel me from the sheet");
  await signInWithCookie(page, fx);
  await openOnChannel(page, channel);
  await page.locator("button.task", { hasText: "Cancel me from the sheet" }).click();
  const sheet = page.getByRole("dialog");
  await expect(sheet).toBeVisible();

  await sheet.getByRole("button", { name: "More actions" }).click();
  await sheet.getByRole("menuitem", { name: "Cancel task…" }).click();
  const why = sheet.getByRole("textbox", { name: /Why cancel it/ });
  await expect(why).toBeFocused();
  await why.fill("Nobody needs this any more");
  await sheet.getByRole("button", { name: "Continue" }).click();

  await expect(sheet.getByRole("heading", { name: "Cancel “Cancel me from the sheet”?" })).toBeFocused();
  await expect(sheet.locator(".end-effects")).toContainText("The claim ends at once");
  expect(await stateOf(request, task)).toBe("open");

  await sheet.getByRole("button", { name: "Cancel task", exact: true }).click();
  await expect(sheet.locator(".sheet-note")).toHaveText("Cancelled. The task is closed.");
  expect(await stateOf(request, task)).toBe("closed");
});

test("closing as abandoned from the thread view closes the task and the buttons go away", async ({ page, request }) => {
  const { channel, task } = await freshTask(request, "Abandon me from the thread");
  await signInWithCookie(page, fx);
  await openOnChannel(page, channel, `#/threads/t/${task}`);
  const row = page.getByRole("group", { name: "End this task" });
  await expect(row.getByRole("button", { name: "Cancel task…" })).toBeVisible();
  await row.getByRole("button", { name: "Close as abandoned…" }).click();

  const form = page.locator(".end-task");
  await expect(form.getByRole("radio", { name: "Nobody will do it" })).toBeChecked();
  const r = await new AxeBuilder({ page }).include(".end-task").withTags(["wcag2a", "wcag2aa"]).analyze();
  expect(r.violations.filter((v) => v.impact === "serious" || v.impact === "critical").map((v) => v.id)).toEqual([]);

  await form.getByRole("button", { name: "Continue" }).click();
  await expect(form.locator(".end-effects")).toContainText("keep waiting");
  await form.getByRole("button", { name: "Close task" }).click();

  await expect(page.locator(".toasts .toast").filter({ hasText: "Closed. Nothing landed." })).toBeVisible();
  await expect(page.locator(".end-task, .task-end")).toHaveCount(0);
  expect(await stateOf(request, task)).toBe("closed");
});
