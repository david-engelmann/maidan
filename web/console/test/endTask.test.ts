import { test } from "node:test";
import assert from "node:assert/strict";
import {
  CANCEL_REASON_MAX,
  EndTaskApi,
  EndTaskError,
  cancelReasonRefusal,
  closeBody,
  closeRefusal,
  endActionsFor,
  endEffects,
  endRefusal,
  taskIdFrom,
} from "../src/core/endTask.ts";
import type { Credentials } from "../src/core/api.ts";

const ME = "0190a1b2-0000-7000-8000-000000000001";
const OTHER = "0190a1b2-0000-7000-8000-000000000002";
const TASK = "0190a1b2-0000-7000-8000-0000000000aa";
const TWIN = "0190a1b2-0000-7000-8000-0000000000bb";
const MEMBER = ["workspace:read", "workspace:write", "message:post", "thread:transition"];

const task = (over: Partial<{ state: string; assignee_id: string | null; created_by: string | null }> = {}) => ({
  state: "open",
  assignee_id: null,
  created_by: OTHER,
  ...over,
});

test("a signed-in member can cancel or close any live task in the workspace", () => {
  assert.deepEqual(endActionsFor(task(), ME, MEMBER), { cancel: true, abandon: false });
  assert.deepEqual(endActionsFor(task({ created_by: ME }), ME, MEMBER), { cancel: true, abandon: true });
  assert.deepEqual(endActionsFor(task({ assignee_id: ME }), ME, MEMBER), { cancel: true, abandon: true });
  assert.deepEqual(endActionsFor(task({ state: "in_review", created_by: ME }), ME, MEMBER), { cancel: true, abandon: true });
});

test("a maintainer may close as abandoned a task someone else made and holds", () => {
  assert.deepEqual(endActionsFor(task({ assignee_id: OTHER }), ME, [...MEMBER, "token:admin"]), { cancel: true, abandon: true });
});

test("without thread:transition only the creator may cancel, and nobody may close as abandoned", () => {
  const writer = ["workspace:read", "workspace:write"];
  assert.deepEqual(endActionsFor(task({ created_by: ME }), ME, writer), { cancel: true, abandon: false });
  assert.deepEqual(endActionsFor(task(), ME, writer), { cancel: false, abandon: false });
  assert.deepEqual(endActionsFor(task({ created_by: ME }), ME, ["workspace:read", "thread:transition"]), { cancel: false, abandon: true });
});

test("an ended task, an unknown caller or unknown capabilities offer nothing", () => {
  for (const state of ["closed", "archived"]) assert.deepEqual(endActionsFor(task({ state, created_by: ME }), ME, MEMBER), { cancel: false, abandon: false });
  assert.deepEqual(endActionsFor(task({ created_by: ME }), null, MEMBER), { cancel: false, abandon: false });
  assert.deepEqual(endActionsFor(task({ created_by: ME }), ME, null), { cancel: false, abandon: false });
});

test("the confirm step names what each ending does before anything is sent", () => {
  const cancel = endEffects("cancel").join("\n");
  assert.match(cancel, /claim ends at once/);
  assert.match(cancel, /never reopened/);
  assert.match(cancel, /reason is posted on the task/);
  assert.match(cancel, /result already written is kept/);
  const abandoned = endEffects("abandon", "abandoned").join("\n");
  assert.match(abandoned, /keep waiting/);
  assert.match(abandoned, /review and land gates still apply/);
  assert.match(endEffects("abandon", "duplicate").join("\n"), /move to the task you name/);
  assert.match(endEffects("abandon", "superseded").join("\n"), /move to the task you name/);
});

test("a cancel needs a reason the server keeps", () => {
  assert.ok(cancelReasonRefusal(""));
  assert.ok(cancelReasonRefusal("   \n "));
  assert.equal(cancelReasonRefusal("nobody needs it"), null);
  assert.equal(cancelReasonRefusal("x".repeat(CANCEL_REASON_MAX)), null);
  assert.match(cancelReasonRefusal("x".repeat(CANCEL_REASON_MAX + 1)) ?? "", /under 500/);
  // Counted in characters, as the server counts, not UTF-16 units.
  assert.equal(cancelReasonRefusal("🙂".repeat(CANCEL_REASON_MAX)), null);
});

test("a pasted console link or bare id names the other task", () => {
  assert.equal(taskIdFrom(`https://maidan.world/ui/#/threads/t/${TWIN.toUpperCase()}`), TWIN);
  assert.equal(taskIdFrom(` ${TWIN} `), TWIN);
  assert.equal(taskIdFrom("task 42"), null);
});

test("a duplicate must name another task; a superseded close may", () => {
  assert.equal(closeRefusal("abandoned", "", TASK), null);
  assert.equal(closeRefusal("abandoned", "garbage", TASK), null);
  assert.match(closeRefusal("duplicate", "", TASK) ?? "", /Paste a link/);
  assert.equal(closeRefusal("superseded", "", TASK), null);
  assert.match(closeRefusal("duplicate", "not a link", TASK) ?? "", /not a task link/);
  assert.match(closeRefusal("superseded", TASK.toUpperCase(), TASK) ?? "", /different task/);
  assert.equal(closeRefusal("duplicate", TWIN, TASK), null);
});

