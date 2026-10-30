import { defineConfig, devices } from "@playwright/test";

// The review gallery (`npm run gallery`): every first screen of the landing
// and the console at a desktop and a phone width, for a person to look at
// before a UI change merges. It reuses the capture seed
// (crates/maidan-server/examples/capture_server.rs) on its own port and writes
// PNGs plus report.json into gallery-out/, never into docs/. See
// ui-tests/README.md, "Review gallery".
const PORT = process.env.GALLERY_PORT ?? "8962";
const BASE = `http://127.0.0.1:${PORT}`;

export default defineConfig({
  testDir: "./gallery",
  fullyParallel: false,
  workers: 1,
  forbidOnly: !!process.env.CI,
  retries: 0,
  reporter: "list",
  timeout: 60_000,
  expect: { timeout: 15_000 },
  use: {
    ...devices["Desktop Chrome"],
    baseURL: BASE,
    deviceScaleFactor: 1,
    colorScheme: "light",
    locale: "en-US",
    timezoneId: "America/New_York",
    trace: "retain-on-failure",
    screenshot: "off",
  },
  webServer: {
    command: "cargo run --quiet --example capture_server -p maidan-server",
    cwd: "..",
    url: `${BASE}/ui/`,
    reuseExistingServer: false,
    timeout: 300_000,
    env: {
      CAPTURE_PORT: PORT,
      CAPTURE_FIXTURES: "ui-tests/.gallery.json",
    },
    stdout: "pipe",
    stderr: "pipe",
  },
});
