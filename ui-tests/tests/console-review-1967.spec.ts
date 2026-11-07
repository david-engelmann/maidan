import { test, expect, type APIRequestContext } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openOnChannel, signInWithCookie } from "./_console";

// Review follow-ups: a DM thread link opens the conversation, an attached
// file lands on the thread, a dependency is a task name, and a card past
// the first page of a lane still opens.
const fx = fixtures();
const auth = { Authorization: `Bearer ${fx.admin_token}` };

async function openDm(request: APIRequestContext): Promise<{ id: string; thread_id: string }> {
  const created = await request.post(`/workspaces/${fx.workspace_id}/dm`, {
    headers: auth,
    data: { other_member_id: fx.rae_member_id },
  });
  if (created.status() === 201) return created.json();
  const list = await request.get(`/workspaces/${fx.workspace_id}/dm?member_id=${fx.member_id}`, { headers: auth });
  expect(list.ok()).toBeTruthy();
  const rows = (await list.json()) as Array<{ id: string; thread_id: string; member_low_id: string; member_high_id: string }>;
  const found = rows.find((d) => d.member_low_id === fx.rae_member_id || d.member_high_id === fx.rae_member_id);
  expect(found, "the direct conversation exists").toBeTruthy();
  return found!;
}

test("a thread link for a direct conversation opens under Direct", async ({ page, request }) => {
  const dm = await openDm(request);
  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${dm.thread_id}`);
  await expect(page).toHaveURL(new RegExp(`#/threads/dm/${dm.id}$`));
  await expect(page.getByRole("tab", { name: "Direct" })).toHaveAttribute("aria-selected", "true");
  await expect(page.locator("details.facts-panel")).toHaveCount(0);
  await expect(page.locator(".conv-title h2")).toHaveText("Rae Reviewer");
});

test("attaching a file links it, and Depends on names the task", async ({ page, request }) => {
  const depTitle = `Earlier task ${Date.now()}`;
  const taskTitle = `Needs ${depTitle}`;
  const dep = await request.post(`/channels/${fx.channel_id}/threads`, { headers: auth, data: { title: depTitle } });
  const task = await request.post(`/channels/${fx.channel_id}/threads`, { headers: auth, data: { title: taskTitle } });
  expect(dep.status()).toBe(201);
  expect(task.status()).toBe(201);
  const depId = (await dep.json()).id as string;
  const taskId = (await task.json()).id as string;
  const edge = await request.post(`/threads/${taskId}/dependencies`, { headers: auth, data: { depends_on_thread_id: depId } });
  expect(edge.status()).toBe(204);

  await signInWithCookie(page, fx);
  await openOnChannel(page, fx.channel_id, `#/threads/t/${taskId}`);
  await expect(page.locator(".conv-title h2")).toHaveText(taskTitle);
  await page.getByText("Result and evidence").click();
  const link = page.locator(".facts-panel a", { hasText: depTitle });
  await expect(link).toHaveAttribute("href", `#/threads/t/${depId}`);
  await expect(page.locator(".facts-panel")).not.toContainText(depId);

  const refreshed = page.waitForResponse((r) => r.url().includes(`/threads/${taskId}/evidence`) && r.request().method() === "GET");
  const uploaded = page.waitForResponse((r) => r.url().includes("/artifacts?") && r.request().method() === "POST");
  const linked = page.waitForResponse((r) => /\/threads\/[^/]+\/artifacts\//.test(r.url()) && r.request().method() === "PUT");
  await page.locator('.composer-row input[type="file"]').setInputFiles({
    name: "evidence.txt",
    mimeType: "text/plain",
    buffer: Buffer.from("review notes"),
  });
  const up = await uploaded;
  expect(up.status()).toBe(201);
  const artifact = (await up.json()) as { sha256: string; filename?: string };
  expect(artifact.filename).toBe("evidence.txt");
  expect((await linked).ok()).toBeTruthy();
  await refreshed;
  await expect(page.locator(".composer-note")).toHaveText("Attached evidence.txt");
  const list = await request.get(`/threads/${taskId}/artifacts`, { headers: auth });
  expect(list.ok()).toBeTruthy();
  expect(JSON.stringify(await list.json())).toContain(artifact.sha256);
});

test("a card past the first page of a lane opens from its link", async ({ page, request }) => {
  test.setTimeout(90_000);
  const channel = await request.post(`/workspaces/${fx.workspace_id}/channels`, {
    headers: auth,
    data: { name: `long-lane-${Date.now()}` },
  });
  expect(channel.status()).toBe(201);
  const channelId = (await channel.json()).id as string;
  const titles: string[] = [];
  for (let i = 0; i < 66; i += 1) {
    const title = `Lane card ${String(i).padStart(2, "0")}`;
    titles.push(title);
    const res = await request.post(`/channels/${channelId}/threads`, { headers: auth, data: { title } });
    expect(res.status(), title).toBe(201);
  }
  const listed = (await (await request.get(`/channels/${channelId}/threads?limit=500`, { headers: auth })).json()) as Array<{ id: string; title?: string; updated_at?: string }>;
  listed.sort((a, b) => (Date.parse(b.updated_at ?? "") || 0) - (Date.parse(a.updated_at ?? "") || 0) || String(a.id).localeCompare(String(b.id)));
  const target = listed[64];
  expect(target, "the 65th card").toBeTruthy();

  await signInWithCookie(page, fx);
  await openOnChannel(page, channelId, `#/board?channel=${encodeURIComponent(channelId)}&task=${encodeURIComponent(target.id)}`);
  const card = page.locator("button.task.selected", { hasText: target.title ?? "" });
  await expect(card).toHaveCount(1);
  await card.scrollIntoViewIfNeeded();
  await expect(card).toBeVisible();
  await expect(card).toHaveAttribute("aria-current", "true");
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, isMobile: true, hasTouch: true });

  test("Attach is at least 44px and the thread has no serious axe violations", async ({ page }) => {
    await signInWithCookie(page, fx);
    await openOnChannel(page, fx.channel_id, `#/threads/t/${fx.thread_id}`);
    const attach = page.locator("label.chip-btn", { hasText: "Attach" });
    await expect(attach).toBeVisible();
    const box = await attach.boundingBox();
    expect(box?.height ?? 0).toBeGreaterThanOrEqual(44);
    const wide = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth + 1);
    expect(wide).toBe(false);
    const result = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    const bad = result.violations.filter((v) => v.impact === "serious" || v.impact === "critical");
    expect(bad.map((v) => `${v.id}: ${v.nodes.length}`)).toEqual([]);
  });
});
