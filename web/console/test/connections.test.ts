import assert from "node:assert/strict";
import { test } from "node:test";
import type { Credentials } from "../src/core/api.ts";
import {
  CONNECTIONS_NEED,
  ConnectionsApi,
  SURFACES,
  connectBody,
  connectRefusal,
  connectedOn,
  inboundLine,
  scrub,
  stateLine,
  stateWord,
  surfaceInfo,
  surfaceName,
  type Connection,
} from "../src/core/connections.ts";

const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };

function api(responses: Array<{ status?: number; body?: unknown; headers?: Record<string, string> }>, creds = session) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  let i = 0;
  const fn = async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const r = responses[Math.min(i++, responses.length - 1)];
    const empty = r.status === 204;
    return new Response(empty ? null : JSON.stringify(r.body ?? {}), { status: r.status ?? 200, headers: { "content-type": "application/json", ...(r.headers ?? {}) } });
  };
  return { api: new ConnectionsApi(fn, () => creds, () => "session"), calls };
}

const row = (over: Partial<Connection> = {}): Connection => ({
  surface: "github",
  source: "workspace",
  account: "octocat",
  connected_at: "2026-11-23T16:00:00Z",
  inbound: { url: "https://maidan.example/workspaces/w/connections/github/webhook", secret_set: true },
  ...over,
});
const year = (d: Date) => String(d.getUTCFullYear());

test("the three surfaces are github, slack and linear, in that order, and only the first two have an inbound secret", () => {
  assert.deepEqual(SURFACES.map((s) => s.id), ["github", "slack", "linear"]);
  assert.deepEqual(SURFACES.map((s) => s.secret?.field ?? null), ["webhook_secret", "signing_secret", null]);
  assert.equal(surfaceName("slack"), "Slack");
  assert.equal(surfaceName("jira"), "jira");
  assert.equal(surfaceInfo("jira"), null);
});

test("each state has its own plain sentence", () => {
  assert.equal(stateLine(row(), year), "Connected with this workspace’s own GitHub account, octocat, since 2026.");
  assert.equal(stateLine(row({ account: null, connected_at: null }), year), "Connected with this workspace’s own GitHub account.");
  assert.match(stateLine(row({ surface: "slack", source: "instance", account: null, connected_at: null })), /^Using this server’s own Slack connection\. Connect this workspace’s own account/);
  assert.match(stateLine(row({ surface: "linear", source: null, account: null, connected_at: null, inbound: null })), /^Not connected\. Nothing in this workspace can send to or read from Linear/);
  assert.deepEqual(["workspace", "instance", null].map((s) => stateWord(s as Connection["source"])), ["Connected", "Server’s account", "Not connected"]);
  assert.equal(connectedOn("not a date"), null);
});

test("the inbound line says whether the URL accepts anything yet", () => {
  assert.match(inboundLine("github", { url: "u", secret_set: true }), /^A webhook secret is set\./);
  assert.match(inboundLine("github", { url: "u", secret_set: false }), /^No webhook secret is set, so this URL accepts nothing yet/);
  assert.match(inboundLine("slack", { url: "u", secret_set: true }), /^A signing secret is set\./);
});

test("nothing a person reads has an em dash, a semicolon or a colon in running prose", () => {
  const words = [
    CONNECTIONS_NEED,
    ...SURFACES.flatMap((s) => [s.paste, s.secret?.hint ?? ""]),
    stateLine(row()),
    stateLine(row({ source: "instance" })),
    stateLine(row({ source: null })),
    inboundLine("github", { url: "u", secret_set: false }),
    connectRefusal("github", { token: "x", webhook_secret: "short" }, false) ?? "",
    connectRefusal("linear", {}, false) ?? "",
  ];
  for (const w of words) assert.doesNotMatch(w, /[—;:]/, w);
});

test("a form needs a token, except a secret alone on a surface that is already connected", () => {
  assert.match(connectRefusal("linear", {}, false) ?? "", /Paste the API key to connect Linear/);
  assert.match(connectRefusal("github", { token: "  " }, false) ?? "", /Paste the fine-grained personal access token to connect GitHub/);
  assert.match(connectRefusal("slack", { signing_secret: "abc" }, false) ?? "", /Paste the bot token to connect Slack/);
  assert.equal(connectRefusal("slack", { signing_secret: "abc" }, true), null);
  assert.equal(connectRefusal("linear", {}, true) === null, false, "linear has no secret to send alone");
  assert.equal(connectRefusal("github", { token: "t" }, false), null);
});

test("a GitHub webhook secret is 16 to 128 characters with no spaces", () => {
  assert.match(connectRefusal("github", { token: "t", webhook_secret: "short" }, false) ?? "", /16 to 128 characters with no spaces/);
  assert.match(connectRefusal("github", { token: "t", webhook_secret: "has a space inside it" }, false) ?? "", /16 to 128/);
  assert.match(connectRefusal("github", { token: "t", webhook_secret: "x".repeat(129) }, false) ?? "", /16 to 128/);
  assert.equal(connectRefusal("github", { token: "t", webhook_secret: "x".repeat(16) }, false), null);
  assert.equal(connectRefusal("github", { token: "t", webhook_secret: "  " }, false), null, "an empty secret is just left out");
});

