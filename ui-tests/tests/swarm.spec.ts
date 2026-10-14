import { test, expect } from "@playwright/test";
import AxeBuilder from "@axe-core/playwright";
import { fixtures } from "./_fixtures";
import { openMoreTools, signIn } from "./_tools";

// The swarm panel: one workspace's live claims with their footprints, the
// threads waiting and why, slot usage, and paused repositories. The state
// is seeded through the public API, the same calls a worker's tools make.
const fx = fixtures();
const suffix = Math.random().toString(36).slice(2, 8);
const repo = `acme/swarm-${suffix}`;

test("the swarm panel shows claims, waiting threads, slots and pauses for the workspace", async ({
  page,
  request,
}) => {
  const bearer = { Authorization: `Bearer ${fx.requester_token}` };
  const admin = { Authorization: `Bearer ${fx.admin_token}` };

  const channel = await request.post(`${fx.base_url}/workspaces/${fx.workspace_id}/channels`, {
    headers: admin,
    data: { name: `swarm-${suffix}`, topic: "swarm panel spec" },
  });
  expect(channel.ok()).toBeTruthy();
  const channelId = (await channel.json()).id;

  const mkThread = async (title: string) => {
    const res = await request.post(`${fx.base_url}/channels/${channelId}/threads`, {
      headers: admin,
      data: { title },
    });
    expect(res.ok()).toBeTruthy();
    return (await res.json()).id as string;
  };
  const holderThread = await mkThread("Swarm holder task");
  const waitingThread = await mkThread("Swarm waiting task");

  // The deployer claims the holder; the queue hands out the oldest first.
  const claim = await request.post(`${fx.base_url}/channels/${channelId}/claim-next`, {
    headers: bearer,
    data: { lease_secs: 900 },
  });
  expect(claim.ok()).toBeTruthy();
  expect((await claim.json()).id).toBe(holderThread);

  const footprint = (items: unknown[]) => ({ repo, mode: "exclusive", items });
  const holderPrint = await request.put(`${fx.base_url}/threads/${holderThread}/footprint`, {
    headers: bearer,
    data: footprint([
      { kind: "glob", value: "src/**" },
      { kind: "slot", value: "ci" },
    ]),
  });
  expect(holderPrint.ok()).toBeTruthy();
  // The waiting thread wants the same paths, so the live claim defers it.
  const waitingPrint = await request.put(`${fx.base_url}/threads/${waitingThread}/footprint`, {
    headers: bearer,
    data: footprint([{ kind: "glob", value: "src/**" }]),
  });
  expect(waitingPrint.ok()).toBeTruthy();

  // One ci slot, and the holder is using it: the slot reads full.
  const slot = await request.put(`${fx.base_url}/workspaces/${fx.workspace_id}/slots`, {
    headers: admin,
    data: { slot: "ci", capacity: 1 },
  });
  expect(slot.ok()).toBeTruthy();

  const pausedRepo = `acme/frozen-${suffix}`;
  const pause = await request.put(`${fx.base_url}/workspaces/${fx.workspace_id}/repo-pauses`, {
    headers: admin,
    data: { repo: pausedRepo, reason: "release freeze" },
  });
  expect(pause.ok()).toBeTruthy();

  await signIn(page, fx.workspace_id, fx.token);
  await openMoreTools(page);
  await page.click('.tabs button[data-tab="swarm"]');

  const claims = page.locator("#swarm-claims");
  await expect(claims).toContainText(repo);
  await expect(claims).toContainText("src/**");
  await expect(claims).toContainText("Deployer");

  const deferred = page.locator("#swarm-deferred");
  await expect(deferred).toContainText("Swarm waiting task");
  await expect(deferred).toContainText("A live claim overlaps its footprint");

  const slots = page.locator("#swarm-slots");
  await expect(slots).toContainText("ci");
  await expect(slots).toContainText("Full.");

  const pauses = page.locator("#swarm-pauses");
  await expect(pauses).toContainText(pausedRepo);
  await expect(pauses).toContainText("release freeze");

  const axe = await new AxeBuilder({ page })
    .withTags(["wcag2a", "wcag2aa", "wcag21a", "wcag21aa"])
    .analyze();
  expect(
    axe.violations.map((v) => `${v.id} (${v.impact})`),
    "axe on the swarm panel",
  ).toEqual([]);

  // Resuming from the panel removes the pause.
  await pauses.locator("button[data-resume]").click();
  await expect(pauses).toContainText("No repositories are paused.");
});
