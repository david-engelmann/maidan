// What the try.maidan.world demo runs with. Kept apart from demo.js so the
// tests can read it without the Containers runtime.
//
// The demo is a throwaway: SQLite inside the container, empty on boot, and
// wiped by destroying the container on a schedule. Every workspace in it is a
// visitor's sandbox (POST /sandbox, crates/maidan-server/src/sandbox.rs).
// Nothing it holds outlives a reset, and it cannot reach the internet.

// How often the cron destroys the container. The next request boots a fresh
// one. Keep this and wrangler.demo.jsonc's cron in step. Once a day: nothing
// in the demo runs on a timer more often than that.
export const DEMO_RESET_CRON = "0 4 * * *";

// The most sandboxes one container holds before the mint answers 503 until
// the reset. At the 4 MiB stored-bytes allowance below, that is under 1 GiB
// of visitor content on a basic instance's disk.
export const DEMO_SANDBOX_MAX = 200;

// The server's own environment. Sign-up stays off (MAIDAN_SIGNUP unset, which
// the sandbox mint refuses beside it anyway) and the build has no bootstrap
// routes. Rate limits are tighter than production, and the free allowance is
// a hard cutoff for every sandbox (each is an `open` workspace), so one
// visitor can neither spam through the demo nor fill its disk.
export const DEMO_SERVER_ENV = {
  MAIDAN_BIND: "0.0.0.0:8080",
  MAIDAN_ENV: "production",
  MAIDAN_LOG_FORMAT: "json",
  MAIDAN_TRUSTED_PROXY_HOPS: "1",
  DATABASE_URL: "sqlite:///tmp/maidan-demo.db?mode=rwc",
  ARTIFACT_BACKEND: "localfs",
  ARTIFACT_LOCALFS_ROOT: "/tmp/maidan-demo-artifacts",
  // The per-visitor sandbox, and the marker production mode needs beside it.
  MAIDAN_DEMO_SANDBOX: "1",
  MAIDAN_DEMO_INSTANCE: "1",
  MAIDAN_DEMO_SANDBOX_MAX: String(DEMO_SANDBOX_MAX),
  MAIDAN_DEMO_SANDBOX_RATE_LIMIT_MAX: "3",
  MAIDAN_DEMO_SANDBOX_RATE_LIMIT_WINDOW_SECS: "3600",
  MAIDAN_RATE_LIMIT_MAX: "60",
  MAIDAN_RATE_LIMIT_WINDOW_SECS: "60",
  MAIDAN_WORKSPACE_RATE_LIMIT_MAX: "120",
  MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS: "60",
  MAIDAN_OAUTH_IP_RATE_LIMIT_MAX: "10",
  MAIDAN_OAUTH_IP_RATE_LIMIT_WINDOW_SECS: "60",
  // A hard cutoff per sandbox (docs/Operations.md, "The hard cutoff for open
  // sign-up workspaces"): far more than the Say hello task needs, far less
  // than a host for anything else.
  MAIDAN_FREE_ALLOWANCE: "requests=5000,mcp_calls=2000,stored_bytes=4194304",
  MAIDAN_MAX_BODY_BYTES: "65536",
  MAIDAN_ARTIFACT_MAX_BODY_BYTES: "65536",
};

// Request bodies above this are refused at the edge before they wake the
// container.
export const DEMO_MAX_BODY_BYTES = 64 * 1024;

// Fail closed at the demo edge: a missing, non-decimal, or chunked body
// length is treated as oversized so a visitor cannot sneak a large upload
// past the gate by omitting Content-Length or sending Transfer-Encoding.
export function tooLarge(request) {
  const method = (request.method || "GET").toUpperCase();
  if (method === "GET" || method === "HEAD" || method === "OPTIONS" || method === "TRACE") {
    return false;
  }
  const transfer = (request.headers.get("transfer-encoding") ?? "").trim().toLowerCase();
  if (transfer && transfer !== "identity") {
    return true;
  }
  const raw = request.headers.get("content-length");
  if (raw === null || raw.trim() === "") {
    return true;
  }
  const trimmed = raw.trim();
  if (!/^\d+$/.test(trimmed)) {
    return true;
  }
  const length = Number(trimmed);
  return length > DEMO_MAX_BODY_BYTES;
}
