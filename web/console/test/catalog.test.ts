import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import type { Credentials } from "../src/core/api.ts";
import { CATALOG_FILTERS, CATALOG_FORBIDDEN, CATALOG_SESSION_MISSING, COORDINATOR_TOOLS, CatalogApi, REVIEWER_TOOLS, WORKER_TOOLS, annotationBadges, filterTools, openTool, schemaOneOfRequired, schemaParams, summary, toolProfiles, toolTitle, typeWord, type JsonSchema } from "../src/core/catalog.ts";
import { GO_KEYS } from "../src/shell/commands.ts";
import { parseRoute } from "../src/shell/router.ts";

const consoleRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");

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

test("role and safety filters narrow the list, and an omitted filter stays the whole list", () => {
  const tools = [
    { name: "post_message", description: "Post a message to a thread." },
    { name: "list_threads", description: "List threads.", annotations: { readOnlyHint: true } },
    { name: "delete_thread", description: "Delete a thread.", annotations: { destructiveHint: true } },
    { name: "whoami", description: "Who the caller is.", annotations: { readOnlyHint: true, destructiveHint: true } },
    { name: "not_a_profile_tool", description: "Outside every profile.", annotations: { destructiveHint: true } },
  ];
  assert.deepEqual(CATALOG_FILTERS.map((f) => f.id), ["all", "worker", "reviewer", "coordinator", "read-only", "destructive"]);
  assert.equal(CATALOG_FILTERS.find((f) => f.id === "worker")?.path, "/mcp/worker");
  assert.equal(CATALOG_FILTERS.find((f) => f.id === "reviewer")?.path, "/mcp/reviewer");
  assert.equal(CATALOG_FILTERS.find((f) => f.id === "coordinator")?.path, "/mcp/coordinator");
  assert.deepEqual(filterTools(tools, "").map((t) => t.name), ["delete_thread", "list_threads", "not_a_profile_tool", "post_message", "whoami"]);
  assert.deepEqual(filterTools(tools, "", "all").map((t) => t.name), ["delete_thread", "list_threads", "not_a_profile_tool", "post_message", "whoami"]);
  assert.deepEqual(filterTools(tools, "", "worker").map((t) => t.name), ["post_message", "whoami"]);
  assert.deepEqual(filterTools(tools, "", "reviewer").map((t) => t.name), ["post_message", "whoami"]);
  assert.deepEqual(filterTools(tools, "", "coordinator").map((t) => t.name), ["list_threads", "post_message", "whoami"]);
  assert.deepEqual(filterTools(tools, "", "read-only").map((t) => t.name), ["list_threads", "whoami"]);
  assert.deepEqual(filterTools(tools, "", "destructive").map((t) => t.name), ["delete_thread", "not_a_profile_tool"]);
  assert.deepEqual(filterTools(tools, "delete", "worker"), []);
  assert.deepEqual(filterTools(tools, "post thread", "coordinator").map((t) => t.name), ["post_message"]);
});

test("tool profiles name the endpoints that list a tool", () => {
  assert.deepEqual(toolProfiles("post_message"), [
    { id: "worker", label: "Worker", path: "/mcp/worker" },
    { id: "reviewer", label: "Reviewer", path: "/mcp/reviewer" },
    { id: "coordinator", label: "Coordinator", path: "/mcp/coordinator" },
  ]);
  assert.deepEqual(toolProfiles("submit_review").map((p) => p.id), ["reviewer"]);
  assert.deepEqual(toolProfiles("claim_next_thread").map((p) => [p.id, p.path]), [["worker", "/mcp/worker"]]);
  assert.deepEqual(toolProfiles("not_a_tool"), []);
});

