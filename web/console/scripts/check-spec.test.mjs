import { test } from "node:test";
import assert from "node:assert/strict";
import { callsIn, matches, check, PENDING, isSafeApiPath } from "./check-spec.mjs";
import { tsType, generate } from "./gen-api-types.mjs";

const spec = {
  paths: { "/threads/{id}": { get: {}, post: {} }, "/members/{member_id}/channel-follows": { get: {}, post: {} } },
  components: { schemas: { Id: { type: "string" }, Row: { type: "object", required: ["id"], properties: { id: { $ref: "#/components/schemas/Id" }, note: { type: ["string", "null"] } } } } },
};

test("reads method and path off a line, dropping the query", () => {
  assert.deepEqual(callsIn('await this.send("POST", `/threads/${id}?x=1`, {});'), [{ line: 1, method: "POST", path: "/threads/{}" }]);
  assert.deepEqual(callsIn("return this.get(`/threads/${id}`);"), [{ line: 1, method: "GET", path: "/threads/{}" }]);
});

test("a path built into a variable takes the method of the send that uses it", () => {
  const src = 'const path = `/threads/${id}`;\nreturn this.send<Message>("POST", path, { body });';
  assert.equal(callsIn(src)[0].method, "POST");
});

test("template holes match path params, including inside a segment", () => {
  assert.ok(matches(spec, "GET", "/threads/{}"));
  assert.ok(matches(spec, "POST", "/members/{}/{}-follows"));
  assert.ok(!matches(spec, "DELETE", "/threads/{}"));
  assert.ok(!matches(spec, "GET", "/threads/{}/votes-typo"));
});

test("drift is reported with file and line", () => {
  const errs = check(spec, ["a.ts"], () => "x\nthis.get(`/threads/${t}/nope`)");
  assert.deepEqual(errs, ["a.ts:2: GET /threads/{}/nope is not in the spec"]);
});

test("pending routes name their PR", () => {
  for (const why of PENDING.values()) assert.match(why, /^#\d+/);
});

test("schemas become TypeScript types", () => {
  assert.equal(tsType({ type: ["string", "null"] }), "string | null");
  assert.equal(tsType({ type: "array", items: { $ref: "#/components/schemas/Id" } }), "Id[]");
  const out = generate(spec);
  assert.match(out, /export type Row = \{ id: Id; note\?: string \| null; \};/);
  assert.match(out, /\| "POST \/members\/\{member_id\}\/channel-follows"/);
});

test("single-quoted paths and the /tools and /ui/api roots are checked too", () => {
  assert.deepEqual(callsIn("return this.get('/threads/nope');"), [{ line: 1, method: "GET", path: "/threads/nope" }]);
  assert.deepEqual(callsIn("return this.get(`/tools`);"), [{ line: 1, method: "GET", path: "/tools" }]);
  assert.deepEqual(callsIn("return this.get(`/ui/api/messages/${m}/votes`);"), [{ line: 1, method: "GET", path: "/ui/api/messages/{}/votes" }]);
});

test("paths with empty or parent segments are not console API calls", () => {
  assert.equal(isSafeApiPath("/threads/{}"), true);
  assert.equal(isSafeApiPath("/threads/../members"), false);
  assert.equal(isSafeApiPath("/threads//id"), false);
  assert.equal(isSafeApiPath("threads/x"), false);
  assert.equal(isSafeApiPath("/threads/{}?x=1"), false);
  assert.deepEqual(callsIn('await this.send("GET", `/threads/${id}/../members`);'), []);
  assert.deepEqual(callsIn('await this.send("GET", `/threads//${id}`);'), []);
  assert.ok(!matches(spec, "GET", "/threads/../members"));
  assert.ok(!matches(spec, "GET", "/threads//id"));
  assert.ok(!matches(spec, "", "/threads/{}"));
});
