import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import {
  ALLOWLIST_NEED,
  EGRESS_SURFACES,
  asksForAllowlist,
  canReplay,
  deliveryReason,
  deliveryTarget,
  isSurface,
  replayOutcome,
  resultDeliveryQuery,
  selectorRefusal,
  statusTone,
  statusWord,
  surfaceWord,
  withRow,
  type ResultDelivery,
} from "../src/core/deliveries.ts";
import { ToolsApi } from "../src/core/tools.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

function api(responses: Array<{ status?: number; body?: unknown }>, creds = session) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let i = 0;
  const fn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const r = responses[Math.min(i++, responses.length - 1)] ?? {};
    return new Response(r.body === undefined ? null : JSON.stringify(r.body), { status: r.status ?? 200 });
  };
  return { api: new ToolsApi(fn, () => creds, () => "session"), calls };
}

const row = (over: Partial<ResultDelivery> = {}): ResultDelivery => ({
  id: "d1",
  thread_id: "t1",
  surface: "github",
  selector: "acme/widgets#7",
  status: "failed",
  attempts: 3,
  last_error: "GitHub answered 502",
  created_at: "2026-11-17T10:00:00Z",
  updated_at: "2026-11-17T10:05:00Z",
  ...over,
});

test("a GitHub repository is owner/name, with no issue and no branch", () => {
  assert.equal(selectorRefusal("github", "acme/widgets"), null);
  assert.equal(selectorRefusal("github", "  acme/widgets  "), null, "a stray space is trimmed, not refused");
  assert.match(selectorRefusal("github", "acme/widgets#7") ?? "", /without an issue number/);
  assert.match(selectorRefusal("github", "acme/widgets@main") ?? "", /choose GitHub branch/);
  assert.match(selectorRefusal("github", "widgets") ?? "", /owner\/name/);
  assert.match(selectorRefusal("github", "acme/widgets/extra") ?? "", /owner\/name/);
});

test("a GitHub branch names the repository and one base", () => {
  assert.equal(selectorRefusal("github_branch", "acme/widgets@main"), null);
  assert.equal(selectorRefusal("github_branch", "acme/widgets@release/2026-11"), null);
  assert.match(selectorRefusal("github_branch", "acme/widgets") ?? "", /owner\/name@base/);
  assert.match(selectorRefusal("github_branch", "widgets@main") ?? "", /Start with the repository/);
  assert.match(selectorRefusal("github_branch", "acme/widgets@") ?? "", /End with the base branch/);
  // Only the server knows its forbidden bases; the shape alone passes here.
  assert.equal(selectorRefusal("github_branch", "acme/widgets@prod"), null);
});

