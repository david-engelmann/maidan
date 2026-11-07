// Fault injection: a lost or cut-short claim retries under the same
// Idempotency-Key, and a claim the client cannot read is an error.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import http from "node:http";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { Client, MaidanError, UnreadableResponseError } from "./index.js";

const HERE = path.dirname(fileURLToPath(import.meta.url));
const PROXY = path.resolve(HERE, "../../scripts/mcp-fault-proxy.py");

function client(fetch, opts = {}) {
  return new Client("http://x", "t", { fetch, sleep: async () => {}, ...opts });
}

test("a cut-short claim retries with the same key and is not claimed twice", async () => {
  const keys = [];
  let claims = 0;
  const fetch = async (_url, init) => {
    const key = init.headers["idempotency-key"];
    keys.push(key);
    if (keys.length === 1) {
      claims += 1;
      return new Response('{"id":"thr', {
        status: 200,
        headers: { "content-type": "application/json", "content-length": "80" },
      });
    }
    return new Response(
      JSON.stringify({ id: "thr", claim_lease_id: "lease", state: "open", channel_id: "c" }),
      { status: 200, headers: { "content-type": "application/json" } },
    );
  };
  const claim = await client(fetch).claimNextThread("c");
  assert.equal(claim.id, "thr");
  assert.equal(claim.claim_lease_id, "lease");
  assert.equal(keys.length, 2);
  assert.equal(keys[0], keys[1]);
  assert.equal(claims, 1);
});

test("a non-JSON 200 on claim retries, then is UnreadableResponseError", async () => {
  let n = 0;
  const keys = [];
  const fetch = async (_url, init) => {
    keys.push(init.headers["idempotency-key"]);
    n += 1;
    return new Response("<html>proxy</html>", {
      status: 200,
      headers: { "content-type": "text/html", "content-length": "18" },
    });
  };
  await assert.rejects(() => client(fetch).claimNextThread("c"), (err) => {
    assert.ok(err instanceof UnreadableResponseError);
    assert.ok(err instanceof MaidanError);
    assert.equal(err.status, 200);
    return true;
  });
  assert.equal(n, 3);
  assert.equal(keys[0], keys[1]);
  assert.equal(keys[1], keys[2]);
});

test("502 and 503 with Retry-After, and 409 in flight, recover", async () => {
  const answers = [
    [502, "bad gateway", { "retry-after": "0" }],
    [503, { type: "https://maidan.world/problems/overloaded" }, { "retry-after": "0" }],
    [409, { type: "https://maidan.world/problems/idempotency-key-in-flight" }],
    [200, { id: "thr", claim_lease_id: "lease", state: "open", channel_id: "c" }],
  ];
  const keys = [];
  const fetch = async (_url, init) => {
    keys.push(init.headers["idempotency-key"]);
    const [status, body, headers] = answers.shift();
    const text = typeof body === "string" ? body : JSON.stringify(body);
    return new Response(text, { status, headers });
  };
  const claim = await client(fetch, { maxRetries: 4 }).claimNextThread("c");
  assert.equal(claim.id, "thr");
  assert.equal(new Set(keys).size, 1);
});

test("an exhausted transport failure is MaidanError status 0", async () => {
  const fetch = async () => {
    throw new TypeError("terminated");
  };
  await assert.rejects(() => client(fetch).claimNextThread("c"), (err) => {
    assert.ok(err instanceof MaidanError);
    assert.equal(err.status, 0);
    assert.equal(err.cause.message, "terminated");
    return true;
  });
});

test("a drifted claim is UnreadableResponseError", async () => {
  for (const body of [{}, { id: 1, claim_lease_id: "lease" }, { id: "thr" }]) {
    const fetch = async () =>
      new Response(JSON.stringify(body), {
        status: 200,
        headers: { "content-type": "application/json" },
      });
    await assert.rejects(() => client(fetch, { maxRetries: 0 }).claimNextThread("c"), (err) => {
      assert.ok(err instanceof UnreadableResponseError && err instanceof MaidanError, `${err}`);
      return true;
    });
  }
});

function listen(handler) {
  return new Promise((resolve) => {
    const server = http.createServer(handler);
    server.listen(0, "127.0.0.1", () => resolve(server));
  });
}

function idempotentMaidan() {
  const claims = new Map();
  return listen((req, res) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const key = req.headers["idempotency-key"] || "";
      if (req.method === "POST" && req.url.includes("/threads/claim-next")) {
        if (!claims.has(key)) {
          claims.set(key, {
            id: `thr-${claims.size + 1}`,
            claim_lease_id: `lease-${claims.size + 1}`,
            state: "open",
            channel_id: "c",
          });
        }
        const body = JSON.stringify(claims.get(key));
        res.writeHead(200, {
          "content-type": "application/json",
          "content-length": Buffer.byteLength(body),
        });
        res.end(body);
        return;
      }
      res.writeHead(404);
      res.end();
    });
    req.claims = claims;
  }).then((server) => ({
    server,
    url: `http://127.0.0.1:${server.address().port}`,
    claimsOf: () => {
      // The map lives in the closure; expose it by walking isn't possible.
      return claims;
    },
  }));
}

function startProxy(upstream, plan) {
  return new Promise(async (resolve, reject) => {
    const dir = await mkdtemp(path.join(tmpdir(), "maidan-fault-"));
    const planPath = path.join(dir, "plan.json");
    await writeFile(planPath, JSON.stringify(plan));
    const child = spawn(
      "python3",
      [PROXY, "--upstream", upstream, "--listen", "127.0.0.1:0", "--plan", planPath],
      { stdio: ["ignore", "pipe", "pipe"] },
    );
    let buf = "";
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error(`proxy did not start: ${buf}`));
    }, 5000);
    child.stderr.on("data", (chunk) => {
      buf += chunk.toString();
      const match = buf.match(/fault proxy on (http:\/\/\S+)/);
      if (match) {
        clearTimeout(timer);
        resolve({
          url: match[1],
          close: () =>
            new Promise((done) => {
              child.once("exit", () => done());
              child.kill();
              rm(dir, { recursive: true, force: true }).catch(() => {});
            }),
        });
      }
    });
    child.on("error", reject);
  });
}

test("drop_after through the fault proxy claims one thread", async () => {
  const upstream = await idempotentMaidan();
  const proxy = await startProxy(upstream.url, [
    { path: "/threads/claim-next", times: 1, fault: { kind: "drop_after" } },
  ]);
  try {
    const c = new Client(proxy.url, "t", { sleep: async () => {}, maxRetries: 2 });
    const claim = await c.claimNextThread("c");
    assert.equal(claim.claim_lease_id, "lease-1");
    assert.equal(upstream.claimsOf().size, 1);
  } finally {
    await proxy.close();
    upstream.server.close();
  }
});

test("a mid-body cut through the fault proxy retries onto the stored claim", async () => {
  const upstream = await idempotentMaidan();
  const proxy = await startProxy(upstream.url, [
    {
      path: "/threads/claim-next",
      times: 1,
      fault: { kind: "truncate", bytes: 10, keep_length: true },
    },
  ]);
  try {
    const c = new Client(proxy.url, "t", { sleep: async () => {}, maxRetries: 2 });
    const claim = await c.claimNextThread("c");
    assert.equal(claim.id, "thr-1");
    assert.equal(upstream.claimsOf().size, 1);
  } finally {
    await proxy.close();
    upstream.server.close();
  }
});
