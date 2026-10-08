import { test, expect, Page, APIRequestContext } from "@playwright/test";
import { fixtures } from "./_fixtures";
import { signIn } from "./_tools";

const fx = fixtures();

// A review row in Needs you shows the evidence it approves: the review
// packet's root, the result's hash and who produced it, then each linked
// artifact. Approve sends the root the row showed, and the row names who
// decided. Attestation tiers are not here: nothing on the server defines one.

// The decide spec approves a seeded task, so a retry would find it decided.
test.describe.configure({ retries: 0 });

const bearer = (token: string) => ({ Authorization: `Bearer ${token}` });

const row = (page: Page, threadId: string) =>
  page.locator(`#needs-you-list .ny-item[data-thread-id="${threadId}"]`);

async function packet(request: APIRequestContext, threadId: string) {
  const res = await request.get(`/threads/${threadId}/review-packet`, { headers: bearer(fx.review_token) });
  expect(res.ok()).toBeTruthy();
  return res.json();
}

test("a review row lists the packet's evidence: the result, then each artifact", async ({ page, request }) => {
  const handed = await packet(request, fx.proof_view_thread_id);
  await signIn(page, fx.workspace_id, fx.review_token);

  const evidence = row(page, fx.proof_view_thread_id).locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  await expect(evidence.locator(".ny-ev-root")).toHaveAttribute("data-root", handed.evidence_root);
  await expect(evidence.locator(".ny-ev-root")).toHaveText(`${handed.evidence_root.slice(0, 12)}…`);

  const result = evidence.locator('.ny-ev-item[data-ev="result"]');
  await expect(result.locator(".ny-ev-hash")).toHaveAttribute("title", handed.manifest.result.sha256);
  await expect(result).toContainText("produced by");
  await expect(result.locator(".person .name")).toHaveText("Deployer");

  // The manifest's order: hashes sorted.
  const items = evidence.locator('.ny-ev-item[data-ev="artifact"]');
  await expect(items).toHaveCount(2);
  expect(await items.evaluateAll((els) => els.map((el) => (el as HTMLElement).dataset.sha))).toEqual(
    handed.manifest.artifacts,
  );
  const shot = evidence.locator(`.ny-ev-item[data-sha="${fx.proof_screenshot_sha}"]`);
  await expect(shot.locator(".ny-ev-kind")).toHaveText("screenshot");
  await expect(shot.locator(".ny-ev-name")).toHaveText("login-after.png");
  await expect(shot.locator(".ny-ev-size")).toHaveText("2.0 KB");
  await expect(shot.locator(".ny-ev-by")).toContainText("uploaded by");
  await expect(shot.locator(".ny-ev-by .person .name")).toHaveText("Deployer");
  await expect(shot.locator(".ny-ev-by")).toContainText(/(just now|\d+[mhd] ago)$/);
  const log = evidence.locator(`.ny-ev-item[data-sha="${fx.proof_transcript_sha}"]`);
  await expect(log.locator(".ny-ev-kind")).toHaveText("transcript");
  await expect(log.locator(".ny-ev-name")).toHaveText("test-run.log");
  await expect(log.locator(".ny-ev-size")).toHaveText("21 B");
  await expect(evidence.locator(".ny-ev-err")).toHaveCount(0);
  await expect(evidence.locator(".ny-ev-decider")).toHaveCount(0);
});

test("a packet with no result and no artifact says nothing was handed over", async ({ page }) => {
  await signIn(page, fx.workspace_id, fx.review_token);
  const evidence = row(page, fx.proof_empty_thread_id).locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "empty");
  await expect(evidence.locator(".ny-ev-empty")).toHaveText(
    "No evidence was handed over: no result and no linked artifacts.",
  );
  await expect(evidence.locator(".ny-ev-item")).toHaveCount(0);
  // The packet still has a root, so the empty hand-off is what Approve names.
  await expect(evidence.locator(".ny-ev-root")).toHaveAttribute("data-root", /^[0-9a-f]{64}$/);
  await expect(row(page, fx.proof_empty_thread_id).getByRole("button", { name: "Approve", exact: true })).toBeEnabled();
});

