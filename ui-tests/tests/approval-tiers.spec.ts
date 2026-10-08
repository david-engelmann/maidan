import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

// Each item on a review row's evidence card ends in the attestation tier the
// server recorded at the hand-off: verified (a land-gate pass the close gate
// accepts), attached (linked by a member who never worked the task) or
// self-reported (a worker's own result or link). When the server says every
// item is self-reported, the card warns. The page draws what the packet says
// and decides nothing itself.

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

async function packet(request: APIRequestContext, threadId: string, token = fx.review_token) {
  const res = await request.get(`/threads/${threadId}/review-packet`, { headers: bearer(token) });
  expect(res.ok()).toBeTruthy();
  return res.json();
}

const WARNING =
  "Self-reported only: all of this evidence comes from the task's own workers. Nothing was attached by anyone else or verified by a land gate.";

test("a link from someone who never worked the task is attached, and the card does not warn", async ({ page, request }) => {
  const handed = await packet(request, fx.tiers_attached_thread_id);
  expect(handed.self_reported_only).toBe(false);
  expect(handed.manifest.attestations.map((a: { tier: string }) => a.tier)).toEqual(["self_reported", "attached"]);
  await signIn(page, fx.workspace_id, fx.review_token);

  const evidence = row(page, fx.tiers_attached_thread_id).locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  const result = evidence.locator('.ny-ev-item[data-ev="result"] .ny-ev-tier');
  await expect(result).toHaveText("self-reported");
  await expect(result).toHaveAttribute("data-tier", "self_reported");
  const shot = evidence.locator(`.ny-ev-item[data-sha="${fx.proof_screenshot_sha}"] .ny-ev-tier`);
  await expect(shot).toHaveText("attached");
  await expect(shot).toHaveAttribute("data-tier", "attached");
  await expect(shot).toHaveAttribute("title", "Linked by a member who never worked the task");
  await expect(evidence.locator(".ny-ev-warn")).toHaveCount(0);
});

test("a land-gate pass at the hand-off is its own verified line, with its recorder", async ({ page, request }) => {
  const handed = await packet(request, fx.tiers_verified_thread_id);
  expect(handed.self_reported_only).toBe(false);
  await signIn(page, fx.workspace_id, fx.review_token);

  const evidence = row(page, fx.tiers_verified_thread_id).locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  await expect(evidence.locator(".ny-ev-root")).toHaveAttribute("data-root", handed.evidence_root);
  const gate = evidence.locator('.ny-ev-item[data-ev="land_gate"]');
  await expect(gate).toHaveCount(1);
  await expect(gate.locator(".ny-ev-kind")).toHaveText("land-gate pass");
  await expect(gate.locator(".ny-ev-hash")).toHaveAttribute("title", fx.proof_transcript_sha);
  await expect(gate.locator(".person .name")).toHaveText("Verifier");
  await expect(gate.locator(".ny-ev-tier")).toHaveText("verified");
  await expect(gate.locator(".ny-ev-tier")).toHaveAttribute("data-tier", "verified");
  await expect(evidence.locator('.ny-ev-item[data-ev="result"] .ny-ev-tier')).toHaveText("self-reported");
  await expect(evidence.locator(".ny-ev-warn")).toHaveCount(0);
  // The pass is the last item, after the result.
  expect(
    await evidence.locator(".ny-ev-item").evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.ev)),
  ).toEqual(["result", "land_gate"]);
});

test("only the worker's own account: every item self-reported, and the card warns", async ({ page, request }) => {
  const handed = await packet(request, fx.tiers_self_thread_id);
  expect(handed.self_reported_only).toBe(true);
  await signIn(page, fx.workspace_id, fx.review_token);

  const evidence = row(page, fx.tiers_self_thread_id).locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  const warn = evidence.locator(".ny-ev-warn");
  await expect(warn).toHaveText(WARNING);
  await expect(warn).toHaveAttribute("role", "note");
  const tiers = evidence.locator(".ny-ev-tier");
  await expect(tiers).toHaveCount(2);
  expect(await tiers.evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.tier))).toEqual([
    "self_reported",
    "self_reported",
  ]);
  await expect(row(page, fx.tiers_self_thread_id).getByRole("button", { name: "Approve", exact: true })).toBeEnabled();
});

