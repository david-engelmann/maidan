import assert from "node:assert/strict";
import { test } from "node:test";
import { NeedsApi, NeedsError, approvedLine, groupNeeds, kindLabel, modelRequestLine, movesNeeds, needKey, sortNeeds, splitBlocked, splitQuestion, waitingFor, type GateView, type WaitingItem } from "../src/core/needs.ts";
import type { Credentials } from "../src/core/api.ts";
import { backoff } from "../src/hooks/useNeeds.ts";

const it = (o: Partial<WaitingItem> & { kind: string }): WaitingItem => ({ summary: "s", since: "2026-10-30T10:00:00Z", age_secs: 60, ...o });

test("only the kinds Needs you acts on are kept, decisions first", () => {
  const s = sortNeeds([
    it({ kind: "question", thread_id: "q" }),
    it({ kind: "mention", thread_id: "m" }),
    it({ kind: "blocked", thread_id: "b" }),
    it({ kind: "open_gate", gate_id: "g", since: "2026-10-30T11:00:00Z" }),
    it({ kind: "review_request", thread_id: "r", since: "2026-10-30T09:00:00Z" }),
    it({ kind: "assigned", thread_id: "a" }),
  ]);
  assert.deepEqual(s.map((x) => x.kind), ["review_request", "open_gate", "blocked", "question"]);
});

test("within a kind, newest first", () => {
  const s = sortNeeds([it({ kind: "open_gate", gate_id: "old", since: "2026-10-30T08:00:00Z" }), it({ kind: "open_gate", gate_id: "new", since: "2026-10-30T09:00:00Z" })]);
  assert.deepEqual(s.map((x) => x.gate_id), ["new", "old"]);
});

test("headings show only for two or more groups", () => {
  assert.equal(groupNeeds(sortNeeds([it({ kind: "open_gate" }), it({ kind: "review_request" })])).headings, false);
  const g = groupNeeds(sortNeeds([it({ kind: "open_gate" }), it({ kind: "question" })]));
  assert.equal(g.headings, true);
  assert.deepEqual(g.groups.map((x) => x.label), ["Needs your decision", "An agent asked"]);
});

test("row keys, labels, and the split summaries", () => {
  assert.equal(needKey(it({ kind: "open_gate", thread_id: "t", gate_id: "g" })), "open_gate:t:g");
  assert.equal(kindLabel("unassigned_review"), "Review");
  assert.equal(kindLabel("open_gate"), "Approval");
  assert.deepEqual(splitBlocked("Ship it — blocked (ci): red"), { title: "Ship it", why: "blocked (ci): red" });
  assert.deepEqual(splitBlocked("plain"), { title: "plain", why: "plain" });
  assert.deepEqual(splitQuestion(it({ kind: "question", summary: "A: b: which db?", detail: "which db?" })), { title: "A: b", question: "which db?" });
  assert.equal(waitingFor(30), "waiting under a minute");
  assert.equal(waitingFor(7200), "waiting 2h");
  assert.equal(approvedLine({ approvals: 1, required_count: 2, approvals_met: false }), "Approved ✓ 1/2");
  assert.equal(approvedLine(null), "Approved ✓");
  assert.equal(modelRequestLine({ client_name: "cli", client_source: "credential" }), "a model asked via cli, waiting for you to confirm");
  assert.equal(modelRequestLine(null), null);
});

test("live frames that move the list", () => {
  assert.ok(movesNeeds("approval_gate_created"));
  assert.ok(movesNeeds("review_submitted"));
  assert.ok(movesNeeds("message_posted"));
  assert.ok(!movesNeeds("member_joined"));
  assert.ok(!movesNeeds(undefined));
});

test("retries back off from 5 s to a minute", () => {
  assert.deepEqual([1, 2, 3, 4, 5, 9].map(backoff), [5000, 10000, 20000, 40000, 60000, 60000]);
});

