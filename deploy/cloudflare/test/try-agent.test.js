import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { ADVICE_ALL_CLEAR, ADVICE_TITLE, HELLO, MCP_PATH, RESULT, STEPS, createDemoAgent, readAdvice } from "../try/demo-agent.js";
import { START_RETRY_SECS, createTryPage } from "../try/try.js";

// The try page's demo agent against a stand-in for the sandbox's MCP
// endpoint. The stand-in answers the way crates/maidan-mcp does: a tool's
// answer is JSON in result.content[0].text, a refusal is a JSON-RPC error.

const CHANNEL = "0193a000-0000-7000-8000-00000000c4a1";
const TASK = "0193a000-0000-7000-8000-0000000074a5";
const LEASE = "0193a000-0000-7000-8000-00000000ea5e";
const AGENT_TOKEN = "mdn_agent";

const minted = {
  workspace_id: "0193a000-0000-7000-8000-000000000005",
  channel_id: CHANNEL,
  task_id: TASK,
  agent: { token: AGENT_TOKEN, handle: "agent" },
  admin: { token: "mdn_admin", handle: "you" },
  mcp_url: "https://try.maidan.world/mcp/streamable",
};

function answer(value) {
  return new Response(
    JSON.stringify({ jsonrpc: "2.0", id: 1, result: { content: [{ type: "text", text: JSON.stringify(value) }], isError: false } }),
    { status: 200, headers: { "content-type": "application/json" } },
  );
}

function refused(message, code = -32602) {
  return new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, error: { code, message } }), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

const thread = (extra = {}) => ({ id: TASK, channel_id: CHANNEL, title: "Say hello", state: "open", ...extra });

// What a healthy server answers to each tool the agent may call.
const healthy = {
  claim_next_thread: () => answer({ ...thread({ assignee_id: "agent-id", claim_lease_id: LEASE }), pin: { uri: "maidan://x", content_hash: "h" } }),
  acknowledge_claim: () => answer(thread({ claim_lease_id: LEASE, work_started_at: "2026-11-16T12:00:00Z" })),
  post_message: () => answer({ id: "m1", thread_id: TASK, body: HELLO }),
  set_thread_result: () => answer({ thread_id: TASK, result: RESULT }),
  transition_thread: () => answer(thread({ state: "in_review" })),
  release_claim: () => answer(thread()),
};

// A fetch that records every MCP call and answers from `tools` (falling back
// to the healthy server). Any other URL is the console sign-in.
function server(tools = {}) {
  const calls = [];
  const fetch = async (url, init) => {
    if (url !== MCP_PATH) {
      calls.push({ url, init });
      return new Response(null, { status: 204 });
    }
    const body = JSON.parse(init.body);
    calls.push({ url, init, body, tool: body.params.name, args: body.params.arguments });
    const handler = tools[body.params.name] ?? healthy[body.params.name];
    if (!handler) throw new Error(`unexpected tool ${body.params.name}`);
    return handler(body.params.arguments, calls);
  };
  const tools_ = () => calls.filter((c) => c.tool).map((c) => c.tool);
  return { fetch, calls, tools: tools_ };
}

function agentOn(srv) {
  return createDemoAgent({ fetch: srv.fetch, token: AGENT_TOKEN, channelId: CHANNEL, taskId: TASK, retrySecs: START_RETRY_SECS });
}

function recorder() {
  const events = [];
  return { events, onStep: (index, state, detail) => events.push([index, state, detail]) };
}

