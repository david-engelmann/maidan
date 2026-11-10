import { defineConfig, devices } from "@playwright/test";

// The docs' motion clips (`scripts/make-motion.sh`). Kept apart from both
// other configs: `npm test` runs ./tests against a local seeded server in CI,
// and `npm run capture` runs every *.spec.ts here against the capture seed.
// This one records the public demo, try.maidan.world, over the network, so
// its spec is named motion.capture.ts, which neither of their globs matches.
export default defineConfig({
  testDir: ".",
  testMatch: "motion.capture.ts",
  fullyParallel: false,
  workers: 1,
  // A clip that does not come out right is rerun by a person who looks at it,
  // not retried into a different take.
  retries: 0,
  reporter: "list",
  // The demo boots a container on first use, every step crosses the network,
  // and the spec rests a minute before each clip so the demo's rate limits
  // never show on screen: a whole take runs about ten minutes.
  timeout: 900_000,
  expect: { timeout: 30_000 },
  use: {
    ...devices["Desktop Chrome"],
    baseURL: process.env.MOTION_BASE_URL ?? "https://try.maidan.world",
    // A high-density screen, so the spec's 1.5x frames are rendered, not
    // stretched; make-motion.sh scales them down to 960 px.
    viewport: { width: 1280, height: 800 },
    deviceScaleFactor: 2,
    locale: "en-US",
    timezoneId: "America/New_York",
    launchOptions: { args: ["--force-color-profile=srgb", "--hide-scrollbars"] },
    // The console's service worker would hold the first paint of a route on
    // its cache; a take wants what the server answers now.
    serviceWorkers: "block",
    trace: "retain-on-failure",
    screenshot: "off",
    video: "off",
  },
});
