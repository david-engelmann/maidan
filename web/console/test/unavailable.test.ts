import assert from "node:assert/strict";
import { test } from "node:test";
import { failure, humanError } from "../src/core/api.ts";

const said = async (res: Response) => {
  try {
    await failure(res, "session");
  } catch (e) {
    return (e as Error).message;
  }
  throw new Error("failure did not throw");
};

test("a 502, 503 or 504 says the server is busy or restarting, not to read a log", () => {
  for (const status of [502, 503, 504]) {
    const text = humanError(status, null, "session");
    assert.match(text, /busy or restarting/);
    assert.doesNotMatch(text, /server log/);
  }
  assert.match(humanError(500, null, "session"), /server hit an error/);
});

test("a 503 with Retry-After says when, and says Try again once", async () => {
  const text = await said(new Response("", { status: 503, headers: { "Retry-After": "30" } }));
  assert.match(text, /^The server is busy or restarting\. Try again in /);
  assert.equal(text.match(/Try again/g)?.length, 1);
});
