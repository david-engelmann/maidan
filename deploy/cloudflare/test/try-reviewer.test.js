import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { HELLO, MCP_PATH, RESULT, STEPS } from "../try/demo-agent.js";
import { REVIEW_STEPS, createDemoReviewer } from "../try/demo-reviewer.js";
import { REVIEW_SOLO, createTryPage } from "../try/try.js";
import { TRY_PATHS } from "../src/try-page.js";

// The try page's second agent against a stand-in for the sandbox's MCP
// endpoint: it reviews what the first handed off and closes the task, or
// leaves it for a person and says why.

const CHANNEL = "0193a000-0000-7000-8000-00000000c4a1";
const TASK = "0193a000-0000-7000-8000-0000000074a5";
const LEASE = "0193a000-0000-7000-8000-00000000ea5e";
const ROOT = "c22c992660fe43839a5650c95d6c9ce7db993a77baa85a521cf7e7a299e1dcdc";
const AGENT_TOKEN = "mdn_agent";
const REVIEWER_TOKEN = "mdn_reviewer";

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

const judgment = (id, reading, probability, extra = {}) => ({
  id,
  reading,
  probability,
  answer: probability >= 0.7 ? "yes" : probability <= 0.3 ? "no" : "unsettled",
  concern: false,
  ...extra,
});

const advice = (changes = {}) => ({
  thread_id: TASK,
  provider: "typesafe",
  model: "jev-1.14.0",
  land: "green",
  reasons: [],
  judgments: [judgment("done", "The task is done", 0.97), judgment("deletes_data", "Deletes no data", 0.02)],
  ...changes,
});

// What a healthy server answers to each tool either agent may call.
const healthy = {
  claim_next_thread: () => answer(thread({ claim_lease_id: LEASE })),
  acknowledge_claim: () => answer(thread({ claim_lease_id: LEASE, work_started_at: "2026-11-22T12:00:00Z" })),
  post_message: () => answer({ id: "m2", thread_id: TASK, body: HELLO }),
  set_thread_result: () => answer({ thread_id: TASK, result: RESULT }),
  get_review_packet: () => answer({ id: "p1", thread_id: TASK, evidence_root: ROOT, manifest: { artifacts: [] }, self_reported_only: true }),
  get_thread_result: () => answer({ thread_id: TASK, result: RESULT }),
  list_messages: () =>
    answer([
      { id: "m1", thread_id: TASK, body: "Welcome to Maidan." },
      { id: "m2", thread_id: TASK, body: HELLO },
    ]),
  advise_land_gate: () => answer(advice()),
  submit_review: (args) => answer({ thread_id: TASK, decision: args.decision, evidence_root: args.evidence_root }),
  transition_thread: (args) => answer(thread({ state: args.action === "close" ? "closed" : "in_review" })),
};

function server(tools = {}) {
  const calls = [];
  const fetch = async (url, init) => {
    if (url !== MCP_PATH) {
      calls.push({ url, init });
      return new Response(null, { status: 204 });
    }
    const body = JSON.parse(init.body);
    calls.push({ tool: body.params.name, args: body.params.arguments, token: init.headers.authorization, init });
    const handler = tools[body.params.name] ?? healthy[body.params.name];
    if (!handler) throw new Error(`unexpected tool ${body.params.name}`);
    return handler(body.params.arguments);
  };
  return { fetch, calls, tools: () => calls.filter((c) => c.tool).map((c) => c.tool) };
}

const reviewerOn = (srv) => createDemoReviewer({ fetch: srv.fetch, token: REVIEWER_TOKEN, taskId: TASK });

