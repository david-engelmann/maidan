// TypeScript worker for the two-language lease demo. It claims one task, runs the
// fenced lifecycle, proves a third claim is null while both workers hold leases,
// and releases in `finally`. The Python orchestrator reads the tagged JSON result.
//
// In production: `npm i maidan` and `import { Client } from "maidan"`. Here we import
// the in-tree SDK so the demo is self-contained.
import { Client } from "../../sdk/typescript/index.js";

const base = (process.env.MAIDAN_URL || "http://127.0.0.1:8080").trim().replace(/\/+$/, "");
const token = process.env.MAIDAN_TOKEN || "";
const channel = process.env.MAIDAN_CHANNEL;

function loopback(hostname) {
  const host = (hostname || "").replace(/^\[|\]$/g, "").toLowerCase();
  return host === "localhost" || host === "::1" || host.startsWith("127.");
}

if (token && process.env.MAIDAN_ALLOW_INSECURE_HTTP !== "1") {
  let parsed;
  try {
    parsed = new URL(base);
  } catch {
    parsed = null;
  }
  if (parsed && parsed.protocol === "http:" && !loopback(parsed.hostname)) {
    console.error(
      `refusing to send MAIDAN_TOKEN over cleartext to ${base}; use https, or set MAIDAN_ALLOW_INSECURE_HTTP=1 for a trusted network`,
    );
    process.exit(2);
  }
}

if (!channel || !token) {
  console.error("MAIDAN_CHANNEL and MAIDAN_TOKEN are required");
  process.exit(2);
}

const client = new Client(base, token);

async function post(path, body) {
  const headers = {
    "content-type": "application/json",
    "user-agent": "maidan-lease-demo-js/0.1",
    "idempotency-key": crypto.randomUUID(),
  };
  if (token) headers.authorization = `Bearer ${token}`;
  const response = await fetch(`${base}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(30_000),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`POST ${path} failed: HTTP ${response.status}: ${text}`);
  return text ? JSON.parse(text) : null;
}

let claim;
let drained = false;
let released = false;
let failed = false;
try {
  claim = await client.claimNextThread(channel, { lease_secs: 120 });
  if (!claim?.id || !claim.claim_lease_id) throw new Error("expected a fenced claim");
  console.log(`[typescript worker] claimed thread: ${claim.id}`);

  const holder = { claim_lease_id: claim.claim_lease_id };
  const acknowledged = await post(`/threads/${claim.id}/claim/acknowledge`, holder);
  if (!acknowledged.work_started_at) throw new Error("acknowledge did not start the working clock");
  const usage = await post(`/threads/${claim.id}/usage`, {
    usage_report_id: crypto.randomUUID(),
    claim_lease_id: claim.claim_lease_id,
    model: "demo-model",
    tokens: { input: 80, output: 0, cache_read: 0, cache_write_5m: 0, cache_write_1h: 0 },
    usd_micros: 0,
    price_snapshot: {
      input_usd_micros_per_million: 0,
      output_usd_micros_per_million: 0,
      cache_read_usd_micros_per_million: 0,
      cache_write_5m_usd_micros_per_million: 0,
      cache_write_1h_usd_micros_per_million: 0,
    },
    turns: 1,
  });
  if (usage.stopped) throw new Error(`unexpected budget stop: ${JSON.stringify(usage)}`);
  const renewed = await client.renewClaim(claim.id, claim.claim_lease_id, 300);
  if (!renewed.assignment_expires_at) throw new Error("renew did not preserve a finite lease");
  console.log("[typescript worker] acknowledged, reported usage, renewed");

  const third = await client.claimNextThread(channel, { lease_secs: 120 });
  drained = third === null;
  console.log(`[typescript worker] third claim (queue held): ${third?.id ?? "null"}`);
  if (!drained) throw new Error(`third claim must be null, got ${third.id}`);
} catch (error) {
  failed = true;
  throw error;
} finally {
  if (claim?.id && claim.claim_lease_id) {
    try {
      const returned = await post(`/threads/${claim.id}/claim/release`, {
        claim_lease_id: claim.claim_lease_id,
      });
      released = returned.assignee_id == null;
      console.log("[typescript worker] released claim on exit");
    } catch (error) {
      if (!failed) throw error;
      console.error(`warning: release failed: ${error}`);
    }
  }
}

console.log(`RESULT=${JSON.stringify({ claim_id: claim.id, drained, released })}`);
console.log("[typescript worker] clean exit");
