// TypeScript worker for the two-language lease demo. It claims one task, runs the
// fenced lifecycle, proves a third claim is null while both workers hold leases,
// and releases in `finally`. The Python orchestrator reads the tagged JSON result.
//
// In production: `npm i maidan` and `import { Client } from "maidan"`. Here we import
// the in-tree SDK so the demo is self-contained.
import { Client } from "../../sdk/typescript/index.js";

const base = process.env.MAIDAN_URL || "http://127.0.0.1:8080";
const token = process.env.MAIDAN_TOKEN || undefined;
const channel = process.env.MAIDAN_CHANNEL;
const member = process.env.MAIDAN_MEMBER;

if (!channel || !member) {
  console.error("MAIDAN_CHANNEL and MAIDAN_MEMBER are required");
  process.exit(2);
}

const client = new Client(base, token);

async function post(path, body) {
  const headers = { "content-type": "application/json" };
  if (token) headers.authorization = `Bearer ${token}`;
  const response = await fetch(`${base}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) throw new Error(`POST ${path} failed: HTTP ${response.status}: ${text}`);
  return text ? JSON.parse(text) : null;
}

let claim;
let drained = false;
let released = false;
try {
  claim = await client.claimNextThread(channel, { member_id: member, lease_secs: 120 });
  if (!claim?.id || !claim.claim_lease_id) throw new Error("expected a fenced claim");
  console.log(`[typescript worker] claimed thread: ${claim.id}`);

  const holder = { member_id: member, claim_lease_id: claim.claim_lease_id };
  const acknowledged = await post(`/threads/${claim.id}/claim/acknowledge`, holder);
  if (!acknowledged.work_started_at) throw new Error("acknowledge did not start the working clock");
  const usage = await post(`/threads/${claim.id}/usage`, { tokens: 80, turns: 1 });
  if (usage.stopped) throw new Error(`unexpected budget stop: ${JSON.stringify(usage)}`);
  const renewed = await client.renewClaim(claim.id, member, claim.claim_lease_id, 300);
  if (!renewed.assignment_expires_at) throw new Error("renew did not preserve a finite lease");
  console.log("[typescript worker] acknowledged, reported usage, renewed");

  const third = await client.claimNextThread(channel, { member_id: member, lease_secs: 120 });
  drained = third === null;
  console.log(`[typescript worker] third claim (queue held): ${third?.id ?? "null"}`);
  if (!drained) throw new Error(`third claim must be null, got ${third.id}`);
} finally {
  if (claim?.id && claim.claim_lease_id) {
    const returned = await post(`/threads/${claim.id}/claim/release`, {
      member_id: member,
      claim_lease_id: claim.claim_lease_id,
    });
    released = returned.assignee_id == null;
    console.log("[typescript worker] released claim on exit");
  }
}

console.log(`RESULT=${JSON.stringify({ claim_id: claim.id, drained, released })}`);
console.log("[typescript worker] clean exit");
