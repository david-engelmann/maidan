import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import {
  ENV_FINGERPRINT_HEADER,
  ENV_FINGERPRINT_KEY,
  SERVER_ENV,
  ContainerEnvSession,
  containerEnvPlan,
  envFingerprint,
  serverEnv,
  withoutFingerprint,
} from "../src/server-env.js";
import { demoEnv } from "../src/demo-env.js";

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

test("every result sender's credential reaches the container", () => {
  const credentials = [
    "MAIDAN_SLACK_BOT_TOKEN",
    "MAIDAN_LINEAR_API_KEY",
    "MAIDAN_GITHUB_TOKEN",
    "MAIDAN_GITHUB_APP_ID",
    "MAIDAN_GITHUB_APP_INSTALLATION_ID",
    "MAIDAN_GITHUB_APP_PRIVATE_KEY",
  ];
  const env = serverEnv(Object.fromEntries(credentials.map((name) => [name, `${name}-value`])));
  for (const name of credentials) {
    assert.ok(SERVER_ENV.includes(name), name);
    assert.equal(env[name], `${name}-value`, name);
  }
});

test("the GitHub ingress secret and the CI backstop reach the container", () => {
  // With the secret the change sweep reads CI only when a GitHub CI event
  // says it changed; the backstop is how long it waits without one.
  const env = serverEnv({
    MAIDAN_GITHUB_WEBHOOK_SECRET: "whsec",
    MAIDAN_CHANGE_CI_BACKSTOP_SECS: 900,
  });
  assert.equal(env.MAIDAN_GITHUB_WEBHOOK_SECRET, "whsec");
  assert.equal(env.MAIDAN_CHANGE_CI_BACKSTOP_SECS, "900");
});

test("the workflow-path refusal reaches the container", () => {
  // A hosted delivery token without the Workflows permission is the case
  // the switch is for: the builder hears at once instead of after the push.
  const env = serverEnv({ MAIDAN_CHANGE_REFUSE_WORKFLOW_PATHS: "1" });
  assert.equal(env.MAIDAN_CHANGE_REFUSE_WORKFLOW_PATHS, "1");
});

test("SERVER_ENV is sorted, unique, and forwards rotation keys and integration secrets", () => {
  const sorted = [...SERVER_ENV].sort();
  assert.deepEqual(SERVER_ENV, sorted);
  assert.equal(new Set(SERVER_ENV).size, SERVER_ENV.length);
  for (const name of [
    "FEDERATION_DECRYPT_KEYS",
    "FEDERATION_ENCRYPTION_KEY",
    "MAIDAN_CONTENT_KEK",
    "MAIDAN_CONTENT_KEK_PREVIOUS",
    "MAIDAN_EXPORT_SIGNING_KEY",
    "MAIDAN_EXPORT_VERIFY_KEYS",
    "MAIDAN_SLACK_BOT_TOKEN",
    "MAIDAN_SLACK_SIGNING_SECRET",
    "MAIDAN_SUBSCRIBE_RESUME_SECRET",
    "MAIDAN_SESSION_SECRET",
    "MAIDAN_GITHUB_WEBHOOK_SECRET",
    "MAIDAN_SMTP_PASSWORD",
  ]) {
    assert.ok(SERVER_ENV.includes(name), name);
  }
  assert.ok(SERVER_ENV.every((name) => !name.endsWith("_FILE")));
});

test("every wrangler var is forwarded or is the internal-host map", () => {
  for (const file of ["wrangler.jsonc", "wrangler.prod.jsonc"]) {
    for (const name of Object.keys(config(file).vars)) {
      const kept = SERVER_ENV.includes(name) || name === "MAIDAN_INTERNAL_HOSTS";
      assert.ok(kept, `${file} drops ${name}`);
    }
  }
});

