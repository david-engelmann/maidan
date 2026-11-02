import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import {
  ThreadsApi,
  ThreadsError,
  evidenceBody,
  evidenceLine,
  evidenceRefusal,
  recordLine,
  safeHref,
  withRecords,
  type EvidenceDraft,
  type EvidenceEntry,
  type EvidenceRecord,
} from "../src/core/threads.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

type Call = { url: string; init?: RequestInit };
function fake(responses: Array<{ status?: number; body?: unknown }>, creds: Credentials) {
  const calls: Call[] = [];
  let i = 0;
  const api = new ThreadsApi(
    async (url: string | URL | Request, init?: RequestInit) => {
      calls.push({ url: String(url), init });
      const r = responses[Math.min(i++, responses.length - 1)] ?? {};
      return new Response(r.body === undefined ? "" : JSON.stringify(r.body), { status: r.status ?? 200 });
    },
    () => creds,
    () => (creds.pastedToken ? "bearer" : "session"),
  );
  return { api, calls };
}

const rec = (o: Partial<EvidenceRecord> & { id: string }): EvidenceRecord => ({
  thread_id: "t",
  kind: "ci_run",
  source: "github-actions",
  url: "https://ci.example.test/run/1",
  recorded_by: "m1",
  recorded_at: "2026-11-02T10:00:00Z",
  ...o,
});
const draft = (o: Partial<EvidenceDraft> = {}): EvidenceDraft => ({ kind: "test_run", source: "cargo test", url: "", sha256: "", note: "", ...o });
const name = (id: string | undefined) => (id === "m1" ? "Ada" : "someone");

test("a record reads as its kind, source, recorder, digest and note", () => {
  assert.equal(recordLine(rec({ id: "r1" }), name), "CI run from github-actions, recorded by Ada");
  assert.equal(
    recordLine(rec({ id: "r2", kind: "diff", sha256: "b".repeat(64), note: "the fix" }), name),
    `Diff from github-actions, recorded by Ada, sha256 ${"b".repeat(12)}…: the fix`,
  );
  assert.equal(recordLine(rec({ id: "r3", kind: "later_kind" }), name), "later_kind from github-actions, recorded by Ada");
  assert.equal(evidenceLine({ kind: "evidence_recorded", at: "", record: rec({ id: "r1" }) }, name), "CI run from github-actions, recorded by Ada");
  assert.equal(evidenceLine({ kind: "evidence_recorded", at: "" }, name), "Evidence recorded");
});

test("only http and https links are rendered", () => {
  assert.equal(safeHref("https://ci.example.test/run/1"), "https://ci.example.test/run/1");
  assert.equal(safeHref("http://x.test"), "http://x.test/");
  for (const bad of ["javascript:alert(1)", "data:text/html,x", "ftp://x.test", "https://", "not a url", "", null, undefined]) {
    assert.equal(safeHref(bad), null, String(bad));
  }
});

test("drafts are refused as the server refuses them", () => {
  assert.equal(evidenceRefusal(draft({ note: "ok" })), null);
  assert.match(evidenceRefusal(draft({ source: "  ", note: "x" })) ?? "", /where it came from/);
  assert.match(evidenceRefusal(draft({ source: "s".repeat(201), note: "x" })) ?? "", /1 to 200/);
  assert.match(evidenceRefusal(draft()) ?? "", /URL, a sha256 or a note/);
  assert.match(evidenceRefusal(draft({ url: "javascript:alert(1)" })) ?? "", /http or https/);
  assert.match(evidenceRefusal(draft({ sha256: "abc" })) ?? "", /64 hex/);
  assert.equal(evidenceRefusal(draft({ sha256: "A".repeat(64) })), null);
  assert.match(evidenceRefusal(draft({ kind: "screenshot", note: "x" })) ?? "", /screenshot needs a URL/);
  assert.match(evidenceRefusal(draft({ kind: "note", url: "https://x.test" })) ?? "", /note needs words/);
  assert.match(evidenceRefusal(draft({ note: "n".repeat(4001) })) ?? "", /4000/);
});

test("the body is trimmed, leaves blanks out and lowercases the digest", () => {
  assert.deepEqual(evidenceBody(draft({ source: " cargo test ", sha256: "A".repeat(64), url: " ", note: " passed " })), {
    kind: "test_run",
    source: "cargo test",
    sha256: "a".repeat(64),
    note: "passed",
  });
});

test("records the timeline lacks are merged in at their time, without duplicates", () => {
  const entries: EvidenceEntry[] = [
    { kind: "artifact_linked", at: "2026-11-02T09:00:00Z" },
    { kind: "evidence_recorded", at: "2026-11-02T10:00:00Z", record: rec({ id: "r1" }) },
    { kind: "review_verdict", at: "2026-11-02T12:00:00Z" },
  ];
  assert.equal(withRecords(entries, null), entries);
  const merged = withRecords(entries, [rec({ id: "r1" }), rec({ id: "r2", recorded_at: "2026-11-02T11:00:00Z" })]);
  assert.deepEqual(
    merged.map((e) => e.record?.id ?? e.kind),
    ["artifact_linked", "r1", "r2", "review_verdict"],
  );
});

test("a bearer reads the records list; a session reads the timeline only", async () => {
  const b = fake([{ body: [rec({ id: "r1" })] }], bearer);
  assert.deepEqual((await b.api.records("t1"))?.map((r) => r.id), ["r1"]);
  assert.equal(b.calls[0].url, "http://h/threads/t1/evidence-records");
  const s = fake([{ body: [] }], session);
  assert.equal(await s.api.records("t1"), null);
  assert.equal(s.calls.length, 0, "no bearer-only call from a session");
  const f = fake([{ status: 500, body: {} }], bearer);
  assert.equal(await f.api.records("t1"), null, "a failed read is nothing to show");
});

test("adding a record posts the body once with an idempotency key; refusals never go out", async () => {
  const b = fake([{ status: 201, body: rec({ id: "r9" }) }], bearer);
  const out = await b.api.addEvidence("t1", draft({ note: "passed" }));
  assert.equal(out?.id, "r9");
  assert.equal(b.calls.length, 1);
  assert.equal(b.calls[0].url, "http://h/threads/t1/evidence-records");
  assert.equal(b.calls[0].init?.method, "POST");
  assert.deepEqual(JSON.parse(String(b.calls[0].init?.body)), { kind: "test_run", source: "cargo test", note: "passed" });
  assert.ok((b.calls[0].init?.headers as Record<string, string>)["Idempotency-Key"]);

  await assert.rejects(b.api.addEvidence("t1", draft()), ThreadsError);
  assert.equal(b.calls.length, 1);

  const s = fake([], session);
  await assert.rejects(s.api.addEvidence("t1", draft({ note: "x" })), /bearer token/);
  assert.equal(s.calls.length, 0);

  const r = fake([{ status: 400, body: { detail: "url must be an http or https URL" } }], bearer);
  await assert.rejects(r.api.addEvidence("t1", draft({ url: "https://x.test" })), /http or https/);
});
