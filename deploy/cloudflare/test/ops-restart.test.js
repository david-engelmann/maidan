import { test } from "node:test";
import assert from "node:assert/strict";
import { isOpsRestart, OPS_RESTART_PATH } from "../src/edge.js";

const req = (method, path, auth) =>
  new Request(`https://swarm.maidan.world${path}`, { method, headers: auth ? { authorization: auth } : {} });

test("only a POST to the restart path with the exact bearer restarts", async () => {
  assert.ok(await isOpsRestart(req("POST", OPS_RESTART_PATH, "Bearer s3cret"), "s3cret"));
  assert.equal(await isOpsRestart(req("POST", OPS_RESTART_PATH, "Bearer wrong!"), "s3cret"), false);
  assert.equal(await isOpsRestart(req("POST", OPS_RESTART_PATH), "s3cret"), false);
  assert.equal(await isOpsRestart(req("GET", OPS_RESTART_PATH, "Bearer s3cret"), "s3cret"), false);
  assert.equal(await isOpsRestart(req("POST", "/other", "Bearer s3cret"), "s3cret"), false);
});

test("an instance without the token has no restart at all", async () => {
  assert.equal(await isOpsRestart(req("POST", OPS_RESTART_PATH, "Bearer "), undefined), false);
  assert.equal(await isOpsRestart(req("POST", OPS_RESTART_PATH, "Bearer "), ""), false);
});
