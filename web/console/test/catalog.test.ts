import { test } from "node:test";
import assert from "node:assert/strict";
import type { Credentials } from "../src/core/api.ts";
import { CATALOG_FORBIDDEN, CATALOG_SESSION_MISSING, CatalogApi, annotationBadges, filterTools, openTool, schemaParams, summary, toolTitle, typeWord } from "../src/core/catalog.ts";
import { GO_KEYS } from "../src/shell/commands.ts";
import { parseRoute } from "../src/shell/router.ts";

const bearer: Credentials = { base: "http://h", pastedToken: "tok", tokenSession: false, sessionMemberId: null };
const session: Credentials = { base: "http://h", pastedToken: "", tokenSession: false, sessionMemberId: "me" };
const answer = (status: number, body?: unknown) => async () => new Response(body === undefined ? "" : JSON.stringify(body), { status });

test("parameters come from the schema, required first, with type and description", () => {
  const params = schemaParams({
    type: "object",
    required: ["thread_id"],
    properties: {
      limit: { type: "integer", description: " How many. " },
      thread_id: { type: "string", description: "The thread." },
      tags: { type: "array", items: { type: "string" } },
      mode: { type: "string", enum: ["lexical", "semantic"] },
      note: { type: ["string", "null"] },
      who: { anyOf: [{ type: "string" }, { type: "null" }] },
    },
  });
  assert.deepEqual(params.map((p) => [p.name, p.type, p.required]), [
    ["thread_id", "string", true],
    ["limit", "integer", false],
    ["mode", "enum", false],
    ["note", "string?", false],
    ["tags", "string[]", false],
    ["who", "string?", false],
  ]);
  assert.equal(params[1].description, "How many.");
  assert.deepEqual(params[2].choices, ["lexical", "semantic"]);
});

test("a schema with no properties has no parameters, and odd shapes read as any", () => {
  assert.deepEqual(schemaParams({ type: "object" }), []);
  assert.deepEqual(schemaParams(null), []);
  assert.equal(typeWord(undefined), "any");
  assert.equal(typeWord({}), "any");
  assert.equal(typeWord({ properties: {} }), "object");
  assert.equal(typeWord({ $ref: "#/defs/Thread" }), "Thread");
});

test("annotation hints become badges; read-only wins over destructive", () => {
  assert.deepEqual(annotationBadges({ readOnlyHint: true, idempotentHint: true }).map((b) => b.id), ["read-only", "idempotent"]);
  assert.deepEqual(annotationBadges({ destructiveHint: true }).map((b) => b.id), ["destructive"]);
  assert.deepEqual(annotationBadges({ readOnlyHint: true, destructiveHint: true }).map((b) => b.id), ["read-only"]);
  assert.deepEqual(annotationBadges(null), []);
});

test("the filter matches every word across name, title and description", () => {
  const tools = [
    { name: "post_message", description: "Post a message to a thread." },
    { name: "list_threads", title: "List threads", description: "Threads in a channel." },
    { name: "claim_task", description: "Claim a ready task." },
  ];
  assert.deepEqual(filterTools(tools, "").map((t) => t.name), ["claim_task", "list_threads", "post_message"]);
  assert.deepEqual(filterTools(tools, "THREAD").map((t) => t.name), ["list_threads", "post_message"]);
  assert.deepEqual(filterTools(tools, "post thread").map((t) => t.name), ["post_message"]);
  assert.deepEqual(filterTools(tools, "nothing"), []);
  assert.equal(toolTitle(tools[1]), "List threads");
  assert.equal(toolTitle(tools[0]), "post_message");
});

test("the list row shows a description's first sentence", () => {
  assert.equal(summary("Post a message. Then more."), "Post a message.");
  assert.equal(summary("  no stop  "), "no stop");
  assert.equal(summary(null), "");
});

test("a catalog hash opens one tool, by its encoded name", () => {
  assert.equal(parseRoute("#/catalog"), "catalog");
  assert.equal(parseRoute("#/catalog/post_message"), "catalog");
  assert.equal(openTool("#/catalog/post_message"), "post_message");
  assert.equal(openTool("#/catalog/a%2Fb"), "a/b");
  assert.equal(openTool("#/catalog"), null);
  assert.equal(openTool("#/people/m/x"), null);
  assert.equal(GO_KEYS.c, "catalog");
});

test("GET /tools reads the catalog with the bearer and drops malformed entries", async () => {
  const calls: string[] = [];
  const api = new CatalogApi(async (url, init) => {
    calls.push(`${url} ${new Headers(init?.headers).get("Authorization")}`);
    return new Response(JSON.stringify({ tools: [{ name: "a" }, { nope: 1 }, null] }), { status: 200 });
  }, () => bearer, () => "bearer");
  assert.deepEqual((await api.list()).map((t) => t.name), ["a"]);
  assert.deepEqual(calls, ["http://h/tools Bearer tok"]);
});

test("a 403 says which capability is missing; a 401 is an error", async () => {
  await assert.rejects(new CatalogApi(answer(403, { error: "forbidden" }), () => bearer, () => "bearer").list(), (e: Error & { status?: number }) => e.message === CATALOG_FORBIDDEN && e.status === 403);
  await assert.rejects(new CatalogApi(answer(401, { error: "unauthorized" }), () => bearer, () => "bearer").list(), (e: Error & { status?: number }) => e.status === 401);
});

test("a session goes through /ui/api, and a server without that route says so", async () => {
  const urls: string[] = [];
  const api = new CatalogApi(async (url) => {
    urls.push(url);
    return new Response("", { status: 404 });
  }, () => session, () => "session");
  await assert.rejects(api.list(), (e: Error) => e.message === CATALOG_SESSION_MISSING);
  assert.deepEqual(urls, ["http://h/ui/api/tools"]);
});
