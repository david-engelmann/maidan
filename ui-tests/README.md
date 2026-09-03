# maidan `/ui` browser tests

Playwright tests that drive the real `/ui` console in **headless Chromium**, so
`/ui` changes never need manual testing.

## How it works

- `crates/maidan-server/examples/ui_test_server.rs` stands up the real
  `maidan-server` router on in-memory SQLite, seeds a deterministic
  workspace / channel / thread / pending approval-gate + a bearer token, writes
  the fixtures to `.fixtures.json`, and serves `/ui/`.
- `playwright.config.ts`'s `webServer` starts that harness, waits for `/ui/`,
  runs the specs, then stops it.
- Specs read `.fixtures.json` (via `tests/_fixtures.ts`) for the base URL,
  bearer token, and seeded ids, then drive the browser and assert the DOM.

## Run locally

Prereqs: the Rust toolchain (to build the harness) + Node.

```sh
cd ui-tests
npm install
npx playwright install --with-deps chromium
npm test              # headless
npm run test:headed   # watch it in a real browser
npm run report        # open the HTML report after a run
```

## Add a test for a new `/ui` feature

1. If the feature needs seeded data, add it in the harness
   (`examples/ui_test_server.rs`) and a field on `Fixtures` in
   `tests/_fixtures.ts`.
2. Add `tests/<feature>.spec.ts`: `goto("/ui/")`, authenticate (`#workspace` +
   `#token` from the fixtures), interact, assert the DOM.
3. `npm test`. The same suite runs in CI (the `ui-tests` job).

**Every `/ui` change should land with a spec here — that is the assurance that
replaces manual UI testing.**