test("the agent claims Say hello, acknowledges, says hello, sets a result and sends it to review", async () => {
  const srv = server();
  const { events, onStep } = recorder();
  const outcome = await agentOn(srv).run(onStep);
  assert.deepEqual(outcome, { ok: true });
  assert.deepEqual(srv.tools(), STEPS.map((s) => s.tool));
  assert.deepEqual(srv.tools(), ["claim_next_thread", "acknowledge_claim", "post_message", "set_thread_result", "transition_thread"]);

  const [claim, ack, post, result, review] = srv.calls;
  assert.deepEqual(claim.args, { channel_id: CHANNEL });
  assert.deepEqual(ack.args, { thread_id: TASK, claim_lease_id: LEASE });
  assert.deepEqual(post.args, { thread_id: TASK, body: HELLO });
  assert.deepEqual(result.args, { thread_id: TASK, claim_lease_id: LEASE, result: { said: "hello" } });
  assert.deepEqual(review.args, { thread_id: TASK, action: "start_review" });

  // Each step is shown starting, then done with what the server answered.
  assert.deepEqual(
    events.map(([i, state]) => `${i}:${state}`),
    ["0:running", "0:done", "1:running", "1:done", "2:running", "2:done", "3:running", "3:done", "4:running", "4:done"],
  );
  assert.match(events[1][2], /Took “Say hello”/);
  assert.match(events[3][2], /working clock started/);
  assert.match(events[9][2], /in review/);
});

test("every call is a JSON-RPC tools/call to this origin's MCP endpoint, with the agent token in a header only", async () => {
  const srv = server();
  await agentOn(srv).run();
  assert.equal(MCP_PATH, "/mcp/streamable");
  const ids = new Set();
  for (const call of srv.calls) {
    assert.equal(call.url, "/mcp/streamable");
    assert.equal(call.init.method, "POST");
    assert.equal(call.init.headers.authorization, `Bearer ${AGENT_TOKEN}`);
    assert.equal(call.init.headers.accept, "application/json");
    assert.equal(call.init.headers["content-type"], "application/json");
    // The console's admin session cookie never rides along.
    assert.equal(call.init.credentials, "omit");
    assert.equal(call.body.jsonrpc, "2.0");
    assert.equal(call.body.method, "tools/call");
    ids.add(call.body.id);
    assert.doesNotMatch(call.init.body, new RegExp(AGENT_TOKEN));
  }
  assert.equal(ids.size, srv.calls.length, "each call has its own id");
});

