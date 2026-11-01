# maidan console browser tests

Playwright tests that drive the real console at `/ui` in **headless Chromium**.

## How it works

- `crates/maidan-server/examples/ui_test_server.rs` stands up the real
  server with a seeded workspace, channels, threads and tokens, writes
  the fixtures to `.fixtures.json`, and serves `/ui/`.
- `playwright.config.ts`'s `webServer` starts that harness, waits for `/ui/`,
  runs the specs, then stops it.
- Specs read `.fixtures.json` (via `tests/_fixtures.ts`) for the base URL,
  tokens, the session cookie and seeded ids, then drive the browser.
  `tests/_console.ts` signs in with the session cookie and opens the console
  on a channel.
- The console's own logic (board lanes, needs you, the service worker policy)
  is unit tested in `web/console/test` with `npm test` there. These specs
  check that the built console works against a real server.

## Run locally

Prereqs: the Rust toolchain (to build the harness) + Node. Build the console
first (`cd web/console && npm ci && npm run build`): the server embeds it.

```sh
cd ui-tests
npm install
npx playwright install --with-deps chromium
npm test              # headless
npm run test:headed   # watch it in a real browser
npm run report        # open the HTML report after a run
```

## Screenshots

`npm run capture` writes the README and docs screenshots into
`docs/assets/screens/` from `crates/maidan-server/examples/capture_server.rs`
(`capture.config.ts`: 1440 px wide, light mode, a fixed clock). Each shot
asserts what it must show first, so a screen that does not render fails and
its PNG is not written.

## Coverage

| Area | Spec |
| --- | --- |
| Loads under its CSP with no page errors; `/console` redirects to `/ui/` | `console.spec.ts` |
| Signed-out sign-in: a refused token, a pasted token | `console.spec.ts` |
| The board's four lanes on `#build` | `console.spec.ts` |
| Needs you | `console.spec.ts` |
| OAuth consent lands on the client redirect | `oauth-consent.spec.ts` |

**Every console change that reaches the server should land with a spec here.**

## Review gallery

`npm run gallery` takes every first screen of the landing and the console at
a desktop (1440 x 900) and a phone (390 x 844) width into `gallery-out/`:
the landing (and the landing under reduced motion), the signed-out console,
the board with work in flight, Needs you with a review waiting, a task sheet,
the Connect tab and an empty channel. It reuses the capture seed on port 8962
(`GALLERY_PORT`) and never writes into `docs/`.

The pictures are for a person to look at before a UI change merges. Two
things are checked so no one has to look for them: a screen that scrolls
sideways at its own width, and a page that throws. Either fails the run.
`gallery-out/report.json` lists every screen with what was measured.

The `ui gallery` CI job runs it on every pull request that changes code and
uploads `gallery-out/` as the `ui-gallery` artifact, pass or fail.