test("the warning follows the server's flag, not the page's own count", async ({ page }) => {
  // The same packet, with the server's verdict flipped both ways: the page
  // draws the flag it is given and never derives one from the tiers.
  const flip = async (tid: string, flag: boolean) =>
    page.route(`**/threads/${tid}/review-packet`, async (route) => {
      const res = await route.fetch();
      const body = await res.json();
      route.fulfill({ response: res, json: { ...body, self_reported_only: flag } });
    });
  await flip(fx.tiers_self_thread_id, false);
  await flip(fx.tiers_attached_thread_id, true);
  await signIn(page, fx.workspace_id, fx.review_token);
  const own = row(page, fx.tiers_self_thread_id).locator(".ny-evidence");
  await expect(own).toHaveAttribute("data-state", "ready");
  await expect(own.locator(".ny-ev-tier")).toHaveCount(2);
  await expect(own.locator(".ny-ev-warn")).toHaveCount(0);
  await expect(row(page, fx.tiers_attached_thread_id).locator(".ny-evidence .ny-ev-warn")).toHaveText(WARNING);
});

test("a packet from before tiers draws no tier and no warning", async ({ page }) => {
  await page.route(`**/threads/${fx.tiers_self_thread_id}/review-packet`, async (route) => {
    const res = await route.fetch();
    const body = await res.json();
    const { attestations: _dropped, ...manifest } = body.manifest;
    const { self_reported_only: _flag, ...rest } = body;
    route.fulfill({ response: res, json: { ...rest, manifest } });
  });
  await signIn(page, fx.workspace_id, fx.review_token);
  const evidence = row(page, fx.tiers_self_thread_id).locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  await expect(evidence.locator(".ny-ev-item")).toHaveCount(2);
  await expect(evidence.locator(".ny-ev-tier")).toHaveCount(0);
  await expect(evidence.locator(".ny-ev-warn")).toHaveCount(0);
});

test("each workspace's console shows its own tiers and none of the other's", async ({ page, request }) => {
  // The neighbour's own packet, read with its own token.
  const theirs = await packet(request, fx.other_tiers_thread_id, fx.other_token);
  expect(theirs.self_reported_only).toBe(true);
  expect(theirs.manifest.attestations.map((a: { attested_by: string }) => a.attested_by)).toEqual([
    fx.outsider_member_id,
    fx.outsider_member_id,
  ]);

  await signIn(page, fx.other_workspace_id, fx.other_token);
  const r = row(page, fx.other_tiers_thread_id);
  const evidence = r.locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  await expect(evidence.locator(".ny-ev-warn")).toHaveText(WARNING);
  await expect(evidence.locator('.ny-ev-item[data-ev="result"] .person .name')).toHaveText("Outsider");
  await expect(evidence.locator(".ny-ev-tier")).toHaveCount(2);
  for (const id of [fx.tiers_attached_thread_id, fx.tiers_verified_thread_id, fx.tiers_self_thread_id]) {
    await expect(row(page, id)).toHaveCount(0);
  }
  await expect(page.locator('#needs-you-list .ny-ev-item[data-ev="land_gate"]')).toHaveCount(0);

  // The neighbour's session reads none of the first workspace's packets on
  // either tree, and the first's token reads none of the neighbour's.
  const reads = await page.evaluate(
    async (tid) =>
      Promise.all(
        [`/threads/${tid}/review-packet`, `/ui/api/threads/${tid}/review-packet`].map(async (p) => {
          const res = await fetch(p, { credentials: "same-origin" });
          return { p, status: res.status, body: await res.text() };
        }),
      ),
    fx.tiers_verified_thread_id,
  );
  for (const read of reads) {
    expect([403, 404], read.p).toContain(read.status);
    expect(read.body).not.toContain("attestations");
  }
  for (const path of [`/threads/${fx.other_tiers_thread_id}/review-packet`, `/ui/api/threads/${fx.other_tiers_thread_id}/review-packet`]) {
    const res = await request.get(path, { headers: bearer(fx.review_token) });
    expect([403, 404], path).toContain(res.status());
  }

  // And back in the first workspace, its own rows draw its own tiers and
  // nothing of the neighbour's.
  await page.click("#logout");
  await expect(page.locator("#logout")).toBeHidden();
  await signIn(page, fx.workspace_id, fx.review_token);
  await expect(row(page, fx.tiers_verified_thread_id).locator('.ny-ev-item[data-ev="land_gate"] .ny-ev-tier')).toHaveText(
    "verified",
  );
  await expect(row(page, fx.other_tiers_thread_id)).toHaveCount(0);
});
