import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { internalOutboundByHost, parseInternalHosts, routedEgressHosts } from "../src/edge.js";

const binding = (name) => ({
  calls: [],
  async fetch(request) {
    this.calls.push(request);
    return new Response(`${name} ${new URL(request.url).pathname}`);
  },
});

test("only .internal hosts with a declared binding are mapped", () => {
  const env = { SEATS: binding("seats"), CONTROL: binding("control"), MAIDAN: {} };
  assert.deepEqual(
    parseInternalHosts(
      " Seats.Internal. = SEATS , control.internal=CONTROL, github.com=SEATS, " +
        "missing.internal=NOPE, durable.internal=MAIDAN, internal=SEATS, .internal=SEATS, " +
        "a..internal=SEATS, *.internal=SEATS, a.internal:80=SEATS, x.internal=SE-ATS, " +
        "y.internal=SEATS=CONTROL, internal.example.com=SEATS, seats.internal=CONTROL, " +
        "bare.internal, ,",
      env,
    ),
    [
      ["seats.internal", "SEATS"],
      ["control.internal", "CONTROL"],
    ],
  );
  assert.deepEqual(parseInternalHosts(undefined, env), []);
  assert.deepEqual(parseInternalHosts("seats.internal=SEATS", {}), []);
});

test("a mapped host is forwarded to its binding, and nothing else is intercepted", async () => {
  const env = { SEATS: binding("seats") };
  const handlers = internalOutboundByHost("seats.internal=SEATS,control.internal=CONTROL", env);
  // An unmapped host gets no handler, so the container's own egress serves it.
  assert.deepEqual(Object.keys(handlers), ["seats.internal"]);
  for (const host of ["control.internal", "api.github.com", "slack.com", "maidan.r2.cloudflarestorage.com"]) {
    assert.equal(handlers[host], undefined, host);
  }
  const request = new Request("http://seats.internal/wake", { method: "POST", body: "{}" });
  const response = await handlers["seats.internal"](request, env, { containerId: "c", className: "MaidanServer" });
  assert.equal(await response.text(), "seats /wake");
  assert.equal(env.SEATS.calls[0], request);
});

test("the server is handed only the listed names the Worker routes", () => {
  const env = { SEATS: binding("seats") };
  const map = "seats.internal=SEATS,control.internal=CONTROL";
  assert.equal(routedEgressHosts("seats.internal", map, env), "seats.internal");
  // A listed name with no route, or no declared binding, is dropped.
  assert.equal(
    routedEgressHosts(" Seats.Internal. ,control.internal,other.internal,seats.internal", map, env),
    "seats.internal",
  );
  assert.equal(routedEgressHosts("control.internal", map, env), undefined);
  assert.equal(routedEgressHosts("seats.internal", "", env), undefined);
  assert.equal(routedEgressHosts(undefined, map, env), undefined);
});

test("no map means no interception at all", () => {
  assert.deepEqual(internalOutboundByHost("", { SEATS: binding("seats") }), {});
  assert.deepEqual(internalOutboundByHost(undefined, {}), {});
});

// `outboundByHost` is a static accessor on Container that registers the map by
// class name; a static class field would shadow it and register nothing.
test("the Worker assigns the map through the accessor and passes the egress list on", () => {
  const worker = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  assert.doesNotMatch(worker, /static\s+outboundByHost\s*=/);
  assert.match(worker, /MaidanServer\.outboundByHost = /);
  assert.match(worker, /export \{ ContainerProxy \} from "@cloudflare\/containers";/);
  assert.match(worker, /"MAIDAN_INTERNAL_EGRESS_HOSTS",/);
});

const config = (file) =>
  JSON.parse(
    readFileSync(new URL(`../${file}`, import.meta.url), "utf8")
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n"),
  );

test("only the swarm declares internal hosts, each on a declared binding", () => {
  const swarm = config("wrangler.swarm.jsonc");
  const declared = Object.fromEntries((swarm.services ?? []).map((s) => [s.binding, { fetch() {} }]));
  assert.deepEqual(swarm.services, [{ binding: "SEATS", service: "maidan-wax-seats" }]);
  assert.deepEqual(parseInternalHosts(swarm.vars.MAIDAN_INTERNAL_HOSTS, declared), [["seats.internal", "SEATS"]]);
  assert.equal(swarm.vars.MAIDAN_INTERNAL_EGRESS_HOSTS, "seats.internal");
  for (const file of ["wrangler.prod.jsonc", "wrangler.jsonc"]) {
    const other = config(file);
    assert.equal(other.services, undefined, file);
    assert.equal(other.vars?.MAIDAN_INTERNAL_HOSTS, undefined, file);
    assert.equal(other.vars?.MAIDAN_INTERNAL_EGRESS_HOSTS, undefined, file);
  }
});
