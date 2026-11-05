import { test } from "node:test";
import assert from "node:assert/strict";
import { CLIENTS, HANDLE_MAX, clientSetup, cursorInstallLink, handleFrom, nameRefusal, vscodeInstallLink } from "../src/core/addAgent.ts";
import { STARTER_TASK_TITLE, askAgentLine, guideTasks, showFirstRun } from "../src/core/firstRun.ts";
import { ADD_AGENT_REFUSED, PeopleApi, PeopleError } from "../src/core/people.ts";
import type { Credentials } from "../src/core/api.ts";

const BASE = "https://maidan.example";
const TOKEN = "mdn_secret_123";

test("a typed name becomes a handle the server takes", () => {
  assert.equal(handleFrom("Claude"), "claude");
  assert.equal(handleFrom("  My Coding  Agent "), "my-coding-agent");
  assert.equal(handleFrom("Zoë's bot!"), "zoes-bot");
  assert.equal(handleFrom("codex_2.0"), "codex_2.0");
  assert.equal(handleFrom("---"), "");
  assert.equal(handleFrom("x".repeat(60)).length, HANDLE_MAX);
  assert.match(handleFrom("a ".repeat(30)), /^[a-z0-9._-]+$/);
  assert.doesNotMatch(handleFrom("a ".repeat(30)), /-$/);
  assert.equal(nameRefusal("Claude"), null);
  assert.ok(nameRefusal("   "));
  assert.ok(nameRefusal("!!!"));
  assert.ok(nameRefusal("x".repeat(101)));
});

test("every client's setup carries this origin and this agent's token", () => {
  for (const { id } of CLIENTS) {
    const steps = clientSetup(id, BASE, TOKEN, "chan-1");
    assert.ok(steps.length > 0, id);
    const all = steps.map((s) => s.text).join("\n");
    assert.ok(all.includes(TOKEN), `${id} carries the token`);
    assert.ok(all.includes(BASE), `${id} uses the console's origin`);
    assert.doesNotMatch(all, /REPLACE_WITH|127\.0\.0\.1|maidan\.world/, id);
  }
});

test("Claude Code gets one command with the header", () => {
  const [step] = clientSetup("claude", `${BASE}/`, TOKEN);
  assert.equal(step.text, `claude mcp add --transport http maidan ${BASE}/mcp/streamable --header "Authorization: Bearer ${TOKEN}"`);
});

test("the Cursor link is base64 of the server config, with no name wrapper", () => {
  const link = cursorInstallLink(BASE, TOKEN);
  const config = new URL(link).searchParams.get("config");
  assert.ok(config);
  assert.deepEqual(JSON.parse(atob(config)), { url: `${BASE}/mcp/streamable`, headers: { Authorization: `Bearer ${TOKEN}` } });
  assert.equal(new URL(link).searchParams.get("name"), "maidan");
});

test("the VS Code link is the named http server as URL-encoded JSON", () => {
  const link = vscodeInstallLink(BASE, TOKEN);
  assert.ok(link.startsWith("vscode:mcp/install?"));
  const server = JSON.parse(decodeURIComponent(link.slice("vscode:mcp/install?".length)));
  assert.deepEqual(server, { name: "maidan", type: "http", url: `${BASE}/mcp/streamable`, headers: { Authorization: `Bearer ${TOKEN}` } });
});

test("Codex reads the token from the environment the snippet exports", () => {
  const steps = clientSetup("codex", BASE, TOKEN);
  assert.equal(steps[0].text, `export MAIDAN_TOKEN=${TOKEN}`);
  assert.match(steps[1].text, /^\[mcp_servers\.maidan\]\nurl = "https:\/\/maidan\.example\/mcp\/streamable"\nbearer_token_env_var = "MAIDAN_TOKEN"/);
  assert.ok(!steps[1].text.includes(TOKEN), "the config file never holds the token");
});

test("your own loop exports both variables and claims from the channel", () => {
  const [shell, python] = clientSetup("loop", `${BASE}/`, TOKEN, "chan-1");
  assert.match(shell.text, new RegExp(`^export MAIDAN_URL=${BASE.replace(/\./g, "\\.")}\nexport MAIDAN_TOKEN=${TOKEN}`));
  assert.match(python.text, /claim_next_thread\("chan-1"\)/);
  assert.match(python.text, /start_review/);
});

function creds(over: Partial<Credentials> = {}): Credentials {
  return { base: BASE, pastedToken: "", tokenSession: false, sessionMemberId: "m1", ...over };
}

function capture(status: number, body: unknown) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  const fetchFn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
  };
  return { calls, fetchFn };
}

test("a signed-in session adds through the console route, a token through the bearer one", async () => {
  const ok = { member: { id: "a1", handle: "claude" }, token: { secret: TOKEN } };
  const session = capture(201, ok);
  const out = await new PeopleApi(session.fetchFn, () => creds(), () => "session").addAgent("w1", "Claude");
  assert.deepEqual(out, { memberId: "a1", handle: "claude", secret: TOKEN });
  assert.equal(session.calls[0].url, `${BASE}/ui/api/workspaces/w1/agents`);
  assert.equal(session.calls[0].init?.method, "POST");
  assert.equal(session.calls[0].init?.credentials, "include");
  assert.deepEqual(JSON.parse(String(session.calls[0].init?.body)), { handle: "claude", display_name: "Claude" });

  const bearer = capture(201, ok);
  await new PeopleApi(bearer.fetchFn, () => creds({ pastedToken: "admin", sessionMemberId: null }), () => "bearer").addAgent("w1", "Claude");
  assert.equal(bearer.calls[0].url, `${BASE}/workspaces/w1/agents`);
});

test("a refused add says who may add one, and a bad name never leaves the page", async () => {
  const refused = capture(403, { detail: "only a workspace admin can add an agent" });
  await assert.rejects(new PeopleApi(refused.fetchFn, () => creds(), () => "session").addAgent("w1", "Claude"), (e: unknown) => e instanceof PeopleError && e.message === ADD_AGENT_REFUSED);
  const never = capture(201, {});
  await assert.rejects(new PeopleApi(never.fetchFn, () => creds(), () => "session").addAgent("w1", "!!!"), PeopleError);
  assert.equal(never.calls.length, 0);
});

test("the untouched starter task keeps the guide up; anything else puts it away", () => {
  const starter = { title: STARTER_TASK_TITLE, state: "open", assignee_id: null };
  assert.equal(guideTasks([]), 0);
  assert.equal(showFirstRun("ready", guideTasks([starter]), false), true);
  assert.equal(guideTasks([{ ...starter, assignee_id: "m1" }]), 1);
  assert.equal(guideTasks([{ ...starter, state: "in_review" }]), 1);
  assert.equal(guideTasks([starter, { title: "Ship the release", state: "open", assignee_id: null }]), 1);
});

test("the guide tells the person what to ask their agent", () => {
  assert.equal(askAgentLine("general"), "Ask your agent: take the next task in #general.");
  assert.equal(askAgentLine(null), "Ask your agent: take the next task in #general.");
});
