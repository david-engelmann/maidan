// TypeScript worker for the two-language lease demo (Cluster 317). Claims one open
// task off a Maidan channel via the published `maidan` SDK's `claimNextThread`, then
// prints `CLAIM=<thread-id>` (or `CLAIM=null` if the queue is drained/leased). The
// Python orchestrator (lease_demo.py) reads that line. No LLM — just the lease board.
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
const claim = await client.claimNextThread(channel, { member_id: member, lease_secs: 120 });
const id = claim && claim.id ? claim.id : "null";
console.log(`[typescript worker] claimed thread: ${id}`);
console.log(`CLAIM=${id}`);
