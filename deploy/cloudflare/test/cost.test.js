import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

// David's cost rule for try.maidan.world: one basic instance, asleep after 15
// idle minutes, and a cron that only destroys a running container, never wakes one.

const wrangler = JSON.parse(
  readFileSync(new URL("../wrangler.demo.jsonc", import.meta.url), "utf8")
    .split("\n")
    .filter((line) => !line.trim().startsWith("//"))
    .join("\n"),
);
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

test("the only timer is the reset cron, and it fires at most every 6 hours", () => {
  assert.equal(wrangler.triggers.crons.length, 1);
  const [min, hour, dom, mon, dow] = wrangler.triggers.crons[0].split(" ");
  assert.match(min, /^\d+$/);
  assert.ok(/^\*\/(\d+)$/.test(hour) ? Number(hour.slice(2)) >= 6 : /^[\d,]+$/.test(hour));
  assert.deepEqual([dom, mon, dow], ["*", "*", "*"]);
  assert.doesNotMatch(worker, /setInterval|setAlarm|storage\.setAlarm/);
});