test("insecure and demo switches on the binding never reach the container", () => {
  const env = serverEnv({
    MAIDAN_ALLOW_INSECURE_DEV_KEK: "1",
    MAIDAN_ALLOW_INSECURE_NO_AUTH: "1",
    MAIDAN_ALLOW_PRIVATE_EGRESS: "1",
    MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE: "1",
    MAIDAN_OIDC_MOCK: "1",
    MAIDAN_OPS_FAULT: "1",
    MAIDAN_BOOTSTRAP: "1",
    MAIDAN_DEMO_SANDBOX: "1",
    MAIDAN_DEMO_INSTANCE: "1",
    MAIDAN_CONTENT_KEK_FILE: "/secrets/kek",
    MAIDAN_SESSION_SECRET: "session-secret-at-least-32-bytes!!",
    FEDERATION_DECRYPT_KEYS: "aa",
    MAIDAN_CONTENT_KEK_PREVIOUS: "bb",
    MAIDAN_EXPORT_VERIFY_KEYS: "cc",
    MAIDAN_SLACK_SIGNING_SECRET: "slack",
    MAIDAN_SUBSCRIBE_RESUME_SECRET: "resume-secret-at-least-32-bytes!!",
  });
  for (const name of [
    "MAIDAN_ALLOW_INSECURE_DEV_KEK",
    "MAIDAN_ALLOW_INSECURE_NO_AUTH",
    "MAIDAN_ALLOW_PRIVATE_EGRESS",
    "MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE",
    "MAIDAN_OIDC_MOCK",
    "MAIDAN_OPS_FAULT",
    "MAIDAN_BOOTSTRAP",
    "MAIDAN_DEMO_SANDBOX",
    "MAIDAN_DEMO_INSTANCE",
    "MAIDAN_CONTENT_KEK_FILE",
  ]) {
    assert.equal(env[name], undefined, name);
  }
  assert.equal(env.MAIDAN_SESSION_SECRET, "session-secret-at-least-32-bytes!!");
  assert.equal(env.FEDERATION_DECRYPT_KEYS, "aa");
  assert.equal(env.MAIDAN_CONTENT_KEK_PREVIOUS, "bb");
  assert.equal(env.MAIDAN_EXPORT_VERIFY_KEYS, "cc");
  assert.equal(env.MAIDAN_SLACK_SIGNING_SECRET, "slack");
  assert.equal(env.MAIDAN_SUBSCRIBE_RESUME_SECRET, "resume-secret-at-least-32-bytes!!");
  assert.equal(env.MAIDAN_ENV, "production");
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

test("the container class delegates environment changes to the session", () => {
  const worker = readFileSync(new URL("../src/index.js", import.meta.url), "utf8");
  const fetch = worker.slice(worker.indexOf("async fetch(request)"), worker.indexOf("export default"));
  assert.match(worker, /this\.baseEnv = serverEnv\(env\)/);
  // The process starts with the hash of its settings and echoes it.
  assert.match(fetch, /envFingerprint\(this\.baseEnv\)/);
  assert.match(fetch, /this\.envVars = \{ \.\.\.this\.baseEnv, \[ENV_FINGERPRINT_VAR\]: fingerprint \}/);
  assert.match(worker, /this\.envSession = new ContainerEnvSession\(\)/);
  assert.match(fetch, /this\.envSession\.serve\(/);
  assert.match(fetch, /forward: \(\) => super\.fetch\(request\)/);
});

// Serializes the Durable Object input lock. The request itself runs after the
// callback, which is when a second request overlaps a start.
function createLock() {
  let tail = Promise.resolve();
  return (fn) => {
    const run = tail.then(() => fn());
    tail = run.then(
      () => {},
      () => {},
    );
    return run;
  };
}

function memoryStorage(seed) {
  const values = new Map(seed);
  return {
    gets: 0,
    puts: 0,
    async get(key) {
      this.gets += 1;
      return values.get(key);
    },
    async put(key, value) {
      this.puts += 1;
      values.set(key, value);
    },
    snapshot(key) {
      return values.get(key);
    },
  };
}

function deferred() {
  let release;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  return { gate, release };
}

function rig({ fingerprint, running = false, stored }) {
  const session = new ContainerEnvSession();
  const container = { running };
  const storage = memoryStorage(stored === undefined ? [] : [[ENV_FINGERPRINT_KEY, stored]]);
  const locks = [];
  const lock = createLock();
  const blockConcurrencyWhile = (fn) => {
    locks.push(1);
    return lock(fn);
  };
  let destroys = 0;
  const destroy = async () => {
    destroys += 1;
    container.running = false;
  };
  return {
    session,
    container,
    storage,
    locks,
    destroy,
    destroys: () => destroys,
    serve(forward) {
      return session.serve({
        fingerprint,
        container,
        storage,
        blockConcurrencyWhile,
        destroy,
        forward,
      });
    },
  };
}

test("two requests cold-starting together leave the container up and record it once it is running", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1" }));
  const world = rig({ fingerprint });
  const inside = deferred();
  const release = deferred();

  const first = world.serve(async () => {
    world.container.running = true;
    inside.release();
    await release.gate;
    return "a";
  });
  await inside.gate;
  const second = world.serve(async () => "b");
  const secondResult = await second;
  release.release();
  const firstResult = await first;

  assert.equal(firstResult, "a");
  assert.equal(secondResult, "b");
  assert.equal(world.destroys(), 0);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.session.startingFingerprint, null);
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.equal(world.container.running, true);
});

test("a recycle in progress is not destroyed again by a request that arrives mid-start", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1" }));
  const world = rig({ fingerprint, running: true, stored: "from-the-previous-start" });
  const inside = deferred();
  const release = deferred();

  const first = world.serve(async () => {
    assert.equal(world.container.running, false, "the stale container is stopped before the replacement starts");
    world.container.running = true;
    inside.release();
    await release.gate;
    return "a";
  });
  await inside.gate;
  assert.equal(world.destroys(), 1);
  const second = await world.serve(async () => "b");
  release.release();
  assert.equal(await first, "a");
  assert.equal(second, "b");
  assert.equal(world.destroys(), 1);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.session.startingFingerprint, null);
});