test("the reviewer reads the hand-off, holds the result against the task, reads the advice, approves that evidence and closes", async () => {
  const srv = server();
  const events = [];
  const outcome = await reviewerOn(srv).run((index, state, detail) => events.push([index, state, detail]));
  assert.deepEqual(outcome, { ok: true });
  assert.deepEqual(srv.tools(), ["get_review_packet", "get_thread_result", "list_messages", "advise_land_gate", "submit_review", "transition_thread"]);
  // One row a step, in the order the page lists them.
  assert.deepEqual(events.filter(([, state]) => state === "done").map(([index]) => index), REVIEW_STEPS.map((_, index) => index));

  const approve = srv.calls.find((c) => c.tool === "submit_review");
  assert.deepEqual(approve.args, { thread_id: TASK, decision: "approve", evidence_root: ROOT });
  assert.deepEqual(srv.calls.at(-1).args, { thread_id: TASK, action: "close" });
  for (const call of srv.calls) {
    assert.equal(call.token, `Bearer ${REVIEWER_TOKEN}`);
    assert.equal(call.init.credentials, "omit");
    assert.doesNotMatch(call.init.body, new RegExp(REVIEWER_TOKEN));
  }
  const said = events.filter(([, state]) => state === "done").map(([, , detail]) => detail);
  assert.match(said[1], /and that line is on the task\.$/);
  assert.equal(said[2], "Nothing flagged. All ten readings are clear.");
  assert.equal(said[4], "Closed. No step waited for a person.");
});

test("advice that flags something goes to a person: no approval, no close, and the reason", async () => {
  const flagged = advice({
    land: "amber",
    reasons: ["tests_not_reported"],
    judgments: [judgment("done", "The task is done", 0.97), judgment("tests_passed", "No passing tests are reported", 0.04, { concern: true })],
  });
  const srv = server({ advise_land_gate: () => answer(flagged) });
  const events = [];
  const reviewer = reviewerOn(srv);
  const outcome = await reviewer.run((index, state) => events.push([index, state]));
  assert.equal(outcome.ok, true);
  assert.match(outcome.left, /^The decision model’s advice reads “Worth a look”\. One of the two readings needs a look\.$/);
  assert.ok(!srv.tools().includes("submit_review"));
  assert.ok(!srv.tools().includes("transition_thread"));
  assert.deepEqual(events.at(-1), [2, "done"], "the step that read the advice is the last one shown");
  assert.equal(reviewer.finished(), false);
  // Asked again, it has not changed its mind and calls nothing more.
  const before = srv.calls.length;
  assert.equal((await reviewer.run()).left, outcome.left);
  assert.equal(srv.calls.length, before);
});

test("a result that is not on the task, or records no line, is left for a person before any advice is asked", async () => {
  for (const [tools, why] of [
    [{ get_thread_result: () => answer({ thread_id: TASK, result: { said: "something else" } }) }, /not on the task/],
    [{ get_thread_result: () => answer({ thread_id: TASK, result: { status: "done" } }) }, /does not record a line/],
  ]) {
    const srv = server(tools);
    const outcome = await reviewerOn(srv).run();
    assert.equal(outcome.ok, true);
    assert.match(outcome.left, why);
    assert.ok(!srv.tools().includes("advise_land_gate"));
    assert.ok(!srv.tools().includes("submit_review"));
  }
});

test("with no decision model answering, the review rests on its own check and still closes", async () => {
  const srv = server({ advise_land_gate: () => refused("resource not found", -32004) });
  const details = [];
  const outcome = await reviewerOn(srv).run((index, state, detail) => state === "done" && details.push(detail));
  assert.deepEqual(outcome, { ok: true });
  assert.equal(details[2], "No decision model answered, so the review rests on the check above.");
  assert.equal(srv.tools().at(-1), "transition_thread");
});

test("a refused approval stops there in the server's words, and a retry approves without reading again", async () => {
  let refuse = true;
  const srv = server({
    submit_review: (args) => (refuse ? refused("invalid params: the evidence changed since the hand-off") : healthy.submit_review(args)),
  });
  const reviewer = reviewerOn(srv);
  const first = await reviewer.run();
  assert.equal(first.ok, false);
  assert.equal(first.step, 3);
  assert.match(first.message, /^The server refused submit_review: invalid params: the evidence changed/);
  assert.ok(!srv.tools().includes("transition_thread"));
  refuse = false;
  assert.deepEqual(await reviewer.run(), { ok: true });
  assert.equal(srv.tools().filter((tool) => tool === "get_review_packet").length, 1);
  assert.equal(srv.tools().at(-1), "transition_thread");
});

test("a close the server does not confirm is an error, not a success", async () => {
  const srv = server({ transition_thread: () => answer(thread({ state: "in_review" })) });
  const outcome = await reviewerOn(srv).run();
  assert.equal(outcome.ok, false);
  assert.match(outcome.message, /The task is in review, not closed\./);
});

