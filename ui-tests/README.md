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

## Screenshots

`npm run capture` writes the screens in the screenshot bar of
[`docs/UI Design.md`](../docs/UI%20Design.md) to
[`docs/assets/screens/`](../docs/assets/screens/). The README hero is
`board-waiting.png`.

```sh
cd ui-tests
npm run capture                          # writes docs/assets/screens/*.png
CAPTURE_OUT=/some/dir npm run capture    # writes somewhere else
```

- `capture.config.ts` starts its own seed harness,
  `crates/maidan-server/examples/capture_server.rs`, on port 8961
  (`CAPTURE_PORT`), never reusing a running one. The harness seeds three
  one-channel workspaces (`#build` with work in flight, the same board with a
  review waiting on David, and an empty channel) and takes the refused close
  through the real route. The cast is agents named by their role and the
  maintainer; the tasks are this repo's own recent work.
- `capture/capture.spec.ts` signs in the way the quickstart does (workspace id
  and token), checks what the bar says each screen must show, then saves the
  PNG. A screen that does not render that way fails the run, and its PNG is
  not written. Do not loosen a check to get a picture.
- **Deterministic.** The harness fixes member, workspace, channel and thread
  ids (an avatar's color comes from its id, and the Connect sheet and the
  evidence root print ids) and moves every timestamp onto a fixed clock,
  2026-10-06 14:40 New York, which the browser clock is set to. The viewport
  (1440 x 900), scale (1), locale, timezone and color scheme are fixed in the
  config, and animations are finished before each shot. Two runs on one
  machine write byte-identical files; check with
  `CAPTURE_OUT=/tmp/a npm run capture && CAPTURE_OUT=/tmp/b npm run capture && cmp`
  over the eight files. The port is in two screens, so a different
  `CAPTURE_PORT` changes them.
- **Fonts come from the OS** (the board uses `system-ui`), so a retake on
  another OS changes every pixel of text. Retake on Linux, with the browser
  `npx playwright install --with-deps chromium` installs.

**The retake rule.** A PR that changes `crates/maidan-server/static/` runs
`npm run capture` and commits any PNG that changed, so the README and the
listing pages show the console as it is. A PNG that changed when the PR did not
mean to change that screen is a regression to look at, not noise to commit.

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
| Reviews that name no reviewer, closed without review | `unassigned-review.spec.ts` | |
| First-run sign-in (Enter, Sign in, no credential yet) | `first-run-sign-in.spec.ts` | |
| Needs you when a load fails (refused, unreachable, recovery) | `needs-you-truth.spec.ts` | |
| Needs you blocked tasks (Unblock, its failures, a second workspace) | `needs-you-blocked.spec.ts` | Blocks the seeded `hold` task itself and clears it after each test |
| Needs you questions (an agent's `needs_input`, its heading, Answer and the reply that clears it) | `needs-you-question.spec.ts` | Asks the question on the seeded `ask` task itself, as the deployer; the reply clears it |
| The approval card's evidence (packet root, result, artifacts, empty, failed reads, root sent, decider, a second workspace) | `approval-evidence.spec.ts` | Seeded `proof` tasks; the decide test approves `proof_decide`, and Rae approves `proof_live` |
| Prefs (delivery mode, email, mute, follow) | `prefs.spec.ts` | |
| Slash commands (register, revoke) | `slash.spec.ts` | MCP tool handler. An http handler needs an encryption key the harness does not set |
| Delivery replay | `deliveries.spec.ts` | One seeded dead-letter webhook |
| Token mint and revoke | `tokens.spec.ts` | `attenuation.spec.ts` only checks the widening warning |
| DMs | `dms.spec.ts`, `member-picker.spec.ts` | Pick a member by name, open and post; search, badges, keyboard, and a second workspace that sees nothing of the first |
| Group DMs | `group-dms.spec.ts`, `member-picker.spec.ts` | Pick two members as chips, open, select, and post; fewer than three refused before the request |
| Connect an agent | `connect.spec.ts` | |
| Token rotation | `rotate.spec.ts` | |

The board modules live in `crates/maidan-server/static/ui`. `crates/maidan-server/tests/ui_js_checks.rs` runs `node --test crates/maidan-server/static/ui/helpers.test.mjs` and `tsc --noEmit --checkJs` on `crates/maidan-server/static/ui` and on `crates/maidan-server/static/ui/tsconfig.sw.json`. The integration job runs that test, so both checks run in CI. The service worker is a separate project because the board project types `self` as a window.