test("a start that throws or never runs is not recorded, and the next request tries again", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1" }));

  const thrown = rig({ fingerprint });
  await assert.rejects(
    thrown.serve(async () => {
      throw new Error("container failed to start");
    }),
    /container failed to start/,
  );
  assert.equal(thrown.storage.snapshot(ENV_FINGERPRINT_KEY), undefined);
  assert.equal(thrown.storage.puts, 0);
  assert.equal(thrown.session.inFlightStarts, 0);
  assert.equal(thrown.session.startingFingerprint, null);
  assert.equal(thrown.session.verifiedFingerprint, null);
  const retried = await thrown.serve(async () => {
    thrown.container.running = true;
    return "retried";
  });
  assert.equal(retried, "retried");
  assert.equal(thrown.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
  assert.equal(thrown.destroys(), 0);

  const quiet = rig({ fingerprint });
  assert.equal(
    await quiet.serve(async () => {
      quiet.container.running = false;
      return "down";
    }),
    "down",
  );
  assert.equal(quiet.storage.snapshot(ENV_FINGERPRINT_KEY), undefined);
  assert.equal(quiet.storage.puts, 0);
  assert.equal(quiet.session.inFlightStarts, 0);
  assert.equal(quiet.session.startingFingerprint, null);
  assert.equal(quiet.session.verifiedFingerprint, null);
  assert.equal(
    await quiet.serve(async () => {
      quiet.container.running = true;
      return "up";
    }),
    "up",
  );
  assert.equal(quiet.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
});

test("a warm container whose environment is already verified skips the lock and the stored fingerprint", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_OAUTH_DYNAMIC_REGISTRATION: "1" }));
  const world = rig({ fingerprint, running: true, stored: fingerprint });
  world.session.verifiedFingerprint = fingerprint;
  const answer = await world.serve(async () => "warm");
  assert.equal(answer, "warm");
  assert.equal(world.locks.length, 0);
  assert.equal(world.storage.gets, 0);
  assert.equal(world.storage.puts, 0);
  assert.equal(world.destroys(), 0);

  await world.serve(async () => {
    world.container.running = false;
    return "slept";
  });
  assert.equal(world.session.verifiedFingerprint, null);
  assert.equal(world.locks.length, 0, "the request that finds it running still skips the lock");

  world.container.running = false;
  await world.serve(async () => {
    world.container.running = true;
    return "woke";
  });
  assert.ok(world.locks.length > 0);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
});