test("the close body carries duplicate_of only where the reason takes one", () => {
  assert.deepEqual(closeBody("abandoned", TWIN), { reason: "abandoned" });
  assert.deepEqual(closeBody("duplicate", `#/threads/t/${TWIN}`), { reason: "duplicate", duplicate_of: TWIN });
  assert.deepEqual(closeBody("superseded", ""), { reason: "superseded" });
});

test("the server's refusals come back as calm sentences", () => {
  // A missing capability is left to the standard sentence, which names it.
  assert.equal(endRefusal("abandon", 403, "missing capability: thread:transition"), null);
  assert.match(endRefusal("cancel", 403, "only the member that created this thread may cancel it without thread:transition") ?? "", /created this task/);
  assert.match(endRefusal("abandon", 403, "only the thread's producer, its claim holder, or a maintainer can close it as abandoned") ?? "", /workspace admin/);
  assert.match(endRefusal("cancel", 409, "thread is already closed; there is nothing to cancel") ?? "", /already ended/);
  assert.match(endRefusal("abandon", 409, "only an open or in-review thread can be closed as abandoned; this one is closed") ?? "", /already ended/);
  assert.match(endRefusal("abandon", 409, "separation of duties: the claimer cannot land its own work on an owned thread; the owner or another member must perform this transition") ?? "", /someone else closes it/);
  assert.match(endRefusal("abandon", 409, "review requirement not met: 0 of 1 required approvals.") ?? "", /pass its review/);
  assert.match(endRefusal("abandon", 409, "land gate required: no pass recorded") ?? "", /pass its review/);
  // Naming the claim holder does not make a separation-of-duties refusal read as "not yours".
  assert.match(endRefusal("abandon", 409, "separation of duties: the claim holder may not close it") ?? "", /someone else closes it/);
  assert.match(endRefusal("abandon", 409, "something else") ?? "", /did not go through: something else/);
  assert.match(endRefusal("abandon", 400, "duplicate_of is not a thread you can read in this workspace") ?? "", /^Check the form/);
  assert.equal(endRefusal("cancel", 500, "boom"), null);
});

type Sent = { url: string; init: RequestInit };

function harness(creds: Partial<Credentials>, status: number, body: unknown) {
  const sent: Sent[] = [];
  const full: Credentials = { base: "https://console.test", pastedToken: "", tokenSession: false, sessionMemberId: ME, ...creds };
  const api = new EndTaskApi(
    async (url, init) => {
      sent.push({ url, init: init ?? {} });
      return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
    },
    () => full,
    () => (full.pastedToken ? "bearer" : "session"),
  );
  return { api, sent };
}

test("a signed-in session cancels through /ui/api, with the trimmed reason and an idempotency key", async () => {
  const { api, sent } = harness({}, 200, { thread: { id: TASK, state: "closed" }, replayed: false });
  const out = await api.cancel(TASK, "  nobody needs it  ");
  assert.equal(out.thread.state, "closed");
  assert.equal(sent.length, 1);
  assert.equal(sent[0].url, `https://console.test/ui/api/threads/${TASK}/cancel`);
  assert.equal(sent[0].init.method, "POST");
  assert.equal(sent[0].init.credentials, "include");
  assert.deepEqual(JSON.parse(String(sent[0].init.body)), { reason: "nobody needs it" });
  const headers = sent[0].init.headers as Record<string, string>;
  assert.ok(headers["Idempotency-Key"]);
  assert.equal(headers.Authorization, undefined);
});

test("a pasted token goes to the bearer route", async () => {
  const { api, sent } = harness({ pastedToken: "mdn_x" }, 200, { id: TASK, state: "closed" });
  await api.closeAbandoned(TASK, "duplicate", TWIN);
  assert.equal(sent[0].url, `https://console.test/threads/${TASK}/close-abandoned`);
  assert.equal((sent[0].init.headers as Record<string, string>).Authorization, "Bearer mdn_x");
  assert.deepEqual(JSON.parse(String(sent[0].init.body)), { reason: "duplicate", duplicate_of: TWIN });
});

test("a form the server would refuse is not sent", async () => {
  const { api, sent } = harness({}, 200, {});
  await assert.rejects(api.cancel(TASK, "  "), (e: unknown) => e instanceof EndTaskError && e.status === 400);
  await assert.rejects(api.closeAbandoned(TASK, "duplicate", ""), EndTaskError);
  assert.equal(sent.length, 0);
});

test("a refused close reads as the calm sentence, not a code", async () => {
  const { api } = harness({}, 409, { detail: "separation of duties: the claimer cannot land its own work on an owned thread; the owner or another member must perform this transition" });
  await assert.rejects(api.closeAbandoned(TASK, "abandoned"), (e: unknown) => e instanceof EndTaskError && e.status === 409 && /outside reviewer/.test(e.message));
});
