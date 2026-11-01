import { test } from "node:test";
import assert from "node:assert/strict";
import { callsIn, matches, check, PENDING } from "./check-spec.mjs";
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