test("one artifact whose details fail says so, and the rest of the evidence still shows", async ({ page }) => {
  await page.route(`**/artifacts/${fx.proof_screenshot_sha}/meta`, (route) =>
    route.fulfill({ status: 500, contentType: "application/json", body: '{"error":"boom"}' }),
  );
  await signIn(page, fx.workspace_id, fx.review_token);
  const r = row(page, fx.proof_view_thread_id);
  const evidence = r.locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "ready");
  const shot = evidence.locator(`.ny-ev-item[data-sha="${fx.proof_screenshot_sha}"]`);
  await expect(shot).toHaveClass(/ny-ev-failed/);
  await expect(shot.locator(".ny-ev-err")).toHaveText(
    "Could not load this artifact's details: The server hit an error. Try again; if it keeps failing, check the server log.",
  );
  await expect(evidence.locator('.ny-ev-item[data-ev="result"] .person .name')).toHaveText("Deployer");
  await expect(evidence.locator(`.ny-ev-item[data-sha="${fx.proof_transcript_sha}"] .ny-ev-name`)).toHaveText(
    "test-run.log",
  );
  // The evidence root came back, so a missing artifact detail does not block a decision.
  await expect(r.getByRole("button", { name: "Approve", exact: true })).toBeEnabled();
});

test("a failed packet read shows an error with Retry, and Retry loads the evidence", async ({ page }) => {
  let fail = true;
  await page.route(`**/threads/${fx.proof_view_thread_id}/review-packet`, (route) =>
    fail ? route.fulfill({ status: 503, body: "" }) : route.continue(),
  );
  await signIn(page, fx.workspace_id, fx.review_token);
  const r = row(page, fx.proof_view_thread_id);
  const evidence = r.locator(".ny-evidence");
  await expect(evidence).toHaveAttribute("data-state", "error");
  await expect(evidence.locator(".ny-ev-err")).toHaveText(
    "Could not load the evidence: The server hit an error. Try again; if it keeps failing, check the server log.",
  );
  await expect(r).not.toHaveAttribute("data-evidence-root", /.+/);
  // Nothing to bind an approval to: both approve buttons stay off, and a
  // change request, which needs no packet, stays on.
  await expect(r.getByRole("button", { name: "Approve", exact: true })).toBeDisabled();
  await expect(r.getByRole("button", { name: "Approve with note" })).toBeDisabled();
  await expect(r.getByRole("button", { name: "Request changes" })).toBeEnabled();
  fail = false;
  await evidence.getByRole("button", { name: "Retry" }).click();
  await expect(evidence).toHaveAttribute("data-state", "ready");
  await expect(evidence.locator('.ny-ev-item[data-ev="artifact"]')).toHaveCount(2);
  await expect(r).toHaveAttribute("data-evidence-root", /^[0-9a-f]{64}$/);
  await expect(r.getByRole("button", { name: "Approve", exact: true })).toBeEnabled();
  await expect(r.getByRole("button", { name: "Approve with note" })).toBeEnabled();
});

test("Approve sends the root the row showed, and the row names who decided", async ({ page, request }) => {
  const handed = await packet(request, fx.proof_decide_thread_id);
  await signIn(page, fx.workspace_id, fx.review_token);
  const r = row(page, fx.proof_decide_thread_id);
  const root = r.locator(".ny-evidence .ny-ev-root");
  await expect(root).toHaveAttribute("data-root", handed.evidence_root);
  const shown = await root.getAttribute("data-root");

  const sent = page.waitForRequest(
    (req) => req.method() === "POST" && req.url().endsWith(`/threads/${fx.proof_decide_thread_id}/reviews`),
  );
  await r.getByRole("button", { name: "Approve", exact: true }).click();
  const body = (await sent).postDataJSON();
  expect(body.decision).toBe("approve");
  expect(body.evidence_root).toBe(shown);
  expect(body.evidence_root).toBe(handed.evidence_root);

  await expect(r.locator(".done-note")).toHaveText("Approved ✓ 1/1");
  const decider = r.locator(".ny-ev-decider");
  await expect(decider).toHaveCount(1);
  await expect(decider).toHaveText("Operator approved");
  await expect(decider).toHaveAttribute("data-reviewer", fx.member_id);

  // The server recorded the same root against the operator's approval.
  const reviews = await request.get(`/threads/${fx.proof_decide_thread_id}/reviews`, {
    headers: bearer(fx.review_token),
  });
  expect(reviews.ok()).toBeTruthy();
  const mine = (await reviews.json()).find((x: { reviewer_id: string }) => x.reviewer_id === fx.member_id);
  expect(mine.decision).toBe("approve");
  expect(mine.evidence_root).toBe(handed.evidence_root);
});

