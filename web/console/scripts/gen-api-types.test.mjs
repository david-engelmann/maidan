import { test } from "node:test";
import assert from "node:assert/strict";
import { tsType, generate, routes, isSafeSchemaRef, isSafePathTemplate } from "./gen-api-types.mjs";

test("schema $refs must name a components schema with a safe ident", () => {
  assert.equal(isSafeSchemaRef("#/components/schemas/Id"), true);
  assert.equal(isSafeSchemaRef("#/components/schemas/../../evil"), false);
  assert.equal(isSafeSchemaRef("#/components/schemas/1Bad"), false);
  assert.equal(isSafeSchemaRef("https://evil.example/schema"), false);
  assert.equal(tsType({ $ref: "#/components/schemas/../../evil" }), "unknown");
  assert.equal(tsType({ $ref: "#/components/schemas/Id" }), "Id");
});

test("path templates with empty or parent segments are not ApiRoute entries", () => {
  assert.equal(isSafePathTemplate("/threads/{id}"), true);
  assert.equal(isSafePathTemplate("/threads/../members"), false);
  assert.equal(isSafePathTemplate("/threads//{id}"), false);
  assert.equal(isSafePathTemplate("/threads/{id}/"), false);
  const spec = {
    paths: {
      "/threads/{id}": { get: {} },
      "/threads/../secret": { get: {} },
      "/threads//{id}": { post: {} },
    },
    components: { schemas: {} },
  };
  assert.deepEqual(routes(spec), ["GET /threads/{id}"]);
  const out = generate(spec);
  assert.match(out, /\| "GET \/threads\/\{id\}"/);
  assert.doesNotMatch(out, /\.\.\/secret/);
});

test("schema names that are not safe idents are skipped", () => {
  const out = generate({
    paths: {},
    components: { schemas: { Good: { type: "string" }, "Bad-Name": { type: "number" }, "12": { type: "boolean" } } },
  });
  assert.match(out, /export type Good = string;/);
  assert.doesNotMatch(out, /Bad-Name/);
  assert.doesNotMatch(out, /export type 12/);
});
