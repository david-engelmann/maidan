import { defineConfig, devices } from "@playwright/test";

// One shared, seeded server drives all specs (the harness seeds a deterministic
// workspace/channel/thread/gate + a bearer token, in
// crates/maidan-server/examples/ui_test_server.rs).
const PORT = process.env.UI_TEST_PORT ?? "8899";
const BASE = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./tests",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  timeout: 30_000,
  expect: { timeout: 10_000 },
  use: {
    baseURL: BASE,
    trace: "on-first-retry",
    screenshot: "only-on-failure",
    // The console registers a service worker with a fetch handler. Once it
    // has claimed a page, Chromium sends that page's requests through it and
    // page.route never sees them, so whether a stubbed or held request reached
    // its route depended on how fast the worker activated: the flaky-retry
    // family of #2126. No spec tests the worker, and it is unit-tested in
    // web/console/test/pwa.test.ts.
    serviceWorkers: "block",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // CI runs the harness the build job compiled (UI_TEST_SERVER_BIN); locally
    // `cargo run` builds it once. The example writes the fixtures to
    // ui-tests/.fixtures.json (relative to the repo root, cwd below).
    command:
      process.env.UI_TEST_SERVER_BIN ?? "cargo run --quiet --example ui_test_server -p maidan-server",
    cwd: "..",
    url: `${BASE}/ui/`,
    reuseExistingServer: !process.env.CI,
    timeout: 300_000,
    env: {
      UI_TEST_PORT: PORT,
      UI_TEST_FIXTURES: "ui-tests/.fixtures.json",
      MAIDAN_OAUTH_CLIENTS: '[{"client_id": "playwright-consent-client", "name": "Playwright Consent Client", "redirect_uris": ["https://client.example/callback"], "allowed_scopes": ["workspace:read"]}]',
    },
    stdout: "pipe",
    stderr: "pipe",
  },
});
