// Request body size gates at the Worker edge, before a container wakes.
//
// The hosted server's default `MAIDAN_MAX_BODY_BYTES` is 2 MiB. Refusing a
// larger Content-Length here keeps an attacker from paying the cold-start
// cost for a body the process would only reject. Chunked bodies without a
// length still reach the container, which enforces the same cap as it reads.

/** Default hosted edge body cap; keep in step with MAIDAN_MAX_BODY_BYTES. */
export const HOSTED_MAX_BODY_BYTES = 2 * 1024 * 1024;

/** True when Content-Length is present and above `maxBytes`. */
export function tooLarge(request, maxBytes = HOSTED_MAX_BODY_BYTES) {
  const length = Number(request.headers.get("content-length") ?? "0");
  return Number.isFinite(length) && length > maxBytes;
}
