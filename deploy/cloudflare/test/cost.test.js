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
  for (const file of ["wrangler.prod.jsonc", "wrangler.jsonc", "wrangler.swarm.jsonc"]) {
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

// David's cost rule for try.maidan.world: one basic instance, asleep after 15
// idle minutes, and a once-a-day cron that only destroys a running container,
// never wakes one.

const wrangler = config("wrangler.demo.jsonc");
const worker = readFileSync(new URL("../src/demo.js", import.meta.url), "utf8");
const scheduled = worker.slice(worker.indexOf("async scheduled("));
const reset = worker.slice(worker.indexOf("async reset()"), worker.indexOf("export default"));

test("the demo runs one basic instance", () => {
  assert.equal(wrangler.containers.length, 1);
  assert.equal(wrangler.containers[0].max_instances, 1);
  assert.equal(wrangler.containers[0].instance_type, "basic");
});

test("an idle demo sleeps after 15 minutes", () => {
  assert.match(worker, /sleepAfter = "15m";/);
});

test("the cron only destroys: it never fetches, starts or wakes the container", () => {
  assert.ok(scheduled.startsWith("async scheduled("));
  assert.match(scheduled, /\.reset\(\)/);
  assert.doesNotMatch(scheduled, /\.(fetch|start|startAndWaitForPorts|containerFetch)\(/);
  assert.match(reset, /if \(this\.ctx\.container\.running\) await this\.destroy\(\);/);
  assert.doesNotMatch(reset, /\.(fetch|start|startAndWaitForPorts|containerFetch)\(/);
});

test("the only timer is the reset cron, and it fires once a day", () => {
  assert.equal(wrangler.triggers.crons.length, 1);
  const [min, hour, dom, mon, dow] = wrangler.triggers.crons[0].split(" ");
  assert.match(min, /^\d+$/);
  assert.match(hour, /^\d+$/);
  assert.deepEqual([dom, mon, dow], ["*", "*", "*"]);
  assert.doesNotMatch(worker, /setInterval|setAlarm|storage\.setAlarm/);
});

test("the try page is answered before the container is reached, so visiting it never wakes one", () => {
  const fetchHandler = worker.slice(worker.indexOf("async fetch("), worker.indexOf("async scheduled("));
  const own = fetchHandler.indexOf("handleTry(");
  const container = fetchHandler.indexOf("getContainer(");
  assert.ok(own > 0 && container > own, "handleTry runs before getContainer");
  assert.equal(wrangler.assets?.binding, "ASSETS");
  assert.equal(wrangler.assets?.directory, "./try");
});
