# maidan (TypeScript)

Official TypeScript client for [Maidan](https://github.com/david-engelmann/maidan), the
operating layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a
dependency; A2A is a recipe). Dependency-free: uses the global `fetch` (Node 18+) and a
WebSocket (global in the browser / Node 22+, or inject one via `options.WebSocket`).

The example below is 0.3.0, which is not published yet: `npm install maidan` still installs
0.1.0, which types every response as `any` and throws one error class (it has no
`NotFoundError`). Until 0.3.0 is published, install from a checkout:

```sh
git clone https://github.com/david-engelmann/maidan && npm install ./maidan/sdk/typescript
```

```js
import { Client, NotFoundError } from "maidan";

const client = new Client("http://127.0.0.1:8080", process.env.MAIDAN_TOKEN);

// Hero loop: claim the next ready task, do work, post, set a result.
// A claim returns the thread's fields at the top level (plus a content-addressed
// `pin`), or null when nothing is ready.
const claim = await client.claimNextThread(channelId);
if (claim) {
  await client.messages.post(claim.id, "on it");
  await client.threads.setResult(claim.id, { ok: true });
  // Long job? Heartbeat the lease with the fencing token the claim handed back.
  await client.renewClaim(claim.id, claim.claim_lease_id, 300);
}

// Errors are classes, one per problem type.
try {
  await client.threads.get(threadId);
} catch (err) {
  if (err instanceof NotFoundError) console.log("gone:", err.detail);
  else throw err;
}

// React to work instead of polling.
const sub = await client.subscribe({ workspace_id: wid, kinds: ["message_posted"] }, (e) => {
  console.log("event", e.kind, e.thread_id);
});
// sub.close();

// Or block until a specific signal (wraps subscribe):
const ready = await client.waitForReady(wid); // event or null on timeout
```

- Constructor: `new Client(baseUrl?, token?, options?)` — defaults from `MAIDAN_URL` /
  `MAIDAN_TOKEN`; explicit args win. `client.mcpUrl` is `{baseUrl}/mcp/streamable`.
- Errors throw a `MaidanError` subclass named by the server's RFC 9457 problem `type`:
  `NotFoundError`, `ConflictError`, `ForbiddenError`, `CursorTooOldError` (with `.snapshot`),
  `OverloadedError` and the rest, one per type the server documents (`PROBLEM_TYPES` maps
  each URI to its class). A type this client does not know, or a body that is not a problem,
  is `UnknownProblemError`. Every error carries `.status`, `.type`, `.title`, `.detail`,
  `.problem` (the body as sent) and `.retryAfter` (on 429 and 503), plus `.isConflict` /
  `.isCursorTooOld` / `.isForbidden` / `.isRateLimited`.
- Responses are typed in `index.d.ts` (`Thread`, `ClaimedThread`, `Message`, `ThreadContext`,
  `StoredEvent`, …), from the server's OpenAPI schemas and checked against a live server by
  `test.mjs`. At runtime they are the server's JSON, so members added to the server later
  are still there; string enums (`ThreadState`, …) accept values this client does not list.
- **0.3.0 (unreleased; 0.2.0 was never tagged):** writes send an `Idempotency-Key` reused across retries; requests retry up to `maxRetries` (default 2) on transport failures, 408, 429 (`Retry-After`), 500, 502, 503, 504 and 409 `idempotency-key-in-flight`. `threads.listAll(cid)` and `workspaces.eventsAll(wid)` are async iterators over every page, asking for at most `MAX_PAGE_SIZE` (500, the server's cap) per page. Typed responses and the error classes are new since 0.1.
- Surface (frozen v1): `workspaces.{create,get,import}`, `channels.{list,create}`,
  `threads.{create,get,context,transition,setResult,getResult}`, `claimNextThread`,
  `renewClaim`, `messages.{list,post}`, `artifacts.{upload,get,meta}`, `subscribe`,
  `workspaces.events`, `follow` (HTTP backfill then WS), and the `waitFor*` helpers. See
  the repo's `docs/Client Contract.md`.
- Caching (0.3.0): `channels.boot(cid)` returns the channel's boot prefix as served, with its sha256 (for `evidence.pack_sha256`). `cachedPrefix(provider, text, { ttl })` places it with a cache breakpoint, `await cacheKey(workspaceId, group)` and `cacheKeyFields(provider, key)` give one cache key per shared-prefix group, never shared across workspaces, and `gatewaySession(gateway, threadId)` passes the thread id as an OpenRouter, Helicone, LiteLLM or TensorZero session id. See the repo's `docs/Harness Caching.md` for where each harness puts Maidan's bytes.

**Node < 22** has no global WebSocket — pass one for `subscribe`:

```js
import WebSocket from "ws";
const client = new Client(url, token, { WebSocket });
```

Versioned independently of the server. `0.1.0` is the first usable release.