test("a daily reset drops verified state so the next request starts with the current fingerprint", async () => {
  const fingerprint = await envFingerprint(demoEnv("https://try.maidan.world"));
  const world = rig({ fingerprint, running: true, stored: fingerprint });
  world.session.verifiedFingerprint = fingerprint;

  world.session.invalidate();
  if (world.container.running) await world.destroy();

  assert.equal(world.session.verifiedFingerprint, null);
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.session.startingFingerprint, null);
  assert.equal(world.session.epoch, 1);
  assert.equal(world.destroys(), 1);
  assert.equal(world.container.running, false);
  // The fingerprint of the destroyed process stays stored. The container is
  // not running, so the next request must start it, not destroy it again.
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);

  const answer = await world.serve(async () => {
    world.container.running = true;
    return "fresh";
  });
  assert.equal(answer, "fresh");
  assert.equal(world.destroys(), 1);
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.session.startingFingerprint, null);
});

test("after a reset, a running container already on this fingerprint is kept and re-checked", async () => {
  const fingerprint = await envFingerprint(demoEnv("https://try.maidan.world"));
  const world = rig({ fingerprint, running: true, stored: fingerprint });
  world.session.verifiedFingerprint = fingerprint;
  world.session.invalidate();

  const answer = await world.serve(async () => "kept");
  assert.equal(answer, "kept");
  assert.equal(world.destroys(), 0);
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.ok(world.locks.length > 0, "verified memory was cleared, so the request checks storage");
});

test("after a reset, a running container on a stale fingerprint is recycled once", async () => {
  const before = await envFingerprint(demoEnv("https://try.maidan.world"));
  const after = await envFingerprint(demoEnv("https://preview.example"));
  assert.notEqual(before, after);
  const world = rig({ fingerprint: after, running: true, stored: before });
  world.session.verifiedFingerprint = before;
  world.session.invalidate();

  const answer = await world.serve(async () => {
    assert.equal(world.container.running, false, "the stale container is stopped before the replacement starts");
    world.container.running = true;
    return "recycled";
  });
  assert.equal(answer, "recycled");
  assert.equal(world.destroys(), 1);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), after);
  assert.equal(world.session.verifiedFingerprint, after);
  assert.equal(world.session.inFlightStarts, 0);
});

test("a request in flight during a reset does not record the destroyed container or clobber the next start", async () => {
  const fingerprint = await envFingerprint(demoEnv("https://try.maidan.world"));
  const world = rig({ fingerprint, running: true, stored: fingerprint });
  world.session.verifiedFingerprint = fingerprint;
  const inside = deferred();
  const release = deferred();

  const first = world.serve(async () => {
    inside.release();
    await release.gate;
    return "old";
  });
  await inside.gate;
  assert.equal(world.locks.length, 0, "a verified container skips the lock");
  assert.equal(world.session.inFlightStarts, 0);

  world.session.invalidate();
  if (world.container.running) await world.destroy();
  assert.equal(world.destroys(), 1);
  assert.equal(world.session.verifiedFingerprint, null);

  const secondInside = deferred();
  const secondRelease = deferred();
  const second = world.serve(async () => {
    assert.equal(world.container.running, false);
    world.container.running = true;
    secondInside.release();
    await secondRelease.gate;
    return "new";
  });
  await secondInside.gate;
  assert.equal(world.destroys(), 1, "the fresh start is not recycled");
  assert.equal(world.session.inFlightStarts, 1);
  assert.equal(world.session.verifiedFingerprint, null, "not recorded until the start finishes");

  release.release();
  assert.equal(await first, "old");
  assert.equal(world.session.inFlightStarts, 1, "the request from before the reset must not decrement the new start");
  assert.equal(world.session.verifiedFingerprint, null);
  assert.equal(world.storage.puts, 0);

  secondRelease.release();
  assert.equal(await second, "new");
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.session.startingFingerprint, null);
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
  assert.equal(world.destroys(), 1);
});

test("a cold start still in flight when the demo resets does not record itself, and its error still surfaces", async () => {
  const fingerprint = await envFingerprint(demoEnv("https://try.maidan.world"));
  const world = rig({ fingerprint });
  const inside = deferred();
  const release = deferred();

  const first = world.serve(async () => {
    world.container.running = true;
    inside.release();
    await release.gate;
    throw new Error("container failed to start");
  });
  await inside.gate;
  assert.equal(world.session.inFlightStarts, 1);

  world.session.invalidate();
  world.container.running = false;
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.session.startingFingerprint, null);

  const second = world.serve(async () => {
    world.container.running = true;
    return "new";
  });
  assert.equal(await second, "new");
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.equal(world.session.inFlightStarts, 0);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);

  release.release();
  await assert.rejects(first, /container failed to start/);
  assert.equal(world.session.inFlightStarts, 0, "the failed start does not decrement the replacement");
  assert.equal(world.session.verifiedFingerprint, fingerprint);
  assert.equal(world.session.startingFingerprint, null);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
  assert.equal(world.destroys(), 0);
});

