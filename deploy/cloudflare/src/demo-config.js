// What the try.maidan.world demo runs with. Kept apart from demo.js so the
// tests can read it without the Containers runtime.
//
// The demo is a throwaway: SQLite inside the container, seeded on boot by
// scripts/demo-board.sh, and wiped by destroying the container on a schedule.
// Nothing it holds outlives a reset, and it cannot reach the internet.

// How often the cron destroys the container. The next request boots a fresh
// one, which reseeds. Keep this and wrangler.demo.jsonc's cron in step.
export const DEMO_RESET_CRON = "0 */6 * * *";

// Seeded tokens expire a little after the next reset would have wiped them, so
// a missed reset still leaves no live token older than this.
export const DEMO_TOKEN_TTL_SECS = 7 * 60 * 60;

// Sign-up stays off (MAIDAN_SIGNUP unset), and bootstrap is never on for the
// public listener: demo/entrypoint.sh turns it on only for the seed, on a
// loopback port, then restarts without it.
//
// The server's own environment. Sign-up stays off (MAIDAN_SIGNUP unset) and
// bootstrap is never on for the public listener: demo/entrypoint.sh turns it
// on only for the seed, on a loopback port, then restarts without it.
// The server's own environment. Rate limits are tighter than production and
// the free allowance meters every workspace, so one visitor can't use the
// demo up for the rest.
export const DEMO_SERVER_ENV = {
  MAIDAN_BIND: "0.0.0.0:8080",
  MAIDAN_ENV: "production",
  MAIDAN_LOG_FORMAT: "json",
  MAIDAN_TRUSTED_PROXY_HOPS: "1",
  DATABASE_URL: "sqlite:///tmp/maidan-demo.db?mode=rwc",
  ARTIFACT_BACKEND: "localfs",
  ARTIFACT_LOCALFS_ROOT: "/tmp/maidan-demo-artifacts",
  MAIDAN_RATE_LIMIT_MAX: "60",
  MAIDAN_RATE_LIMIT_WINDOW_SECS: "60",
  MAIDAN_OAUTH_IP_RATE_LIMIT_MAX: "10",
  MAIDAN_OAUTH_IP_RATE_LIMIT_WINDOW_SECS: "60",
  // Meters on every workspace (docs/Operations.md, "The free allowance").
  MAIDAN_FREE_ALLOWANCE: "requests=20000,mcp_calls=10000,stored_bytes=67108864",
  MAIDAN_ARTIFACT_MAX_BODY_BYTES: "1048576",
  DEMO_TOKEN_TTL_SECS: String(DEMO_TOKEN_TTL_SECS),
};

// Request bodies above this are refused at the edge before they wake the
// container.
export const DEMO_MAX_BODY_BYTES = 64 * 1024;

export function tooLarge(request) {
  const length = Number(request.headers.get("content-length") ?? "0");
  return Number.isFinite(length) && length > DEMO_MAX_BODY_BYTES;
}
