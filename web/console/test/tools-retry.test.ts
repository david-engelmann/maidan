import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// A Tools panel that fails to load offers Try again, like Needs you does,
// instead of leaving an error with no way forward.
test("a Tools panel that fails to load offers Try again", () => {
  const src = readFileSync(new URL("../src/tools/Tools.tsx", import.meta.url), "utf8");
  const at = src.indexOf("function State<T>");
  assert.ok(at >= 0, "State exists");
  const body = src.slice(at, src.indexOf("\n}\n", at));
  const failed = body.slice(body.indexOf("loaded.value === null"));
  assert.match(failed, /role="alert"/);
  assert.match(failed, />Try again<\/button>/);
  assert.match(failed, /reload\(\)/);
});
