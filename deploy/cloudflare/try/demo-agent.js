// The try page's demo agent: a scripted agent that runs in the visitor's
// browser with their sandbox's own agent token, for a visitor who has no MCP
// client to connect. It walks the loop the Say hello task's opening message
// asks of an agent (crates/maidan-server/src/oidc/handlers.rs), over the MCP
// endpoint a real client uses, and every line it shows comes from the
// server's answer. Pure apart from `fetch`, so the Worker's tests drive it
// without a browser (test/try-agent.test.js).
//
// It stops at review. The server refuses an agent closing work it did
// (separation of duties), so the hand-off to the person is the point of the
// demo, not a gap in it.

// Relative, not the sandbox's `mcp_url`: the page's CSP is connect-src 'self',
// and a relative path cannot drift to another origin.
export const MCP_PATH = "/mcp/streamable";

// What the agent posts and records. The message says what posted it, so the
// person reviewing in the console is not misled into thinking their own agent
// ran.
export const HELLO = "Hello from the demo agent the try page runs in your browser.";
export const RESULT = { said: "hello" };

// The steps in order, one MCP call each (the claim makes a second only to
// give back a task that is not Say hello). `tool` is shown beside each step
// so a visitor sees the calls their own agent would make.
export const STEPS = [
  { tool: "claim_next_thread", label: "Claim the Say hello task" },
  { tool: "acknowledge_claim", label: "Acknowledge the claim" },
  { tool: "post_message", label: "Post a hello on the task" },
  { tool: "set_thread_result", label: "Set the task’s result" },
  { tool: "transition_thread", label: "Send it to review" },
];

export class AgentError extends Error {}

async function problemDetail(res) {
  try {
    const body = await res.json();
    return typeof body?.detail === "string" ? body.detail.trim() : "";
  } catch {
    return "";
  }
}

async function httpRefusal(res, retrySecs) {
  const { status } = res;
  if (status === 401) return "The server refused the agent’s token. It expires after 24 hours: start a new sandbox.";
  if (status === 429) return "The demo is rate limiting these calls. Wait a minute and try again.";
  if (status === 503) return `The demo is starting or being updated. Try again in about ${retrySecs} seconds.`;
  const detail = await problemDetail(res);
  if (status >= 500) return `The server failed the call (${status}). Try again.`;
  return detail ? `The server refused the call (${status}): ${detail}` : `The server refused the call (${status}).`;
}

function quoted(title) {
  return `“${typeof title === "string" && title ? title : "untitled"}”`;
}

// `pauseMs` spaces the calls out so a person can follow them; each call
// still runs only after the one before it answered.
export function createDemoAgent({ fetch, token, channelId, taskId, retrySecs = 20, pauseMs = 0, sleep } = {}) {
  const wait = sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  let rpcId = 0;
  // The step a retry resumes at, so a failure after the claim does not ask
  // for the claim again: the task is already the agent's, and a second claim
  // would find nothing.
  let next = 0;
  let leaseId = "";
  let running = false;

  async function call(name, args) {
    rpcId += 1;
    let res;
    try {
      res = await fetch(MCP_PATH, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          accept: "application/json",
          authorization: `Bearer ${token}`,
        },
        body: JSON.stringify({ jsonrpc: "2.0", id: rpcId, method: "tools/call", params: { name, arguments: args } }),
        cache: "no-store",
        // The console's session cookie, if the visitor signed in first, is
        // the admin's: the agent's calls carry the agent's token and nothing
        // else.
        credentials: "omit",
      });
    } catch {
      throw new AgentError("Could not reach the demo. Check your connection and try again.");
    }
    if (!res.ok) throw new AgentError(await httpRefusal(res, retrySecs));
    let body;
    try {
      body = await res.json();
    } catch {
      throw new AgentError(`The server’s answer to ${name} was not JSON.`);
    }
    if (body?.error) {
      const why = typeof body.error.message === "string" ? body.error.message.trim() : "";
      throw new AgentError(why ? `The server refused ${name}: ${why}` : `The server refused ${name}.`);
    }
    const result = body?.result;
    const text = Array.isArray(result?.content) ? result.content.find((part) => part?.type === "text")?.text : undefined;
    if (result?.isError) {
      throw new AgentError(text ? `The server refused ${name}: ${text}` : `The server refused ${name}.`);
    }
    if (typeof text !== "string") throw new AgentError(`The server’s answer to ${name} had no content.`);
    try {
      return JSON.parse(text);
    } catch {
      throw new AgentError(`The server’s answer to ${name} was not JSON.`);
    }
  }

  async function claim() {
    const thread = await call("claim_next_thread", { channel_id: channelId });
    if (thread === null) {
      throw new AgentError(
        "Nothing in #general was free to claim. The Say hello task is already taken, perhaps by an agent you connected.",
      );
    }
    if (typeof thread?.claim_lease_id !== "string" || !thread.claim_lease_id) {
      throw new AgentError("The claim came back without a lease, so the agent cannot go on.");
    }
    if (thread.id !== taskId) {
      // Holding a task it will not work on would keep it from anyone else
      // until the lease lapsed.
      let released = true;
      try {
        await call("release_claim", { thread_id: thread.id, claim_lease_id: thread.claim_lease_id });
      } catch {
        released = false;
      }
      throw new AgentError(
        released
          ? `The claim took ${quoted(thread.title)}, not Say hello, so the agent gave it back.`
          : `The claim took ${quoted(thread.title)}, not Say hello, and could not give it back. Its lease lapses on its own.`,
      );
    }
    leaseId = thread.claim_lease_id;
    return `Took ${quoted(thread.title)}. It is the agent’s until it hands it on.`;
  }

  async function acknowledge() {
    const thread = await call("acknowledge_claim", { thread_id: taskId, claim_lease_id: leaseId });
    return thread?.work_started_at ? "Its working clock started." : "Acknowledged.";
  }

  async function hello() {
    await call("post_message", { thread_id: taskId, body: HELLO });
    return `Posted “${HELLO}”`;
  }

  async function result() {
    await call("set_thread_result", { thread_id: taskId, claim_lease_id: leaseId, result: RESULT });
    return `Recorded ${JSON.stringify(RESULT)}.`;
  }

  async function review() {
    const thread = await call("transition_thread", { thread_id: taskId, action: "start_review" });
    if (thread?.state !== "in_review") {
      const state = typeof thread?.state === "string" ? thread.state.replaceAll("_", " ") : "in an unknown state";
      throw new AgentError(`The task is ${state}, not in review.`);
    }
    return "The task is in review, waiting for you.";
  }

  const work = [claim, acknowledge, hello, result, review];

  // `onStep(index, state, detail)`: state is "running", "done" or "failed".
  // Resolves to { ok: true } or { ok: false, step, message }; never throws.
  async function run(onStep = () => {}) {
    if (running) return { ok: false, step: next, message: "The agent is already running." };
    running = true;
    try {
      while (next < work.length) {
        const index = next;
        onStep(index, "running", "");
        try {
          const detail = await work[index]();
          next += 1;
          onStep(index, "done", detail);
        } catch (err) {
          const message =
            err instanceof AgentError ? err.message : "Something went wrong in this page. Reload it and try again.";
          onStep(index, "failed", message);
          return { ok: false, step: index, message };
        }
        if (next < work.length && pauseMs > 0) await wait(pauseMs);
      }
      return { ok: true };
    } finally {
      running = false;
    }
  }

  return { run, finished: () => next >= work.length };
}
