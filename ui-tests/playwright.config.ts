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
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    // `cargo run` reuses the binary if a prior CI step already built it, so this
    // is instant in CI and a one-time build locally. The example writes the
    // fixtures to ui-tests/.fixtures.json (relative to the repo root, cwd below).
    command: "cargo run --quiet --example ui_test_server -p maidan-server",
    cwd: "..",
    url: `${BASE}/ui/`,
    reuseExistingServer: !process.env.CI,
    timeout: 300_000,
    env: {
      UI_TEST_PORT: PORT,
      UI_TEST_FIXTURES: "ui-tests/.fixtures.json",
    },
    stdout: "pipe",
    stderr: "pipe",
  },
});