// A stand-in for the page's DOM, as in try-agent.test.js.
function element(id) {
  const names = new Set();
  return {
    id,
    hidden: ["turnstile", "ready", "agent-log", "agent-done", "review-log"].includes(id),
    disabled: false,
    textContent: "",
    dataset: {},
    children: [],
    classList: { toggle: (name, on) => (on ? names.add(name) : names.delete(name)), has: (name) => names.has(name) },
    appendChild(child) {
      this.children.push(child);
      return child;
    },
    replaceChildren() {
      this.children = [];
    },
    focus() {
      this.focused = true;
    },
    addEventListener() {},
  };
}

const PAGE_IDS = [
  "turnstile", "start", "start-status", "agent-token", "claude-cmd", "codex-config", "cursor-link", "vscode-link",
  "ready", "ready-title", "console", "console-status", "agent-run", "agent-status", "agent-log", "agent-done",
  "agent-console", "agent-console-status", "review-intro", "review-run", "review-status", "review-log",
];

function page({ tools = {}, reviewer = true } = {}) {
  const elements = new Map(PAGE_IDS.map((id) => [id, element(id)]));
  const doc = { getElementById: (id) => elements.get(id), createElement: () => element("") };
  const srv = server(tools);
  const minted = {
    workspace_id: "0193a000-0000-7000-8000-000000000005",
    channel_id: CHANNEL,
    task_id: TASK,
    agent: { token: AGENT_TOKEN, handle: "agent" },
    admin: { token: "mdn_admin", handle: "you" },
    ...(reviewer ? { reviewer: { token: REVIEWER_TOKEN, handle: "reviewer" } } : {}),
    mcp_url: "https://try.maidan.world/mcp/streamable",
  };
  const fetch = async (url, init) => {
    if (url === "/sandbox") return new Response(JSON.stringify(minted), { status: 201, headers: { "content-type": "application/json" } });
    return srv.fetch(url, init);
  };
  const app = createTryPage(doc, fetch, () => {}, { agentPauseMs: 0 });
  const el = (id) => elements.get(id);
  const rows = (log) =>
    el(log).children.map((li) => {
      const [head, note] = li.children;
      const [label, tool, mark] = head.children;
      return { state: li.dataset.state, label: label.textContent, tool: tool.textContent, mark: mark.textContent, detail: note.textContent };
    });
  return { app, el, srv, rows };
}

test("the page runs the first agent, then the second, each with its own token, and ends with the task closed", async () => {
  const ui = page();
  await ui.app.start();
  await ui.app.watchReviewer();
  assert.equal(ui.el("review-status").textContent, "Watch the first agent hand the task to review first.");
  assert.deepEqual(ui.srv.calls, []);

  await ui.app.watchAgent();
  assert.equal(ui.el("agent-done").hidden, false);
  assert.equal(ui.el("review-run").hidden, false);
  assert.equal(ui.el("review-run").focused, true, "the next thing to do is the second agent");
  assert.equal(ui.el("review-log").hidden, true);

  await ui.app.watchReviewer();
  const rows = ui.rows("review-log");
  assert.deepEqual(rows.map((r) => r.label), REVIEW_STEPS.map((s) => s.label));
  assert.deepEqual(rows.map((r) => r.tool), REVIEW_STEPS.map((s) => s.tool));
  assert.deepEqual(rows.map((r) => r.mark), Array(REVIEW_STEPS.length).fill("Done"));
  assert.equal(ui.rows("agent-log").length, STEPS.length, "the first agent's log is its own");
  assert.equal(ui.el("review-status").textContent, "Say hello is closed. Two agents did the work and the review, and no step waited for a person.");
  assert.equal(ui.el("review-status").classList.has("error"), false);
  assert.equal(ui.el("review-run").hidden, true);
  assert.equal(ui.el("agent-console").textContent, "See it in the console");
  assert.equal(ui.el("agent-console").focused, true);

  // The work is the first agent's and the review the second's: neither
  // token makes the other's calls, and the admin's makes none.
  const by = (token) => ui.srv.calls.filter((c) => c.token === `Bearer ${token}`).map((c) => c.tool);
  // (This stand-in page has no advice block, so the first agent asks for none.)
  assert.deepEqual(by(AGENT_TOKEN), STEPS.map((s) => s.tool));
  assert.deepEqual(by(REVIEWER_TOKEN), ["get_review_packet", "get_thread_result", "list_messages", "advise_land_gate", "submit_review", "transition_thread"]);
  assert.ok(!ui.srv.calls.some((c) => c.token === "Bearer mdn_admin"));
});

