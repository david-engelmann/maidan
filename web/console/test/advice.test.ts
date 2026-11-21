import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import { AdviceApi, LAND_TITLE, concernCount, judgmentLine, percent, readingOf, reasonText, rowMark, sureness, type Advice } from "../src/core/advice.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

function api(responses: Array<{ status?: number; body?: unknown }>, creds = session) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let i = 0;
  const fn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const r = responses[Math.min(i++, responses.length - 1)];
    return new Response(JSON.stringify(r.body ?? {}), { status: r.status ?? 200, headers: { "content-type": "application/json" } });
  };
  return { api: new AdviceApi(fn, () => creds, () => "session"), calls };
}

const advice: Advice = {
  provider: "typesafe",
  model: "jev-1.14.0",
  land: "amber",
  reasons: ["tests_not_reported"],
  judgments: [
    { id: "done", label: "Does what the task asks", probability: 0.94, answer: "yes", reading: "Does what the task asks", concern: false },
    { id: "tests_passed", label: "Reports passing tests", probability: 0.012, answer: "no", reading: "Reports no passing tests", concern: true },
    { id: "flags_risk", label: "Warns about a risk", probability: 0.5, answer: "unsettled", reading: "Unclear whether the agent flags a risk", concern: true },
  ],
  key_source: "workspace",
  cached: false,
};

test("a probability reads as a whole percentage that never claims certainty it lacks", () => {
  assert.equal(percent(0.94), "94%");
  assert.equal(percent(0.999), "99%");
  assert.equal(percent(1), "100%");
  assert.equal(percent(0.004), "1%");
  assert.equal(percent(0), "0%");
  assert.equal(percent(Number.NaN), "0%");
  assert.equal(percent(7), "100%");
});

test("each reason and each advice has its own words, and an unknown reason still reads", () => {
  assert.equal(LAND_TITLE.green, "Nothing flagged");
  assert.equal(LAND_TITLE.amber, "Worth a look");
  assert.equal(LAND_TITLE.red, "Problem reported");
  assert.match(reasonText("tests_not_reported"), /no passing tests/);
  assert.match(reasonText("unresolved_failure"), /still failing/);
  assert.equal(reasonText("something_new"), "something new");
});

test("a judgment reads as a statement with how sure the model is of that statement", () => {
  const [done, tests, risk] = advice.judgments;
  assert.equal(judgmentLine(done), "Does what the task asks, 94% sure");
  // 1.2% yes is a no the model is 99% sure of, never "1%".
  assert.equal(sureness(tests), "99% sure");
  assert.equal(judgmentLine(tests), "Reports no passing tests, 99% sure");
  assert.equal(sureness(risk), "unsure");
  assert.deepEqual([done, tests, risk].map(rowMark), ["clear", "concern", "unsure"]);
  // Unsettled, but the rules did not need it: no mark.
  assert.equal(rowMark({ ...risk, concern: false }), "clear");
  assert.equal(concernCount(advice), 2);
  // A server from before `reading` still reads, from the label and the answer.
  assert.equal(readingOf({ ...tests, reading: undefined }), "Reports passing tests: no");
});

test("advice is asked on the session route or the bearer route, with refresh only when asked for", async () => {
  const a = api([{ body: advice }]);
  const got = await a.api.advise("t1");
  assert.equal(got.kind, "ready");
  assert.equal(a.calls[0].url, "http://h/ui/api/threads/t1/land-gate/advice");
  assert.equal(a.calls[0].init?.method, "POST");
  assert.equal(a.calls[0].init?.body, "{}");
  const b = api([{ body: advice }], bearer);
  await b.api.advise("t1", true);
  assert.equal(b.calls[0].url, "http://h/threads/t1/land-gate/advice");
  assert.equal(b.calls[0].init?.body, JSON.stringify({ refresh: true }));
});

test("off, no key and no result are states, and a provider failure is an error that says the task is unchanged", async () => {
  assert.deepEqual(await api([{ status: 404 }]).api.advise("t"), { kind: "off" });
  assert.deepEqual(await api([{ status: 409, body: { detail: "this workspace has no TypeSafe key: store one" } }]).api.advise("t"), { kind: "no-key" });
  assert.deepEqual(await api([{ status: 409, body: { detail: "this task has no result yet" } }]).api.advise("t"), { kind: "none" });
  await assert.rejects(api([{ status: 502, body: { detail: "land-gate advisor unavailable" } }]).api.advise("t"), /The task is unchanged/);
});

test("the workspace key is set, cleared and read without ever being returned", async () => {
  const a = api([{ body: { enabled: true, key_source: "workspace", model: "jev-latest", host: "api.typesafe.ai" } }]);
  const set = await a.api.setKey("ws", "ts_key");
  assert.equal(set.key_source, "workspace");
  assert.equal(a.calls[0].url, "http://h/ui/api/workspaces/ws/land-gate/advisor");
  assert.equal(a.calls[0].init?.method, "PUT");
  assert.equal(a.calls[0].init?.body, JSON.stringify({ api_key: "ts_key" }));
  assert.ok((a.calls[0].init?.headers as Record<string, string>)["Idempotency-Key"]);
  await a.api.clearKey("ws");
  assert.equal(a.calls[1].init?.method, "DELETE");
  // A server from before the advisor answers 404: advice is simply off.
  assert.deepEqual(await api([{ status: 404 }]).api.status("ws"), { enabled: false });
  await assert.rejects(api([{ status: 400, body: { detail: "TypeSafe refused this key" } }]).api.setKey("ws", "bad"), /TypeSafe refused this key/);
});
