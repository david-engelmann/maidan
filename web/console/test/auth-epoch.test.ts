import { test } from "node:test";
import assert from "node:assert/strict";
import { bumpAuthEpoch, failure, guarded, keepAuth, onAuthLost } from "../src/core/api.ts";

const res401 = () => new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 });

test("a 401 for the current credential signs out; one for an older credential does not", async () => {
  let lost = 0;
  const off = onAuthLost(() => lost++);
  const f = guarded(async () => res401());
  const old = await f("/x");
  bumpAuthEpoch();
  await assert.rejects(failure(old, null));
  assert.equal(lost, 0);
  await assert.rejects(failure(await f("/x"), null));
  assert.equal(lost, 1);
  const kept = await f("/confirm");
  keepAuth(kept);
  await assert.rejects(failure(kept, null));
  assert.equal(lost, 1);
  off();
});
