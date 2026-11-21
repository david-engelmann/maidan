// What the recording's cast does, over the server's own API. Every call here
// is a real one against the server; nothing is drawn that did not happen.
export const O = process.env.STAGE_ORIGIN ?? "http://127.0.0.1:18751";
export const ADMIN = process.env.STAGE_ADMIN_TOKEN ?? "dev-admin-token-change-me-0123456789";

export const pause = (ms) => new Promise((r) => setTimeout(r, ms));

export async function rest(token, method, path, body) {
  const res = await fetch(O + path, {
    method,
    headers: {
      authorization: `Bearer ${token}`,
      accept: "application/json",
      "idempotency-key": crypto.randomUUID(),
      ...(body === undefined ? {} : { "content-type": "application/json" }),
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path} answered ${res.status}: ${text.slice(0, 300)}`);
  return text ? JSON.parse(text) : null;
}

let calls = 0;
/** One MCP tool call, as an agent makes it. Returns the tool's own answer. */
export async function mcp(token, name, args = {}) {
  const res = await fetch(O + "/mcp/streamable", {
    method: "POST",
    headers: { authorization: `Bearer ${token}`, "content-type": "application/json", accept: "application/json, text/event-stream" },
    body: JSON.stringify({ jsonrpc: "2.0", id: ++calls, method: "tools/call", params: { name, arguments: args } }),
  });
  let raw = await res.text();
  if (/^\s*(event|data):/.test(raw)) raw = raw.split("\n").find((l) => l.startsWith("data:"))?.slice(5) ?? "{}";
  const answer = JSON.parse(raw);
  if (answer.error) throw new Error(`${name}: ${answer.error.message}`);
  const text = (answer.result.content ?? []).map((p) => p.text ?? "").join("");
  if (answer.result.isError) throw new Error(`${name}: ${text.slice(0, 300)}`);
  return text ? JSON.parse(text) : null;
}

/** The workspace, its person and three agents, and the channel they share. */
export async function cast() {
  const me = await mcp(ADMIN, "whoami");
  const ws = me.workspace_id;
  await rest(ADMIN, "PATCH", `/members/${me.actor_id}`, { display_name: "Maya" });
  const agent = async (handle, display_name) => {
    const a = await rest(ADMIN, "POST", `/workspaces/${ws}/agents`, { handle, display_name });
    return { name: display_name, handle, id: a.member.id, token: a.token.secret };
  };
  const claude = await agent("claude-code", "Claude Code");
  const codex = await agent("codex", "Codex");
  const cursor = await agent("cursor", "Cursor");
  const channel = await rest(ADMIN, "POST", `/workspaces/${ws}/channels`, { name: "api", topic: "The public API" });
  return { ws, maya: { name: "Maya", id: me.actor_id, token: ADMIN }, claude, codex, cursor, channel: channel.id };
}

/** Claims the oldest open task in the channel and starts its clock. */
export async function claimNext(agent, channel) {
  const got = await mcp(agent.token, "claim_next_thread", { channel_id: channel, lease_secs: 900 });
  const t = got?.thread ?? got;
  if (!t) return null;
  await mcp(agent.token, "acknowledge_claim", { thread_id: t.id, claim_lease_id: t.claim_lease_id });
  return t;
}

/** A task worked from open to done, for the board the recording opens on. */
export async function finished(s, worker, reviewer, title, said, result) {
  await rest(s.maya.token, "POST", `/channels/${s.channel}/threads`, { title });
  const t = await claimNext(worker, s.channel);
  await mcp(worker.token, "post_message", { thread_id: t.id, body: said });
  await mcp(worker.token, "set_thread_result", { thread_id: t.id, claim_lease_id: t.claim_lease_id, result });
  await mcp(worker.token, "transition_thread", { thread_id: t.id, action: "start_review" });
  const packet = await mcp(reviewer.token, "get_review_packet", { thread_id: t.id });
  await mcp(reviewer.token, "submit_review", { thread_id: t.id, decision: "approve", evidence_root: packet.evidence_root });
  await rest(s.maya.token, "POST", `/threads/${t.id}`, { action: "close" });
  return t;
}

/** Events for the workspace, live, as the server's WebSocket sends them. */
export function events(token, ws, onEvent) {
  const socket = new WebSocket(O.replace(/^http/, "ws") + "/ws/subscribe");
  const ready = new Promise((resolve, reject) => {
    socket.addEventListener("open", () => socket.send(JSON.stringify({ token, filter: { workspace_id: ws }, after_id: 0 })));
    socket.addEventListener("error", () => reject(new Error("the event stream did not open")));
    socket.addEventListener("message", (m) => {
      const frame = JSON.parse(m.data);
      if (frame.type === "subscribe_ack") resolve(frame);
      else if (frame.log_id !== undefined) onEvent(frame);
    });
  });
  return { ready, close: () => socket.close() };
}