test("conditional anyOf requirements, nullable enums, item enums, defaults and formats", () => {
  const post = {
    type: "object",
    required: ["thread_id"],
    anyOf: [{ required: ["body"] }, { required: ["content"] }],
    properties: {
      thread_id: { type: "string", format: "uuid", description: "The thread." },
      body: { type: "string", description: " plain text " },
      content: { type: "array", description: "typed blocks", items: { type: "object", properties: { type: { type: "string", enum: ["text", "code"] } } } },
      kind: { anyOf: [{ type: "string", enum: ["note", "result"] }, { type: "null" }] },
      tags: { type: "array", items: { type: "string", enum: ["alpha", "beta"] } },
      inclusion: { type: "string", enum: ["pointer", "quote"], default: "pointer" },
      limit: { type: "integer", default: 100 },
      flag: { type: "boolean", default: false },
      when: { anyOf: [{ type: "string", format: "date-time" }, { type: "null" }] },
      link: { type: "string", format: "uri", default: "" },
    },
  };
  assert.equal(typeWord({ type: "string", enum: ["note", "result"] }), "enum");
  assert.equal(typeWord({ anyOf: [{ type: "string", enum: ["note", "result"] }, { type: "null" }] }), "enum?");
  assert.equal(typeWord({ type: "array", items: { type: "string", enum: ["alpha"] } }), "enum[]");
  assert.deepEqual(schemaOneOfRequired(post), [["body"], ["content"]]);
  const params = schemaParams(post);
  const by = Object.fromEntries(params.map((p) => [p.name, p]));
  assert.deepEqual(params.map((p) => p.name), ["thread_id", "body", "content", "flag", "inclusion", "kind", "limit", "link", "tags", "when"]);
  assert.equal(by.thread_id.required, true);
  assert.equal(by.thread_id.requirement, "required");
  assert.equal(by.thread_id.requiredOneOf, undefined);
  assert.equal(by.thread_id.format, "uuid");
  assert.equal(by.body.required, false);
  assert.equal(by.body.requirement, "one-of");
  assert.equal(by.body.requiredOneOf, true);
  assert.deepEqual(by.body.oneOfGroup, ["body"]);
  assert.equal(by.body.description, "plain text");
  assert.equal(by.content.requirement, "one-of");
  assert.deepEqual(by.content.choices, []);
  assert.equal(by.kind.type, "enum?");
  assert.equal(by.kind.required, false);
  assert.deepEqual(by.kind.choices, ["note", "result"]);
  assert.equal(by.tags.type, "enum[]");
  assert.deepEqual(by.tags.choices, ["alpha", "beta"]);
  assert.deepEqual(by.inclusion.choices, ["pointer", "quote"]);
  assert.equal(by.inclusion.defaultValue, "pointer");
  assert.equal(by.limit.defaultValue, "100");
  assert.equal(by.flag.defaultValue, "false");
  assert.equal(by.when.format, "date-time");
  assert.equal(by.when.type, "string?");
  assert.equal(by.link.format, "uri");
  assert.equal(by.link.defaultValue, '""');
});

test("message tools in the catalog require body or content, not both", () => {
  const listed = JSON.parse(readFileSync(resolve(consoleRoot, "../../crates/maidan-mcp/src/tools/catalog.json"), "utf8")) as { name?: string; inputSchema?: JsonSchema }[];
  for (const name of ["post_message", "edit_message", "post_dm_message"]) {
    const tool = listed.find((entry) => entry.name === name);
    assert.ok(tool, name);
    const params = schemaParams(tool.inputSchema);
    const body = params.find((p) => p.name === "body");
    const content = params.find((p) => p.name === "content");
    assert.equal(body?.required, false, name);
    assert.equal(body?.requirement, "one-of", name);
    assert.equal(content?.required, false, name);
    assert.equal(content?.requirement, "one-of", name);
    assert.deepEqual(schemaOneOfRequired(tool.inputSchema), [["body"], ["content"]]);
    assert.equal(params.some((p) => p.required), true, name);
  }
  const post = schemaParams(listed.find((entry) => entry.name === "post_message")?.inputSchema);
  assert.equal(post.find((p) => p.name === "thread_id")?.format, "uuid");
  const channel = schemaParams(listed.find((entry) => entry.name === "create_channel")?.inputSchema);
  assert.equal(channel.find((p) => p.name === "private")?.defaultValue, "false");
  const member = schemaParams(listed.find((entry) => entry.name === "add_channel_member")?.inputSchema);
  assert.equal(member.find((p) => p.name === "role")?.defaultValue, "member");
  assert.deepEqual(member.find((p) => p.name === "role")?.choices, ["member", "admin"]);
});

test("profile tool lists match profiles.rs", () => {
  const rs = readFileSync(resolve(consoleRoot, "../../crates/maidan-mcp/src/profiles.rs"), "utf8");
  const rustList = (name: string) => {
    const found = new RegExp(`pub const ${name}: &\\[&str\\] = &\\[([\\s\\S]*?)\\];`).exec(rs);
    assert.ok(found, `${name} missing from profiles.rs`);
    return [...found[1].matchAll(/"([a-z0-9_]+)"/g)].map((match) => match[1]);
  };
  assert.deepEqual([...WORKER_TOOLS], rustList("WORKER_TOOLS"));
  assert.deepEqual([...REVIEWER_TOOLS], rustList("REVIEWER_TOOLS"));
  assert.deepEqual([...COORDINATOR_TOOLS], rustList("COORDINATOR_TOOLS"));
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
