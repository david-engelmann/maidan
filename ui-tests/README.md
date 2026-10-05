# maidan `/ui` browser tests

Playwright tests that drive the real `/ui` board in **headless Chromium**.
They cover the scripted paths. A change under `crates/maidan-server/static/`
still walks the PR template's checklist by hand: the pasted-token path and the
OIDC path, keyboard only, a 390 px viewport, and every new string re-read.

## How it works

- `crates/maidan-server/examples/ui_test_server.rs` stands up the real
  `maidan-server` router on in-memory SQLite, seeds a deterministic
  workspace / channel / thread / pending approval-gate + a bearer token, writes
  the fixtures to `.fixtures.json`, and serves `/ui/`.
- `playwright.config.ts`'s `webServer` starts that harness, waits for `/ui/`,
  runs the specs, then stops it.
- Specs read `.fixtures.json` (via `tests/_fixtures.ts`) for the base URL,
  bearer token, and seeded ids, then drive the browser and assert the DOM.
- A red `ui tests (playwright)` job means the board in the browser does not
  match: a task in the wrong lane, a state pill instead of the word, an
  empty board that is not one sentence and Connect an agent, or a 403 that
  shows the server body. The job is not a required check. Do not skip the
  board to make it green.

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

**Every `/ui` change should land with a spec here.** The specs do not replace
the PR template's checklist, which is still run by hand.

## Coverage checklist

Specs that drive a real page, and what is still missing.

| Area | Spec | Notes |
| --- | --- | --- |
| Board lanes, empty board, human refusal | `board.spec.ts`, `needs-you.spec.ts`, `refused-close.spec.ts`, `calm.spec.ts` | The board as it is |
| First-run sign-in (Enter, Sign in, no credential yet) | `first-run-sign-in.spec.ts` | |
| Prefs (delivery mode, email, mute, follow) | `prefs.spec.ts` | |
| Slash commands (register, revoke) | `slash.spec.ts` | MCP tool handler. An http handler needs an encryption key the harness does not set |
| Delivery replay | `deliveries.spec.ts` | One seeded dead-letter webhook |
| Token mint and revoke | `tokens.spec.ts` | `attenuation.spec.ts` only checks the widening warning |
| DMs | `dms.spec.ts` | Open and post |
| Group DMs | `group-dms.spec.ts` | Open with three members, select, and post |
| Connect an agent | `connect.spec.ts` | |
| Token rotation | `rotate.spec.ts` | |

The board modules live in `crates/maidan-server/static/ui`. `crates/maidan-server/tests/ui_js_checks.rs` runs `node --test crates/maidan-server/static/ui/helpers.test.mjs` and `tsc --noEmit --checkJs` on `crates/maidan-server/static/ui` and on `crates/maidan-server/static/ui/tsconfig.sw.json`. The integration job runs that test, so both checks run in CI. The service worker is a separate project because the board project types `self` as a window.