test("another reviewer's verdict on the socket names them on the row", async ({ page, request }) => {
  // The live grant reads and subscribes; Rae decides through the API.
  await signIn(page, fx.workspace_id, fx.live_token);
  const r = row(page, fx.proof_live_thread_id);
  await expect(r.locator(".ny-evidence")).toHaveAttribute("data-state", "ready");
  if ((await page.locator("#ws-status").textContent()) !== "connected") await page.click("#ws-connect");
  await expect(page.locator("#ws-status")).toHaveText("connected");
  await expect(r.locator(".ny-ev-decider")).toHaveCount(0);

  const handed = await packet(request, fx.proof_live_thread_id);
  const res = await request.post(`/threads/${fx.proof_live_thread_id}/reviews`, {
    headers: bearer(fx.rae_token),
    data: { decision: "approve", evidence_root: handed.evidence_root },
  });
  expect(res.ok()).toBeTruthy();

  const decider = r.locator(`.ny-ev-decider[data-reviewer="${fx.rae_member_id}"]`);
  await expect(decider).toHaveText("Rae Reviewer approved");
  // The operator still owes a verdict, so the row and its buttons stay.
  await expect(r.getByRole("button", { name: "Approve", exact: true })).toBeVisible();
});

test("workspace B's console cannot load workspace A's packet or artifact details", async ({ page, request }) => {
  // A reads its own packet and artifact: the probes below are not 404 for a typo.
  expect((await request.get(`/threads/${fx.proof_view_thread_id}/review-packet`, { headers: bearer(fx.review_token) })).ok()).toBeTruthy();
  expect((await request.get(`/artifacts/${fx.proof_screenshot_sha}/meta`, { headers: bearer(fx.review_token) })).ok()).toBeTruthy();

  await signIn(page, fx.second_workspace_id, fx.second_token);
  await expect(page.locator("#needs-you")).toBeVisible();
  // B's queue holds its own blocked task and none of A's evidence rows.
  await expect(page.locator(`#needs-you-list .ny-item[data-thread-id="${fx.second_thread_id}"]`)).toBeVisible();
  for (const id of [fx.proof_view_thread_id, fx.proof_decide_thread_id, fx.proof_live_thread_id, fx.proof_empty_thread_id]) {
    await expect(row(page, id)).toHaveCount(0);
  }
  await expect(page.locator("#needs-you-list .ny-evidence")).toHaveCount(0);

  // The console's own reads, with B's session, on both trees. A refusal is
  // a 403 or a 404 depending on the tree; neither carries A's evidence.
  const handed = await packet(request, fx.proof_view_thread_id);
  const reads = await page.evaluate(
    async ({ tid, sha }) => {
      const paths = [
        `/threads/${tid}/review-packet`,
        `/ui/api/threads/${tid}/review-packet`,
        `/artifacts/${sha}/meta`,
        `/ui/api/artifacts/${sha}/meta`,
      ];
      const out: { path: string; status: number; body: string }[] = [];
      for (const path of paths) {
        const res = await fetch(path, { credentials: "include" });
        out.push({ path, status: res.status, body: await res.text() });
      }
      return out;
    },
    { tid: fx.proof_view_thread_id, sha: fx.proof_screenshot_sha },
  );
  for (const { path, status, body } of reads) {
    expect([403, 404], path).toContain(status);
    expect(body, path).not.toContain(handed.evidence_root);
    expect(body, path).not.toContain("login-after.png");
  }

  // The page's own packet reader comes back empty-handed for A's task.
  const fromPage = await page.evaluate(async (tid) => {
    const importer = new Function("href", "return import(href)") as (href: string) => Promise<Record<string, unknown>>;
    const mod = await importer("/ui/static/needs.js");
    return (mod.reviewPacket as (t: string) => Promise<unknown>)(tid);
  }, fx.proof_view_thread_id);
  expect(fromPage).toBeNull();

  // And B's token, straight at the API.
  for (const path of [`/threads/${fx.proof_view_thread_id}/review-packet`, `/artifacts/${fx.proof_screenshot_sha}/meta`]) {
    const res = await request.get(path, { headers: bearer(fx.second_token) });
    expect([403, 404], path).toContain(res.status());
    expect(await res.text(), path).not.toContain(handed.evidence_root);
  }
});