test("when the reviewer leaves it for a person, the page says why and hands over the console", async () => {
  const ui = page({ tools: { get_thread_result: () => answer({ thread_id: TASK, result: { said: "something else" } }) } });
  await ui.app.start();
  await ui.app.watchAgent();
  await ui.app.watchReviewer();
  assert.equal(ui.el("review-status").textContent, "The reviewer left this one for you. The result records a line that is not on the task.");
  assert.equal(ui.el("review-run").hidden, true);
  assert.equal(ui.el("agent-console").textContent, "");
  assert.equal(ui.el("agent-console").focused, true);
  assert.equal(ui.rows("review-log").length, 2);
});

test("a failed review call shows on its step, and Try again finishes the review", async () => {
  let refuse = true;
  const ui = page({ tools: { submit_review: (args) => (refuse ? new Response("", { status: 429 }) : healthy.submit_review(args)) } });
  await ui.app.start();
  await ui.app.watchAgent();
  await ui.app.watchReviewer();
  assert.match(ui.el("review-status").textContent, /^The reviewer stopped at step 4 of 5\. The demo is rate limiting these calls/);
  assert.equal(ui.el("review-status").classList.has("error"), true);
  assert.equal(ui.el("review-run").textContent, "Try again");
  assert.equal(ui.el("review-run").disabled, false);
  refuse = false;
  await ui.app.watchReviewer();
  assert.deepEqual(ui.rows("review-log").map((r) => r.state), Array(REVIEW_STEPS.length).fill("done"));
  assert.equal(ui.el("review-run").hidden, true);
});

test("a sandbox with no second agent leaves the review to the visitor, as before", async () => {
  const ui = page({ reviewer: false });
  await ui.app.start();
  assert.equal(ui.el("review-run").hidden, true);
  assert.equal(ui.el("review-intro").textContent, REVIEW_SOLO);
  await ui.app.watchAgent();
  assert.equal(ui.el("agent-console").focused, true);
  await ui.app.watchReviewer();
  assert.ok(!ui.srv.tools().includes("submit_review"));
});

test("the try page offers the second agent beside the console, inside the step that follows the first", () => {
  const html = readFileSync(new URL("../try/index.html", import.meta.url), "utf8");
  const done = html.slice(html.indexOf('id="agent-done"'), html.indexOf('<p class="or">'));
  assert.match(done, /<button type="button" id="review-run" class="cta">Let a second agent review it<\/button>\s*<button type="button" id="agent-console" class="cta ghost">Review it yourself in the console<\/button>/);
  assert.match(done, /<ol id="review-log" class="steps" aria-label="What the reviewing agent did" hidden><\/ol>/);
  assert.match(done, /<p id="review-status" class="status" role="status" aria-live="polite"><\/p>/);
  // Nothing on the page says the review has to be a person's.
  assert.doesNotMatch(html, /so it waits for you|Closing the work stays yours|Watch it, and approve/);
});

test("the second agent's file is served with the page, calls this origin only, and loads only the first agent's", () => {
  assert.ok(TRY_PATHS.has("/demo-reviewer.js"));
  const source = readFileSync(new URL("../try/demo-reviewer.js", import.meta.url), "utf8");
  assert.doesNotMatch(source, /https?:\/\//, "no absolute URL to call");
  assert.deepEqual([...source.matchAll(/^import .* from "([^"]+)";$/gm)].map((m) => m[1]), ["./demo-agent.js"]);
  assert.doesNotMatch(source, /localStorage|sessionStorage|document\.cookie/);
  const page = readFileSync(new URL("../try/try.js", import.meta.url), "utf8");
  assert.match(page, /from "\.\/demo-reviewer\.js";/);
});