test("the body is trimmed and leaves out what was not filled in", () => {
  assert.deepEqual(connectBody("github", { token: " ghp_x ", webhook_secret: " " }), { token: "ghp_x" });
  assert.deepEqual(connectBody("github", { token: "t", webhook_secret: " 0123456789abcdef " }), { token: "t", webhook_secret: "0123456789abcdef" });
  assert.deepEqual(connectBody("slack", { token: "xoxb-1", signing_secret: "s" }), { token: "xoxb-1", signing_secret: "s" });
  assert.deepEqual(connectBody("slack", { signing_secret: "s" }), { signing_secret: "s" });
  // Linear has no inbound secret, so one never goes out.
  assert.deepEqual(connectBody("linear", { token: "k", webhook_secret: "0123456789abcdef" }), { token: "k" });
});

test("scrub takes what was typed out of a sentence", () => {
  assert.equal(scrub("GitHub refused ghp_secret123: bad credentials", " ghp_secret123 "), "GitHub refused [hidden]: bad credentials");
  assert.equal(scrub("nothing here", "ab"), "nothing here", "a very short value is not worth matching");
});

test("the list is a quiet refusal on 403, and the three rows otherwise", async () => {
  const no = api([{ status: 403, body: { detail: "only a workspace admin" } }]);
  assert.deepEqual(await no.api.list("w"), { allowed: false });
  assert.equal(no.calls[0].url, "http://h/ui/api/workspaces/w/connections");
  const yes = api([{ body: [row(), row({ surface: "slack", source: "instance" }), row({ surface: "linear", source: null })] }], bearer);
  const read = await yes.api.list("w");
  assert.equal(read.allowed && read.value.length, 3);
  assert.equal(yes.calls[0].url, "http://h/workspaces/w/connections");
  assert.equal(new Headers(yes.calls[0].init?.headers).get("Authorization"), "Bearer tok");
  await assert.rejects(api([{ status: 500 }]).api.list("w"), /The server hit an error/);
});

test("connecting is a PUT of the trimmed fields, to the session tree or the bearer tree", async () => {
  const s = api([{ body: row() }]);
  const saved = await s.api.connect("w", "github", { token: " ghp_x ", webhook_secret: "0123456789abcdef" });
  assert.equal(saved.account, "octocat");
  assert.equal(s.calls[0].url, "http://h/ui/api/workspaces/w/connections/github");
  assert.equal(s.calls[0].init?.method, "PUT");
  assert.deepEqual(JSON.parse(String(s.calls[0].init?.body)), { token: "ghp_x", webhook_secret: "0123456789abcdef" });
  assert.ok(new Headers(s.calls[0].init?.headers).get("Idempotency-Key"));
  const b = api([{ body: row({ surface: "linear" }) }], bearer);
  await b.api.connect("w", "linear", { token: "lin_api" });
  assert.equal(b.calls[0].url, "http://h/workspaces/w/connections/linear");
});

test("a 400 and a 503 come back in the server's own sentence, and other failures in the console's", async () => {
  const refused = api([{ status: 400, body: { detail: "GitHub refused this token: Bad credentials" } }]);
  await assert.rejects(refused.api.connect("w", "github", { token: "t" }), /GitHub refused this token: Bad credentials/);
  const noKey = api([{ status: 503, body: { detail: "Set FEDERATION_ENCRYPTION_KEY to store a connection" } }]);
  await assert.rejects(noKey.api.connect("w", "slack", { token: "t" }), /Set FEDERATION_ENCRYPTION_KEY/);
  const forbidden = api([{ status: 403, body: { detail: "needs token:admin" } }]);
  await assert.rejects(forbidden.api.connect("w", "slack", { token: "t" }), /not allowed to do this/);
  const gone = api([{ status: 404, body: { detail: "unknown surface" } }]);
  await assert.rejects(gone.api.connect("w", "slack", { token: "t" }), /Not found/);
  const failed = await refused.api.connect("w", "github", { token: "t" }).catch((e: unknown) => e as { status: number });
  assert.equal(failed.status, 400);
});

test("removing a connection is a DELETE that answers 204", async () => {
  const { api: a, calls } = api([{ status: 204 }]);
  await a.remove("w", "slack");
  assert.equal(calls[0].url, "http://h/ui/api/workspaces/w/connections/slack");
  assert.equal(calls[0].init?.method, "DELETE");
  assert.ok(new Headers(calls[0].init?.headers).get("Idempotency-Key"));
  await assert.rejects(api([{ status: 403 }]).api.remove("w", "slack"), /not allowed to do this/);
});
