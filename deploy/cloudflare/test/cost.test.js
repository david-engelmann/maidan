import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const config = (file) =>
  JSON.parse(
    readFileSync(new URL(`../${file}`, import.meta.url), "utf8")
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n"),
  );

// Each cron wake keeps a container (and Neon's compute) up for sleepAfter, so
// a schedule more often than daily bills an idle instance for nothing.
test("the hosted instances wake at most once a day on their own", () => {
  for (const file of ["wrangler.prod.jsonc", "wrangler.jsonc"]) {
    const crons = config(file).triggers?.crons ?? [];
    assert.ok(crons.length <= 1, `${file}: one cron at most`);
    for (const cron of crons) {
      const [minute, hour, dom, month, dow] = cron.split(/\s+/);
      for (const field of [minute, hour]) assert.match(field, /^\d+$/, `${file}: "${cron}" fires more than once a day`);
      assert.deepEqual([dom, month, dow], ["*", "*", "*"], `${file}: "${cron}"`);
    }
  }
});

test("prod runs at most one basic container, never more without a decision", () => {
  const prod = config("wrangler.prod.jsonc");
  assert.deepEqual(
    prod.containers.map((c) => [c.instance_type, c.max_instances]),
    [["basic", 1]],
  );
});