function answer(fingerprint) {
  return new Response("ok", { headers: { [ENV_FINGERPRINT_HEADER]: fingerprint } });
}

test("a container that answers with another start's environment is recycled on the next request", async () => {
  // v445: the rollout restarted the new image on the previous start's
  // settings, so MAIDAN_SIGNUP_OPEN never reached the process.
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_SIGNUP_OPEN: "1" }));
  const previous = await envFingerprint(serverEnv({}));
  const world = rig({ fingerprint });
  const stale = await world.serve(async () => {
    world.container.running = true;
    return answer(previous);
  });
  assert.equal(await stale.text(), "ok", "the answer still goes back");
  assert.equal(world.destroys(), 0, "its body is not cut off mid-answer");
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), `stale:${previous}`);

  await world.serve(async () => {
    world.container.running = true;
    return answer(fingerprint);
  });
  assert.equal(world.destroys(), 1, "the next request recycles it");
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
});

test("a verified container that later answers with another environment is recycled too", async () => {
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_SIGNUP_OPEN: "1" }));
  const world = rig({ fingerprint, running: true, stored: fingerprint });
  await world.serve(async () => answer(fingerprint));
  assert.equal(world.destroys(), 0);
  await world.serve(async () => answer("b".repeat(64)));
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), `stale:${"b".repeat(64)}`);
  await world.serve(async () => {
    world.container.running = true;
    return answer(fingerprint);
  });
  assert.equal(world.destroys(), 1);
});

test("an answer without the header is taken as before, so an older image is not recycled for it", async () => {
  const fingerprint = await envFingerprint(serverEnv({}));
  const world = rig({ fingerprint, running: true, stored: fingerprint });
  await world.serve(async () => new Response("ok"));
  await world.serve(async () => new Response("ok"));
  assert.equal(world.destroys(), 0);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint);
});

test("no answer leaves the Worker with the fingerprint header, matching or stale", async () => {
  // OPS-003: the fingerprint is an unsalted hash over every setting, secrets
  // included; a client that sees it can test a guessed secret offline.
  const fingerprint = await envFingerprint(serverEnv({ MAIDAN_SIGNUP_OPEN: "1" }));
  const world = rig({ fingerprint });
  const first = await world.serve(async () => {
    world.container.running = true;
    return answer(fingerprint);
  });
  assert.equal(first.headers.get(ENV_FINGERPRINT_HEADER), null);
  assert.equal(await first.text(), "ok");
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), fingerprint, "still compared first");

  const warm = await world.serve(async () => answer(fingerprint));
  assert.equal(warm.headers.get(ENV_FINGERPRINT_HEADER), null);

  const stale = await world.serve(async () => answer("c".repeat(64)));
  assert.equal(stale.headers.get(ENV_FINGERPRINT_HEADER), null);
  assert.equal(world.storage.snapshot(ENV_FINGERPRINT_KEY), `stale:${"c".repeat(64)}`);
});

test("taking the fingerprint off keeps the status, the other headers and the body", async () => {
  const original = new Response("created", {
    status: 201,
    statusText: "Created",
    headers: { [ENV_FINGERPRINT_HEADER]: "a".repeat(64), "x-request-id": "r1", "content-type": "text/plain" },
  });
  const sent = withoutFingerprint(original);
  assert.equal(sent.status, 201);
  assert.equal(sent.statusText, "Created");
  assert.equal(sent.headers.get("x-request-id"), "r1");
  assert.equal(sent.headers.get("content-type"), "text/plain");
  assert.equal(sent.headers.has(ENV_FINGERPRINT_HEADER), false);
  assert.equal(await sent.text(), "created");

  const plain = new Response("ok");
  assert.equal(withoutFingerprint(plain), plain, "an answer without it is passed through as is");
  assert.equal(withoutFingerprint("not a response"), "not a response");
});
