import { defineConfig, devices } from "@playwright/test";

// The README and listing screenshots (`npm run capture`). Not part of `npm
// test`: this config has its own seed harness
// (crates/maidan-server/examples/capture_server.rs), its own port, and writes
// PNGs into docs/assets/screens/. See ui-tests/README.md, "Screenshots".
const PORT = process.env.CAPTURE_PORT ?? "8961";
const BASE = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./capture",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  // A shot that does not render fails; retrying would hide that.
  retries: 0,
  reporter: "list",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  use: {
    ...devices["Desktop Chrome"],
    baseURL: BASE,
    // The screenshot bar in docs/UI Design.md: 1440 px wide, light mode.
    viewport: { width: 1440, height: 900 },
    deviceScaleFactor: 1,
    colorScheme: "light",
    locale: "en-US",
    timezoneId: "America/New_York",
    // One raster thread and a fixed color profile: with several, an
    // anti-aliased rounded corner came out one shade apart now and then.
    launchOptions: {
      args: ["--num-raster-threads=1", "--force-color-profile=srgb", "--disable-partial-raster"],
    },
    trace: "retain-on-failure",
    screenshot: "off",
  },
  webServer: {
    command: "cargo run --quiet --example capture_server -p maidan-server",
    cwd: "..",
    url: `${BASE}/ui/`,
    // A fresh seed every run: a reused server would carry the last run's
    // clicks into this run's pictures.
    reuseExistingServer: false,
    timeout: 300_000,
    env: {
      CAPTURE_PORT: PORT,
      CAPTURE_FIXTURES: "ui-tests/.capture.json",
    },
    stdout: "pipe",
    stderr: "pipe",
  },
});
