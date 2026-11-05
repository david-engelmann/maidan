import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  ENV_FINGERPRINT_KEY,
  SERVER_ENV,
  containerEnvPlan,
  envFingerprint,
  serverEnv,
} from "../src/server-env.js";

const config = (file) =>
  JSON.parse(
    readFileSync(new URL(`../${file}`, import.meta.url), "utf8")
      .split("\n")
      .filter((line) => !line.trim().startsWith("//"))
      .join("\n"),
  );

test("the fingerprint key is what the container class stores", () => {
  assert.equal(ENV_FINGERPRINT_KEY, "containerEnvFingerprint");
});

test("registration set to 1 is forwarded as the string 1, and every value is a string", async () => {
  for (const file of ["wrangler.jsonc", "wrangler.prod.jsonc"]) {
    const vars = config(file).vars;
    assert.equal(vars.MAIDAN_OAUTH_DYNAMIC_REGISTRATION, "1", file);
    assert.equal(typeof vars.MAIDAN_OAUTH_DCR_REDIRECT_HOSTS, "string", file);
    const env = serverEnv(vars);
    assert.equal(env.MAIDAN_OAUTH_DYNAMIC_REGISTRATION, "1", file);
    assert.equal(env.MAIDAN_OAUTH_DCR_REDIRECT_HOSTS, vars.MAIDAN_OAUTH_DCR_REDIRECT_HOSTS, file);
    assert.equal(env.MAIDAN_PUBLIC_ORIGIN, vars.MAIDAN_PUBLIC_ORIGIN, file);
    for (const value of Object.values(env)) {
      assert.equal(typeof value, "string", file);
    }
  }
});

test("a numeric or boolean binding is stringified so container start can take it", () => {
  const env = serverEnv({
    MAIDAN_OAUTH_DYNAMIC_REGISTRATION: 1,
    MAIDAN_OIDC_ENABLED: true,
    MAIDAN_SIGNUP: false,
  });
  assert.equal(env.MAIDAN_OAUTH_DYNAMIC_REGISTRATION, "1");
  assert.equal(env.MAIDAN_OIDC_ENABLED, "true");
  assert.equal(env.MAIDAN_SIGNUP, "false");
  assert.equal(env.MAIDAN_TRUSTED_PROXY_HOPS, "1");
  assert.equal(env.MAIDAN_ENV, "production");
  assert.equal(env.MAIDAN_BIND, "0.0.0.0:8080");
});

test("an unset registration switch is absent, and a fixed setting wins over a binding", () => {
  const env = serverEnv({ MAIDAN_BIND: "127.0.0.1:1", MAIDAN_PUBLIC_ORIGIN: "https://dev.maidan.world" });
  assert.equal(env.MAIDAN_OAUTH_DYNAMIC_REGISTRATION, undefined);
  assert.equal(env.MAIDAN_BIND, "0.0.0.0:8080");
  assert.equal(env.MAIDAN_PUBLIC_ORIGIN, "https://dev.maidan.world");
  assert.ok(SERVER_ENV.includes("MAIDAN_OAUTH_DYNAMIC_REGISTRATION"));
  assert.ok(SERVER_ENV.includes("MAIDAN_OAUTH_DCR_REDIRECT_HOSTS"));
});

test("a running container whose start predates the current environment is recycled", async () => {
  const before = await envFingerprint(serverEnv({ MAIDAN_PUBLIC_ORIGIN: "https://dev.maidan.world" }));
  const after = await envFingerprint(
    serverEnv({
      MAIDAN_PUBLIC_ORIGIN: "https://dev.maidan.world",
      MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1",
    }),
  );
  assert.notEqual(before, after);
  assert.deepEqual(containerEnvPlan({ running: true, startedFingerprint: before, fingerprint: after }), {
    destroy: true,
    remember: true,
  });
  assert.deepEqual(containerEnvPlan({ running: true, startedFingerprint: undefined, fingerprint: after }), {
    destroy: true,
    remember: true,
  });
});

test("a container already started with this environment is left running", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1" }));
  assert.deepEqual(containerEnvPlan({ running: true, startedFingerprint: fingerprint, fingerprint }), {
    destroy: false,
    remember: false,
  });
  assert.deepEqual(containerEnvPlan({ running: false, startedFingerprint: fingerprint, fingerprint }), {
    destroy: false,
    remember: false,
  });
});

test("a stopped container with a stale environment is started and then remembered", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1" }));
  assert.deepEqual(containerEnvPlan({ running: false, startedFingerprint: "stale", fingerprint }), {
    destroy: false,
    remember: true,
  });
});

test("the same environment fingerprints the same, whatever the key order", async () => {
  const left = await envFingerprint({ B: "1", A: "true" });
  const right = await envFingerprint({ A: "true", B: "1" });
  assert.equal(left, right);
  assert.notEqual(left, await envFingerprint({ A: "true", B: "1", C: "" }));
});

test("the container class applies serverEnv and recycles a stale container before proxying", () => {
  const worker = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  const fetch = worker.slice(worker.indexOf("async fetch(request)"), worker.indexOf("export default"));
  assert.match(worker, /this\.envVars = serverEnv\(env\)/);
  assert.match(fetch, /containerEnvPlan\(/);
  assert.match(fetch, /await this\.destroy\(\)/);
  assert.ok(fetch.indexOf("await this.destroy()") < fetch.indexOf("super.fetch(request)"));
  assert.match(fetch, /plan\.remember && plan\.startedFresh && this\.ctx\.container\.running === true/);
});