test("a Slack channel is its id, never its #name", () => {
  assert.equal(selectorRefusal("slack", "C0123ABCDEF"), null);
  assert.equal(selectorRefusal("slack", "G0123ABCDEF"), null);
  assert.match(selectorRefusal("slack", "#general") ?? "", /not its #name/);
  assert.match(selectorRefusal("slack", "D0123") ?? "", /starts with C or G/);
  assert.match(selectorRefusal("slack", "C") ?? "", /starts with C or G/);
});

test("a Linear team is its lower-case UUID, never its key", () => {
  assert.equal(selectorRefusal("linear", "9cfb482a-81e3-4154-b5b9-2c805e70a02d"), null);
  assert.match(selectorRefusal("linear", "ENG") ?? "", /UUID/);
  assert.match(selectorRefusal("linear", "9CFB482A-81E3-4154-B5B9-2C805E70A02D") ?? "", /lower case/);
});

test("an empty selector is asked for, on every surface", () => {
  for (const s of EGRESS_SURFACES) assert.equal(selectorRefusal(s.id, "   "), "Say which destination to allow.");
});

test("every surface has a field name and an example, and only those are surfaces", () => {
  assert.deepEqual(EGRESS_SURFACES.map((s) => s.id), ["github", "github_branch", "slack", "linear"]);
  for (const s of EGRESS_SURFACES) {
    assert.ok(s.label && s.field && s.example);
    assert.equal(isSurface(s.id), true);
  }
  assert.equal(isSurface("email"), false);
  assert.equal(surfaceWord("github_branch"), "GitHub branch");
  assert.equal(surfaceWord("teams"), "teams", "a surface from a newer server is shown as stored");
});

test("statuses read as words, and only a failed or skipped target replays", () => {
  assert.deepEqual(["pending", "delivered", "failed", "skipped"].map(statusWord), ["Pending", "Delivered", "Failed", "Skipped"]);
  assert.equal(statusWord("held"), "Held");
  assert.equal(statusWord(""), "Unknown");
  assert.equal(statusTone("delivered"), "active");
  assert.equal(statusTone("failed"), "revoked");
  assert.equal(statusTone("skipped"), "", "a skip is not an error");
  assert.equal(canReplay({ status: "failed" }), true);
  assert.equal(canReplay({ status: "skipped" }), true);
  assert.equal(canReplay({ status: "pending" }), false);
  assert.equal(canReplay({ status: "delivered" }), false);
});

test("a delivery says where it went and, when it did not land, why", () => {
  assert.equal(deliveryTarget(row()), "GitHub · acme/widgets#7");
  assert.equal(deliveryReason(row()), "GitHub answered 502");
  assert.equal(deliveryReason(row({ last_error: null })), "The delivery gave up after its retries.");
  assert.equal(deliveryReason(row({ status: "skipped", last_error: "target not in the workspace egress allowlist" })), "target not in the workspace egress allowlist");
  assert.equal(deliveryReason(row({ status: "skipped", last_error: "  " })), "Not delivered on purpose.");
  assert.equal(deliveryReason(row({ status: "pending", attempts: 2 })), "Trying again after: GitHub answered 502");
  assert.equal(deliveryReason(row({ status: "pending", attempts: 0, last_error: null })), null);
  assert.equal(deliveryReason(row({ status: "delivered", last_error: null })), null);
});

test("a replay's notice comes from the row the server sent back", () => {
  assert.deepEqual(replayOutcome(row({ status: "pending", last_error: null })), { text: "Sending again to GitHub · acme/widgets#7", ok: true });
  assert.deepEqual(replayOutcome(row({ status: "skipped", last_error: "target not in the workspace egress allowlist" })), { text: "Still skipped: target not in the workspace egress allowlist", ok: false });
  assert.deepEqual(replayOutcome(row({ status: "skipped", last_error: null })), { text: "Still skipped: the target is not allowed", ok: false });
  assert.equal(replayOutcome(row({ status: "failed" })).ok, false);
  assert.equal(replayOutcome(row({ status: "delivered" })).ok, true);
});

test("a replayed row replaces its old copy in place", () => {
  const rows = [row({ id: "a" }), row({ id: "b" }), row({ id: "c" })];
  const next = withRow(rows, row({ id: "b", status: "pending" }));
  assert.deepEqual(next.map((r) => [r.id, r.status]), [["a", "failed"], ["b", "pending"], ["c", "failed"]]);
  assert.equal(rows[1].status, "failed", "the list it came from is left alone");
});

test("the status filter is a query, and all asks for every status", () => {
  assert.equal(resultDeliveryQuery("all"), "limit=50");
  assert.equal(resultDeliveryQuery("failed"), "status=failed&limit=50");
  assert.equal(resultDeliveryQuery("skipped", 10), "status=skipped&limit=10");
});

test("a token asks for the allowlist only with token:admin; a sign-in always asks", () => {
  assert.equal(asksForAllowlist(true, { capabilities: ["workspace:read", "token:admin"] }), true);
  assert.equal(asksForAllowlist(true, { capabilities: ["workspace:read"] }), false);
  assert.equal(asksForAllowlist(true, null), false);
  assert.equal(asksForAllowlist(false, { capabilities: ["workspace:read"] }), true);
  assert.ok(ALLOWLIST_NEED.startsWith("Only a workspace admin"));
});

test("result deliveries and their replay go to the session tree, or the bearer tree with a token", async () => {
  const s = api([{ body: [] }]);
  await s.api.resultDeliveries("w", "failed");
  assert.equal(s.calls[0].url, "http://h/ui/api/workspaces/w/result-deliveries?status=failed&limit=50");
  const b = api([{ body: row({ status: "pending" }) }], bearer);
  const back = await b.api.replayResult("t1", "d1");
  assert.equal(back.status, "pending");
  assert.equal(b.calls[0].url, "http://h/threads/t1/deliveries/d1/replay");
  assert.equal(b.calls[0].init?.method, "POST");
  assert.ok(new Headers(b.calls[0].init?.headers).get("Idempotency-Key"));
});

test("a replay with no row back is an error, not a silent success", async () => {
  const { api: a } = api([{ status: 200 }]);
  await assert.rejects(a.replayResult("t1", "d1"), /did not say how the replay went/);
});

test("the allowlist is a quiet refusal on 403, and a list otherwise", async () => {
  const no = api([{ status: 403, body: { detail: "only a workspace admin can see or change where results may be delivered" } }]);
  assert.deepEqual(await no.api.egressTargets("w"), { allowed: false });
  assert.equal(no.calls[0].url, "http://h/ui/api/workspaces/w/egress-targets");
  const yes = api([{ body: [{ id: "e1", surface: "slack", selector: "C1", created_at: "2026-11-17T10:00:00Z", created_by: "me" }] }], bearer);
  const read = await yes.api.egressTargets("w");
  assert.equal(read.allowed && read.value[0].selector, "C1");
  assert.equal(yes.calls[0].url, "http://h/workspaces/w/egress-targets");
});

test("a bad selector is refused before anything is sent", async () => {
  const { api: a, calls } = api([{ status: 201, body: {} }]);
  await assert.rejects(a.allowEgressTarget("w", "slack", "#general"), /not its #name/);
  assert.equal(calls.length, 0);
});

test("a good selector is sent trimmed, and the server's refusal comes back in its words", async () => {
  const ok = api([{ status: 201, body: { id: "e1", surface: "github", selector: "acme/widgets", created_at: "2026-11-17T10:00:00Z" } }]);
  const made = await ok.api.allowEgressTarget("w", "github", " acme/widgets ");
  assert.equal(made.id, "e1");
  assert.deepEqual(JSON.parse(String(ok.calls[0].init?.body)), { surface: "github", selector: "acme/widgets" });
  const refused = api([{ status: 400, body: { detail: "`prod` is never an allowed base for change pull requests" } }]);
  await assert.rejects(refused.api.allowEgressTarget("w", "github_branch", "acme/widgets@prod"), /`prod` is never an allowed base/);
});

test("removing an entry is a DELETE on its id", async () => {
  const { api: a, calls } = api([{ status: 204 }]);
  await a.revokeEgressTarget("w", "e1");
  assert.equal(calls[0].url, "http://h/ui/api/workspaces/w/egress-targets/e1");
  assert.equal(calls[0].init?.method, "DELETE");
});
