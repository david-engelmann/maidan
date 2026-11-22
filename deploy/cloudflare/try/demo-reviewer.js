// The try page's second agent: it reviews what the demo agent handed off and
// closes the task, with the sandbox's reviewer token, so the loop on the page
// runs from claim to closed with no person in it. The server refuses a close
// from the member who did the work, and an approval counts only from someone
// else, so the loop needs a second member; here it is another scripted agent.
//
// It is a reviewer with a rule, not a rubber stamp. It approves when the
// result records a line that is on the task and the decision model's advice
// flags nothing. Anything else it leaves for a person, and says why: the
// page then offers the console. Every line it shows comes from the server's
// answer, over the MCP endpoint a real reviewer uses.

import { AgentError, createCaller, createRunner, readAdvice } from "./demo-agent.js";

// The steps in order. The second makes two calls, the result and the task's
// messages, to hold one against the other.
export const REVIEW_STEPS = [
  { tool: "get_review_packet", label: "Read what was handed over" },
  { tool: "get_thread_result", label: "Check the result against the task" },
  { tool: "advise_land_gate", label: "Read the decision model’s advice" },
  { tool: "submit_review", label: "Approve the hand-off" },
  { tool: "transition_thread", label: "Close the task" },
];

export function createDemoReviewer({ fetch, token, taskId, retrySecs = 20, pauseMs = 0, sleep } = {}) {
  const wait = sleep ?? ((ms) => new Promise((resolve) => setTimeout(resolve, ms)));
  const call = createCaller({ fetch, token, retrySecs });
  // What the approval is tied to: the root of the packet this reviewer read.
  let root = "";
  // Why it left the hand-off for a person; empty while it has no reason to.
  let left = "";

  async function packet() {
    const read = await call("get_review_packet", { thread_id: taskId });
    if (typeof read?.evidence_root !== "string" || !read.evidence_root) {
      throw new AgentError("The hand-off came with no evidence root, so there is nothing to approve.");
    }
    root = read.evidence_root;
    return `The hand-off is pinned as evidence ${root.slice(0, 12)}…, and an approval counts for that and nothing later.`;
  }

  async function result() {
    const answer = await call("get_thread_result", { thread_id: taskId });
    const said = answer?.result?.said;
    if (typeof said !== "string" || !said.trim()) {
      left = "The result does not record a line the agent posted.";
      return left;
    }
    const messages = await call("list_messages", { thread_id: taskId });
    if (!Array.isArray(messages) || !messages.some((message) => message?.body === said)) {
      left = "The result records a line that is not on the task.";
      return left;
    }
    return `The result records “${said}”, and that line is on the task.`;
  }

  // Advice is advice: with none, the check above stands on its own. With
  // some, anything short of "nothing flagged" goes to a person.
  async function advice() {
    let read = null;
    try {
      read = readAdvice(await call("advise_land_gate", { thread_id: taskId }));
    } catch {
      read = null;
    }
    if (!read) return "No decision model answered, so the review rests on the check above.";
    if (read.land !== "green") {
      left = `The decision model’s advice reads “${read.title}”. ${read.why}`;
      return `${read.title}. ${read.why}`;
    }
    return `${read.title}. ${read.why}`;
  }

  async function approve() {
    const review = await call("submit_review", { thread_id: taskId, decision: "approve", evidence_root: root });
    if (review?.decision !== "approve") throw new AgentError("The server did not record the approval.");
    return "Approved, for the evidence it read.";
  }

  async function close() {
    const thread = await call("transition_thread", { thread_id: taskId, action: "close" });
    if (thread?.state !== "closed") {
      const state = typeof thread?.state === "string" ? thread.state.replaceAll("_", " ") : "in an unknown state";
      throw new AgentError(`The task is ${state}, not closed.`);
    }
    return "Closed. No step waited for a person.";
  }

  const steps = createRunner([packet, result, advice, approve, close], {
    pauseMs,
    wait,
    who: "reviewer",
    halted: () => left,
  });

  return { run: steps.run, finished: steps.finished };
}
