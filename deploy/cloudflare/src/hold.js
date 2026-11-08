// Holding a request at the edge while the container is replaced.
//
// A deploy or an operator restart stops the one container and starts its
// replacement in the same Durable Object, so for a while there is nothing to
// hand a request to. Answering 503 at once turned every deploy into an outage
// for every client. Instead the Worker keeps the request, asks the container's
// `/health` until something answers, and then sends the request again, up to
// `HOLD_MS`; only after that does the client get the 503.
//
// A request is sent again only when that cannot do something twice: it reads
// (GET, HEAD, OPTIONS), or the runtime said it never reached a server. Its
// body is replayed from a copy, so only a body small enough to copy is held.
// Kept apart from the Worker so it is testable without the Workers runtime.

import { containerUnavailableKind } from "./edge.js";

// Long enough for a cold start against a waking Neon plus the server's boot
// (13 to 47 seconds on prod's deploys, 2026-11-06/08), short of a client
// that gives up first.
export const HOLD_MS = 90_000;

// The first wait, and the longest, between two `/health` probes.
export const FIRST_WAIT_MS = 500;
export const LONGEST_WAIT_MS = 4_000;

// The largest body copied so it can be sent twice. A larger one, or one of
// unknown length, is sent once and not held.
export const REPLAY_BODY_LIMIT = 1024 * 1024;

const READS = new Set(["GET", "HEAD", "OPTIONS"]);

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

// The request's body as bytes to send again, `null` when it has none, or
// `undefined` when it cannot be copied (too large, or of unknown length).
export async function replayableBody(request) {
  if (READS.has(request.method) || request.body === null) {
    return null;
  }
  const length = Number(request.headers.get("content-length"));
  if (!Number.isFinite(length) || length < 0 || length > REPLAY_BODY_LIMIT
      || request.headers.get("content-length") === null) {
    return undefined;
  }
  return await request.arrayBuffer();
}

// One try: the answer, or how the container failed to give one. `kind` is
// null for a real answer, else "unreached" or "unknown"
// (`containerUnavailableKind`).
async function attempt(send) {
  try {
    const response = await send();
    return { response, kind: await containerUnavailableKind(response) };
  } catch (err) {
    console.error("container fetch failed", err);
    return { response: null, kind: "unknown" };
  }
}

// Whether a request that failed with `kind` may be sent again.
export function maySendAgain(method, kind, body) {
  if (body === undefined) {
    return false;
  }
  return READS.has(method) || kind === "unreached";
}

// Send the request; while the container is unavailable, wait for `/health`
// to answer and send it again, until `holdMs` has passed. Returns the
// container's answer, or null when there was none in time (the caller sends
// its 503).
//
// `send()` sends the request (a fresh copy each time), `probe()` asks the
// container's `/health`. `now` and `wait` are the clock, for tests.
export async function sendHeld({
  method,
  body,
  send,
  probe,
  holdMs = HOLD_MS,
  now = Date.now,
  wait = sleep,
}) {
  let tried = await attempt(send);
  if (tried.kind === null) {
    return tried.response;
  }
  const start = now();
  let pause = FIRST_WAIT_MS;
  while (maySendAgain(method, tried.kind, body) && now() - start < holdMs) {
    await wait(Math.min(pause, Math.max(0, holdMs - (now() - start))));
    pause = Math.min(pause * 2, LONGEST_WAIT_MS);
    const health = await attempt(probe);
    await health.response?.body?.cancel();
    if (health.kind !== null) {
      continue;
    }
    // Something answers again: send the request itself.
    tried = await attempt(send);
    if (tried.kind === null) {
      return tried.response;
    }
  }
  return null;
}
