import { test } from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { internalOutboundByHost, parseInternalHosts, routedEgressHosts } from "../src/edge.js";
import { serverEnv } from "../src/server-env.js";

const binding = (name) => ({
  calls: [],
  async fetch(request) {
    this.calls.push(request);
    return new Response(`${name} ${new URL(request.url).pathname}`);
  },
});

test("only .internal hosts with a declared binding are mapped", () => {
  const env = { HOOKS: binding("hooks"), WORKER: binding("worker"), MAIDAN: {} };
  assert.deepEqual(
    parseInternalHosts(
      " Hooks.Internal. = HOOKS , worker.internal=WORKER, github.com=HOOKS, " +
        "missing.internal=NOPE, durable.internal=MAIDAN, internal=HOOKS, .internal=HOOKS, " +
        "a..internal=HOOKS, *.internal=HOOKS, a.internal:80=HOOKS, x.internal=SE-ATS, " +
        "y.internal=HOOKS=WORKER, internal.example.com=HOOKS, hooks.internal=WORKER, " +
        "bare.internal, ,",
      env,
    ),
    [
      ["hooks.internal", "HOOKS"],
      ["worker.internal", "WORKER"],
    ],
  );
  assert.deepEqual(parseInternalHosts(undefined, env), []);
  assert.deepEqual(parseInternalHosts("hooks.internal=HOOKS", {}), []);
});

test("a mapped host is forwarded to its binding, and nothing else is intercepted", async () => {
  const env = { HOOKS: binding("hooks") };
  const handlers = internalOutboundByHost("hooks.internal=HOOKS,worker.internal=WORKER", env);
  // An unmapped host gets no handler, so the container's own egress serves it.
  assert.deepEqual(Object.keys(handlers), ["hooks.internal"]);
  for (const host of ["worker.internal", "api.github.com", "slack.com", "maidan.r2.cloudflarestorage.com"]) {
    assert.equal(handlers[host], undefined, host);
  }
  const request = new Request("http://hooks.internal/wake", { method: "POST", body: "{}" });
  const response = await handlers["hooks.internal"](request, env, { containerId: "c", className: "MaidanServer" });
  assert.equal(await response.text(), "hooks /wake");
  assert.equal(env.HOOKS.calls[0], request);
});

test("the server is handed only the listed names the Worker routes", () => {
  const env = { HOOKS: binding("hooks") };
  const map = "hooks.internal=HOOKS,worker.internal=WORKER";
  assert.equal(routedEgressHosts("hooks.internal", map, env), "hooks.internal");
  // A listed name with no route, or no declared binding, is dropped.
  assert.equal(
    routedEgressHosts(" Hooks.Internal. ,worker.internal,other.internal,hooks.internal", map, env),
    "hooks.internal",
  );
  assert.equal(routedEgressHosts("worker.internal", map, env), undefined);
  assert.equal(routedEgressHosts("hooks.internal", "", env), undefined);
  assert.equal(routedEgressHosts(undefined, map, env), undefined);
});

test("no map means no interception at all", () => {
  assert.deepEqual(internalOutboundByHost("", { HOOKS: binding("hooks") }), {});
  assert.deepEqual(internalOutboundByHost(undefined, {}), {});
});

// `outboundByHost` is a static accessor on Container that registers the map by
// class name; a static class field would shadow it and register nothing.
test("the Worker assigns the map through the accessor", () => {
  const worker = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.doesNotMatch(worker, /static\s+outboundByHost\s*=/);
  assert.match(worker, /MaidanServer\.outboundByHost = /);
  assert.match(worker, /export \{ ContainerProxy \} from "@cloudflare\/containers";/);
});

test("the container's environment carries only the routed internal names", () => {
  const env = {
    MAIDAN_INTERNAL_HOSTS: "hooks.internal=HOOKS",
    MAIDAN_INTERNAL_EGRESS_HOSTS: "hooks.internal,other.internal",
    HOOKS: binding("hooks"),
  };
  assert.equal(serverEnv(env).MAIDAN_INTERNAL_EGRESS_HOSTS, "hooks.internal");
  // The map itself and the binding stay in the Worker.
  assert.equal(serverEnv(env).MAIDAN_INTERNAL_HOSTS, undefined);
  assert.equal(serverEnv(env).HOOKS, undefined);
  const unrouted = { MAIDAN_INTERNAL_EGRESS_HOSTS: "hooks.internal" };
  assert.equal("MAIDAN_INTERNAL_EGRESS_HOSTS" in serverEnv(unrouted), false);
});

const config = (file) =>
  JSON.parse(
    readFileSync(new URL(`../${file}`, import.meta.url), "utf8")
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n"),
  );

// Internal hosts belong to the operator of an instance, not to this repo: no
// Wrangler config here declares a service binding or an internal host, so a
// deploy from the repo intercepts nothing.
test("no Wrangler config in the repo declares services or internal hosts", () => {
  const files = readdirSync(new URL("../", import.meta.url)).filter((f) => /^wrangler\..*jsonc$/.test(f));
  assert.ok(files.includes("wrangler.prod.jsonc"), files.join(", "));
  for (const file of files) {
    const cfg = config(file);
    assert.equal(cfg.services, undefined, file);
    assert.equal(cfg.vars?.MAIDAN_INTERNAL_HOSTS, undefined, file);
    assert.equal(cfg.vars?.MAIDAN_INTERNAL_EGRESS_HOSTS, undefined, file);
  }
});

// What an operator's own config looks like: each mapped name on a binding the
// same config declares, and the egress list naming only mapped names.
test("an operator config's internal hosts resolve against its own bindings", () => {
  const operator = {
    services: [
      { binding: "HOOKS", service: "example-hooks" },
      { binding: "WORKER", service: "example-worker" },
    ],
    vars: {
      MAIDAN_INTERNAL_HOSTS: "hooks.internal=HOOKS,worker.internal=WORKER,stale.internal=GONE",
      MAIDAN_INTERNAL_EGRESS_HOSTS: "hooks.internal,worker.internal,stale.internal",
    },
  };
  const declared = Object.fromEntries(operator.services.map((s) => [s.binding, { fetch() {} }]));
  assert.deepEqual(parseInternalHosts(operator.vars.MAIDAN_INTERNAL_HOSTS, declared), [
    ["hooks.internal", "HOOKS"],
    ["worker.internal", "WORKER"],
  ]);
  assert.equal(
    routedEgressHosts(operator.vars.MAIDAN_INTERNAL_EGRESS_HOSTS, operator.vars.MAIDAN_INTERNAL_HOSTS, declared),
    "hooks.internal,worker.internal",
  );
});