test("the agent never closes or archives the task: the person does", async () => {
  const srv = server();
  await agentOn(srv).run();
  const transitions = srv.calls.filter((c) => c.tool === "transition_thread");
  assert.deepEqual(transitions.map((c) => c.args.action), ["start_review"]);
  const source = readFileSync(new URL("../try/demo-agent.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /action:\s*"(close|archive)"/);
  assert.doesNotMatch(source, /"(close|archive)"/);
});

test("a refused call stops the run there, shows the server's words, and a retry resumes without claiming again", async () => {
  let refuse = true;
  const srv = server({
    set_thread_result: () => (refuse ? refused("claim_lost: the lease lapsed", -32012) : healthy.set_thread_result()),
  });
  const agent = agentOn(srv);
  const { events, onStep } = recorder();
  const first = await agent.run(onStep);
  assert.equal(first.ok, false);
  assert.equal(first.step, 3);
  assert.equal(first.message, "The server refused set_thread_result: claim_lost: the lease lapsed");
  assert.deepEqual(events.at(-1), [3, "failed", first.message]);
  assert.ok(!srv.tools().includes("transition_thread"), "nothing after the refused call ran");
  assert.equal(agent.finished(), false);

  refuse = false;
  const second = await agent.run(onStep);
  assert.deepEqual(second, { ok: true });
  assert.deepEqual(srv.tools(), [
    "claim_next_thread",
    "acknowledge_claim",
    "post_message",
    "set_thread_result",
    "set_thread_result",
    "transition_thread",
  ]);
  assert.equal(agent.finished(), true);
});

test("HTTP refusals, a dropped connection and a tool error each read as what happened", async () => {
  async function failWith(response) {
    const srv = server({ claim_next_thread: response });
    return (await agentOn(srv).run()).message;
  }
  assert.match(await failWith(() => new Response("", { status: 401 })), /refused the agent’s token/);
  assert.match(await failWith(() => new Response("", { status: 429 })), /rate limiting these calls/);
  assert.equal(
    await failWith(() => new Response("", { status: 503, headers: { "retry-after": "20" } })),
    `The demo is starting or being updated. Try again in about ${START_RETRY_SECS} seconds.`,
  );
  assert.equal(
    await failWith(
      () =>
        new Response(JSON.stringify({ detail: "missing workspace:read capability" }), {
          status: 403,
          headers: { "content-type": "application/problem+json" },
        }),
    ),
    "The server refused the call (403): missing workspace:read capability",
  );
  assert.equal(await failWith(() => new Response("boom", { status: 500 })), "The server failed the call (500). Try again.");
  assert.equal(
    await failWith(() => {
      throw new TypeError("offline");
    }),
    "Could not reach the demo. Check your connection and try again.",
  );
  assert.equal(
    await failWith(
      () =>
        new Response(JSON.stringify({ jsonrpc: "2.0", id: 1, result: { content: [{ type: "text", text: "free allowance used up" }], isError: true } }), {
          status: 200,
        }),
    ),
    "The server refused claim_next_thread: free allowance used up",
  );
  assert.equal(await failWith(() => new Response("<html>", { status: 200 })), "The server’s answer to claim_next_thread was not JSON.");
});

test("a claim that finds nothing says the task is already taken, and changes nothing", async () => {
  const srv = server({ claim_next_thread: () => answer(null) });
  const outcome = await agentOn(srv).run();
  assert.equal(outcome.ok, false);
  assert.equal(outcome.step, 0);
  assert.match(outcome.message, /already taken, perhaps by an agent you connected/);
  assert.deepEqual(srv.tools(), ["claim_next_thread"]);
});

test("a claim that takes some other task gives it back rather than holding it", async () => {
  const other = { id: "0193a000-0000-7000-8000-0000000000ff", title: "Something else", state: "open", claim_lease_id: "lease-2" };
  const srv = server({ claim_next_thread: () => answer(other) });
  const outcome = await agentOn(srv).run();
  assert.equal(outcome.ok, false);
  assert.equal(outcome.message, "The claim took “Something else”, not Say hello, so the agent gave it back.");
  assert.deepEqual(srv.tools(), ["claim_next_thread", "release_claim"]);
  assert.deepEqual(srv.calls[1].args, { thread_id: other.id, claim_lease_id: "lease-2" });

  const stuck = server({ claim_next_thread: () => answer(other), release_claim: () => refused("not yours") });
  assert.match((await agentOn(stuck).run()).message, /could not give it back\. Its lease lapses on its own\./);
});

test("a review the server does not confirm is an error, not a success", async () => {
  const srv = server({ transition_thread: () => answer(thread({ state: "open" })) });
  const outcome = await agentOn(srv).run();
  assert.equal(outcome.ok, false);
  assert.equal(outcome.step, 4);
  assert.equal(outcome.message, "The task is open, not in review.");
});

test("the agent waits between calls, and only after each call has answered", async () => {
  const srv = server();
  const order = [];
  const agent = createDemoAgent({
    fetch: async (url, init) => {
      order.push(`call:${JSON.parse(init.body).params.name}`);
      return srv.fetch(url, init);
    },
    token: AGENT_TOKEN,
    channelId: CHANNEL,
    taskId: TASK,
    pauseMs: 500,
    sleep: async (ms) => order.push(`pause:${ms}`),
  });
  await agent.run();
  assert.deepEqual(order, [
    "call:claim_next_thread",
    "pause:500",
    "call:acknowledge_claim",
    "pause:500",
    "call:post_message",
    "pause:500",
    "call:set_thread_result",
    "pause:500",
    "call:transition_thread",
  ]);
});

// A small stand-in for the page's DOM: enough for createTryPage to build the
// step log and toggle what it shows.
function element(id, tag = "div") {
  const names = new Set();
  return {
    id,
    tag,
    hidden: ["turnstile", "ready", "agent-log", "agent-done"].includes(id),
    disabled: false,
    textContent: "",
    className: "",
    href: "",
    dataset: {},
    children: [],
    classList: {
      toggle(name, on) {
        if (on) names.add(name);
        else names.delete(name);
      },
      add: (name) => names.add(name),
      remove: (name) => names.delete(name),
      has: (name) => names.has(name),
    },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    focus() {
      this.focused = true;
    },
    addEventListener(_type, fn) {
      this.listeners ??= [];
      this.listeners.push(fn);
    },
  };
}

const PAGE_IDS = [
  "turnstile",
  "start",
  "start-status",
  "agent-token",
  "claude-cmd",
  "codex-config",
  "cursor-link",
  "vscode-link",
  "ready",
  "ready-title",
  "console",
  "console-status",
  "agent-run",
  "agent-status",
  "agent-log",
  "agent-done",
  "agent-console",
  "agent-console-status",
];

// The advice block's elements, which a page from before it does not have.
const ADVICE_IDS = ["agent-advice", "advice-model", "advice-title", "advice-why", "advice-flagged", "advice-rows"];

function page(tools, ids = PAGE_IDS) {
  const elements = new Map(ids.map((id) => [id, element(id)]));
  if (elements.has("agent-advice")) elements.get("agent-advice").hidden = true;
  const doc = {
    getElementById: (id) => elements.get(id),
    createElement: (tag) => element("", tag),
  };
  const srv = server(tools);
  const fetch = async (url, init) => {
    if (url === "/sandbox") {
      return new Response(JSON.stringify(minted), { status: 201, headers: { "content-type": "application/json" } });
    }
    return srv.fetch(url, init);
  };
  const assigned = [];
  const app = createTryPage(doc, fetch, (href) => assigned.push(href), { agentPauseMs: 0 });
  const el = (id) => elements.get(id);
  // Each step row: its label, the tool, its state word and its detail.
  const rows = () =>
    el("agent-log").children.map((li) => {
      const [head, note] = li.children;
      const [label, tool, mark] = head.children;
      return { state: li.dataset.state, label: label.textContent, tool: tool.textContent, mark: mark.textContent, detail: note.textContent };
    });
  return { app, el, srv, rows, assigned };
}

test("the page asks for a sandbox before it runs the agent", async () => {
  const ui = page();
  await ui.app.watchAgent();
  assert.equal(ui.el("agent-status").textContent, "Start a sandbox first.");
  assert.equal(ui.el("agent-status").classList.has("error"), true);
  assert.deepEqual(ui.srv.calls, []);
});

test("the page shows each step as the agent works, then hands the person the console", async () => {
  const ui = page();
  await ui.app.start();
  assert.equal(ui.el("agent-log").hidden, true, "the log stays hidden until the agent starts");
  await ui.app.watchAgent();

  const rows = ui.rows();
  assert.deepEqual(rows.map((r) => r.label), STEPS.map((s) => s.label));
  assert.deepEqual(rows.map((r) => r.tool), STEPS.map((s) => s.tool));
  assert.deepEqual(rows.map((r) => r.state), Array(STEPS.length).fill("done"));
  assert.deepEqual(rows.map((r) => r.mark), Array(STEPS.length).fill("Done"));
  assert.match(rows[0].detail, /Took “Say hello”/);
  assert.equal(ui.el("agent-log").hidden, false);

  assert.equal(ui.el("agent-status").textContent, "The agent sent Say hello to review.");
  assert.equal(ui.el("agent-status").classList.has("error"), false);
  assert.equal(ui.el("agent-run").hidden, true);
  assert.equal(ui.el("agent-done").hidden, false);
  assert.equal(ui.el("agent-console").focused, true, "focus moves to the next thing to do");
  assert.ok(!ui.srv.calls.some((c) => c.tool === "transition_thread" && c.args.action !== "start_review"));

  // The agent's calls used its own token; the console uses the admin's.
  await ui.app.openConsole(ui.el("agent-console"), ui.el("agent-console-status"));
  const signIn = ui.srv.calls.find((c) => c.url === "/auth/session/from-token");
  assert.equal(signIn.init.headers.authorization, "Bearer mdn_admin");
  assert.deepEqual(ui.assigned, ["/ui/"]);
  assert.equal(ui.el("agent-console-status").textContent, "Signing you in…");
});

test("the page shows a failed call as an error on its step, and Try again finishes the run", async () => {
  let refuse = true;
  const ui = page({
    acknowledge_claim: () => (refuse ? new Response("", { status: 429 }) : healthy.acknowledge_claim()),
  });
  await ui.app.start();
  await ui.app.watchAgent();

  let rows = ui.rows();
  assert.equal(rows.length, 2, "steps after the failure are not shown as started");
  assert.equal(rows[0].state, "done");
  assert.equal(rows[1].state, "failed");
  assert.equal(rows[1].mark, "Failed");
  assert.match(rows[1].detail, /rate limiting these calls/);
  const status = ui.el("agent-status");
  assert.equal(status.classList.has("error"), true);
  assert.match(status.textContent, /^The agent stopped at step 2 of 5\. The demo is rate limiting these calls/);
  assert.equal(ui.el("agent-run").disabled, false);
  assert.equal(ui.el("agent-run").textContent, "Try again");
  assert.equal(ui.el("agent-run").focused, true, "focus returns to the button that retries");
  assert.equal(ui.el("agent-done").hidden, true);

  refuse = false;
  await ui.app.watchAgent();
  rows = ui.rows();
  assert.equal(rows.length, STEPS.length, "the failed step's row is reused");
  assert.deepEqual(rows.map((r) => r.state), Array(STEPS.length).fill("done"));
  assert.equal(rows[1].detail, "Its working clock started.");
  assert.equal(ui.el("agent-done").hidden, false);
  assert.equal(ui.srv.tools().filter((t) => t === "claim_next_thread").length, 1);
});

// What the server answers for the hand-off the demo agent makes: the numbers
// Jev 1.14.0 gave for it on 2026-11-20, in the shape of
// crates/maidan-server/src/land_gate_advisor.rs.
const judgment = (id, reading, probability, extra = {}) => ({
  id,
  label: id,
  probability,
  answer: probability >= 0.7 ? "yes" : probability <= 0.3 ? "no" : "unsettled",
  reading,
  concern: false,
  ...extra,
});
const helloAdvice = (changes = {}) => ({
  provider: "typesafe",
  model: "jev-1.14.0",
  land: "green",
  reasons: [],
  judgments: [
    judgment("done", "Does what the task asks", 0.93),
    judgment("tests_passed", "Reports no passing tests, which this task does not call for", 0.01),
    judgment("code_change", "The task asks for no code change", 0.04),
    judgment("unresolved_failure", "Reports nothing still failing", 0.02),
    judgment("left_undone", "Reports nothing left undone", 0.02),
    judgment("asks_person", "Waits on no question", 0.02),
    judgment("beyond_task", "Stays within the task", 0.02),
    judgment("changes_access", "Leaves sign-in and permissions alone", 0.01),
    judgment("deletes_data", "Deletes no stored data", 0.01),
    judgment("flags_risk", "The agent flags no risk", 0.01),
  ],
  thresholds: { yes_min: 0.7, no_max: 0.3 },
  key_source: "instance",
  cached: false,
  ...changes,
});

test("advice reads as a title, the model, and each judgment as a statement with how sure the model is", () => {
  const advice = readAdvice(helloAdvice());
  assert.equal(advice.land, "green");
  assert.equal(advice.title, "Nothing flagged");
  assert.equal(advice.model, "Jev 1.14.0 by TypeSafe");
  assert.equal(advice.why, ADVICE_ALL_CLEAR);
  assert.equal(advice.rows.length, 10);
  assert.deepEqual(advice.rows[0], { reading: "Does what the task asks", sure: "93% sure", mark: "clear" });
  // 1% yes is a no the model is 99% sure of.
  assert.deepEqual(advice.rows[1], { reading: "Reports no passing tests, which this task does not call for", sure: "99% sure", mark: "clear" });
  assert.ok(advice.rows.every((row) => row.mark === "clear"));
});

test("a concern and an unsettled judgment are marked, counted, and never shown as a percentage they are not", () => {
  const answer = helloAdvice({ land: "amber", reasons: ["not_shown_done", "tests_not_reported"] });
  answer.judgments[0] = judgment("done", "Unclear whether it does what the task asks", 0.5, { concern: true });
  answer.judgments[1] = judgment("tests_passed", "Reports no passing tests", 0.004, { concern: true });
  const advice = readAdvice(answer);
  assert.equal(advice.title, "Worth a look");
  assert.deepEqual(advice.rows[0], { reading: "Unclear whether it does what the task asks", sure: "unsure", mark: "unsure" });
  assert.deepEqual(advice.rows[1], { reading: "Reports no passing tests", sure: "99% sure", mark: "concern" });
  assert.equal(advice.why, "Two of the ten readings need a look.");
  const one = helloAdvice({ land: "red" });
  one.judgments[3] = judgment("unresolved_failure", "Reports something still failing", 0.99, { concern: true });
  assert.equal(readAdvice(one).why, "One of the ten readings needs a look.");
  assert.equal(readAdvice(one).title, "Problem reported");
});

test("an answer that is not advice shows nothing", () => {
  assert.equal(readAdvice(null), null);
  assert.equal(readAdvice({}), null);
  assert.equal(readAdvice(helloAdvice({ land: "blue" })), null);
  assert.equal(readAdvice(helloAdvice({ land: "toString" })), null);
  assert.equal(readAdvice(helloAdvice({ judgments: [] })), null);
  assert.equal(readAdvice(helloAdvice({ judgments: [{ id: "done", probability: 0.9, answer: "yes" }] })), null);
  assert.equal(readAdvice(helloAdvice({ judgments: [{ reading: "Does what the task asks", probability: "0.9", answer: "yes" }] })), null);
  assert.equal(readAdvice(helloAdvice({ model: 7 })), null);
});

test("the page's words for the advice are the console's", () => {
  const consoleWords = readFileSync(new URL("../../../web/console/src/core/advice.ts", import.meta.url), "utf8");
  for (const [land, title] of Object.entries(ADVICE_TITLE)) {
    assert.match(consoleWords, new RegExp(`${land}: "${title}"`), land);
  }
  assert.ok(consoleWords.includes(`ALL_CLEAR = "${ADVICE_ALL_CLEAR}"`));
});

test("the agent asks for advice only once the task is in review, with its own token, and a refusal is no advice", async () => {
  const srv = server({ advise_land_gate: () => answer(helloAdvice()) });
  const agent = agentOn(srv);
  assert.equal(await agent.advice(), null);
  assert.deepEqual(srv.tools(), [], "nothing is asked before the hand-off exists");
  await agent.run();
  const advice = await agent.advice();
  assert.equal(advice.title, "Nothing flagged");
  const call = srv.calls.at(-1);
  assert.equal(call.tool, "advise_land_gate");
  assert.deepEqual(call.args, { thread_id: TASK });
  assert.equal(call.init.headers.authorization, `Bearer ${AGENT_TOKEN}`);

  // Advice is off on this server (NotFound), the model did not answer, or
  // the limit was reached: each is no advice, and none is an error.
  for (const refusal of [
    () => refused("not found", -32004),
    () => new Response("", { status: 502 }),
    () => new Response("", { status: 429 }),
    () => answer({ land: "green" }),
  ]) {
    const off = server({ advise_land_gate: refusal });
    const other = agentOn(off);
    await other.run();
    assert.equal(await other.advice(), null);
  }
});

test("the page shows the advice with the next step, each judgment a row, and says so once", async () => {
  const ui = page({ advise_land_gate: () => answer(helloAdvice()) }, [...PAGE_IDS, ...ADVICE_IDS]);
  await ui.app.start();
  await ui.app.watchAgent();
  const box = ui.el("agent-advice");
  assert.equal(box.hidden, false);
  assert.equal(box.dataset.land, "green");
  assert.equal(ui.el("advice-title").textContent, "Nothing flagged");
  assert.equal(ui.el("advice-model").textContent, "Jev 1.14.0 by TypeSafe");
  assert.equal(ui.el("advice-why").textContent, ADVICE_ALL_CLEAR);
  const rows = ui.el("advice-rows").children;
  assert.equal(rows.length, 10);
  const [mark, word, reading, sure] = rows[0].children;
  assert.equal(rows[0].dataset.mark, "clear");
  assert.equal(mark.ariaHidden, "true");
  assert.equal(word.textContent, "");
  assert.equal(reading.textContent, "Does what the task asks");
  assert.equal(sure.textContent, "93% sure");
  assert.equal(ui.el("agent-status").textContent, "The agent sent Say hello to review. A decision model read the hand-off: nothing flagged.");
  // Nothing was flagged, so only the ten readings are there, behind their press.
  assert.equal(ui.el("advice-flagged").hidden, true);
  assert.equal(ui.el("advice-flagged").children.length, 0);
  assert.equal(ui.el("agent-done").hidden, false);
  assert.equal(ui.el("agent-console").focused, true);
  assert.equal(ui.srv.tools().filter((t) => t === "advise_land_gate").length, 1);
});

test("a marked row carries the word a reader cannot see in its shape", async () => {
  const answerWith = helloAdvice({ land: "amber", reasons: ["tests_not_reported"] });
  answerWith.judgments[1] = judgment("tests_passed", "Reports no passing tests", 0.01, { concern: true });
  answerWith.judgments[0] = judgment("done", "Unclear whether it does what the task asks", 0.5, { concern: true });
  // Unsettled, but the rules did not need it: no mark.
  answerWith.judgments[2] = judgment("code_change", "Unclear whether the task asks for a code change", 0.5);
  const ui = page({ advise_land_gate: () => answer(answerWith) }, [...PAGE_IDS, ...ADVICE_IDS]);
  await ui.app.start();
  await ui.app.watchAgent();
  const rows = ui.el("advice-rows").children;
  assert.equal(rows[1].dataset.mark, "concern");
  assert.equal(rows[1].children[1].textContent, "Flagged: ");
  assert.equal(rows[0].dataset.mark, "unsure");
  assert.equal(rows[0].children[1].textContent, "Unsure: ");
  assert.equal(rows[2].dataset.mark, "clear");
  assert.equal(rows[2].children[1].textContent, "");
  assert.equal(rows[2].children[3].textContent, "unsure");
  assert.equal(ui.el("advice-why").textContent, "Two of the ten readings need a look.");
  // What was flagged shows at once, above the ten readings.
  const flagged = ui.el("advice-flagged");
  assert.equal(flagged.hidden, false);
  assert.deepEqual(flagged.children.map((li) => [li.dataset.mark, li.children[2].textContent]), [
    ["unsure", "Unclear whether it does what the task asks"],
    ["concern", "Reports no passing tests"],
  ]);
  assert.equal(ui.el("agent-advice").dataset.land, "amber");
});

test("without advice the page goes on exactly as before", async () => {
  const ui = page({ advise_land_gate: () => refused("not found", -32004) }, [...PAGE_IDS, ...ADVICE_IDS]);
  await ui.app.start();
  await ui.app.watchAgent();
  assert.equal(ui.el("agent-advice").hidden, true);
  assert.equal(ui.el("advice-rows").children.length, 0);
  assert.equal(ui.el("agent-status").textContent, "The agent sent Say hello to review.");
  assert.equal(ui.el("agent-done").hidden, false);
  assert.equal(ui.el("agent-console").focused, true);
});

test("the try page has the advice block, hidden, inside the step that follows the agent", () => {
  const html = readFileSync(new URL("../try/index.html", import.meta.url), "utf8");
  const done = html.slice(html.indexOf('id="agent-done"'), html.indexOf('id="agent-console"'));
  assert.match(done, /<div id="agent-advice" class="advice" role="group" aria-labelledby="advice-label" hidden>/);
  for (const id of ADVICE_IDS) assert.ok(done.includes(`id="${id}"`), id);
  assert.match(done, /It is advice, and it is on the task in the console too\./);
  // The ten readings wait behind one press; what was flagged does not.
  assert.match(done, /<ul id="advice-flagged"[^>]*><\/ul>\s*<details class="advice-all">\s*<summary>All ten readings<\/summary>\s*<ul id="advice-rows"/);
});