type Call = { url: string; init?: RequestInit };
function rig(responses: Record<string, () => Response>, creds: Partial<Credentials> = {}) {
  const calls: Call[] = [];
  const c: Credentials = { base: "https://m.example", pastedToken: "", tokenSession: false, sessionMemberId: "me", ...creds };
  const api = new NeedsApi(
    async (url, init) => {
      calls.push({ url, init });
      const key = Object.keys(responses).find((k) => url.endsWith(k) && (!k.startsWith("POST ") || init?.method === "POST"));
      const r = key ? responses[key] : undefined;
      return r ? r() : new Response("{}", { status: 200 });
    },
    () => c,
    () => "session",
  );
  return { api, calls };
}
const json = (b: unknown, status = 200) => () => new Response(JSON.stringify(b), { status });
const view: GateView = { gate: { id: "g1", prompt: "Deploy?" }, request_state: "sig" };

test("waiting reads through /ui/api and sorts", async () => {
  const { api, calls } = rig({ "/members/me/waiting": json({ items: [it({ kind: "question" }), it({ kind: "review_request" })] }) });
  const items = await api.waiting("me");
  assert.equal(calls[0].url, "https://m.example/ui/api/members/me/waiting");
  assert.deepEqual(items.map((x) => x.kind), ["review_request", "question"]);
});

test("a gate answer echoes request_state and carries the reason in content.note", async () => {
  const { api, calls } = rig({});
  await api.answerGate(view, "decline", "  not on Friday ");
  assert.equal(calls[0].url, "https://m.example/ui/api/approval-gates/g1/answer");
  assert.deepEqual(JSON.parse(String(calls[0].init?.body)), { request_state: "sig", action: "decline", content: { note: "not on Friday" } });
  assert.ok((calls[0].init?.headers as Record<string, string>)["Idempotency-Key"]);
  await api.answerGate(view, "accept");
  assert.deepEqual(JSON.parse(String(calls[1].init?.body)), { request_state: "sig", action: "accept" });
});

test("a bearer token goes to the bearer route", async () => {
  const { api, calls } = rig({}, { pastedToken: "t0k" });
  await api.unblock("t1");
  assert.equal(calls[0].url, "https://m.example/threads/t1/block");
  assert.equal(calls[0].init?.method, "DELETE");
});

test("approve signs the evidence root; request changes carries the note only", async () => {
  const { api, calls } = rig({});
  await api.review("t1", "approve", "", "root-1");
  assert.deepEqual(JSON.parse(String(calls[0].init?.body)), { decision: "approve", evidence_root: "root-1" });
  await api.review("t1", "request_changes", "add tests", "root-1");
  assert.deepEqual(JSON.parse(String(calls[1].init?.body)), { decision: "request_changes", note: "add tests" });
});

test("a missing review packet is no evidence, not an error", async () => {
  const { api } = rig({ "/threads/t1/review-packet": json({ error: "nope" }, 404) });
  assert.equal(await api.reviewRoot("t1"), null);
  const ok = rig({ "/threads/t2/review-packet": json({ evidence_root: "r" }) });
  assert.equal(await ok.api.reviewRoot("t2"), "r");
});

test("a second answer says someone already answered; a refusal is in words", async () => {
  const { api } = rig({ "/approval-gates/g1/answer": json({}, 409) });
  await assert.rejects(api.answerGate(view, "accept"), (e: Error) => e instanceof NeedsError && e.message === "Someone already answered this");
  const r = rig({ "/approval-gates/g1/answer": json({}, 500) });
  await assert.rejects(r.api.answerGate(view, "accept"), /server hit an error/);
});

test("gate views are keyed by id, and a failed read is empty", async () => {
  const { api } = rig({ "/workspaces/w1/approval-gates": json([view]) });
  assert.equal((await api.gates("w1")).get("g1")?.request_state, "sig");
  const bad = rig({ "/workspaces/w1/approval-gates": json({}, 500) });
  assert.equal((await bad.api.gates("w1")).size, 0);
});
