# Cluster 353 retro — the identity chrome (session + capabilities in the /ui)

Wave 1 #4. The first three Wave-1 rows gave the room its mechanics — a durable
approval gate (350), occupancy clocks (351), and A2A gate discovery (352). This
cluster is the **human-facing chrome** for that machinery: an operator (or an
occupant) opening the vanilla `/ui` can now see, at a glance, *what a session
can do*, *where each task's occupant sits*, *that a token cannot widen its own
grant*, and can *drive the whole surface from the keyboard*. No SPA — it stays
one static `index.html`, guarded by the Cluster-133 `ui_js_contract` static
check and the Cluster-350 Playwright harness.

## What shipped

- **353.1 (#633) — the capability card.** A new "Session" tab renders `{can,
  can't}` from `GET /me`'s *real* grant, not a declared set. `WhoAmI` gained
  `known_capabilities` (= `capability::all()`) so the client computes "can't" =
  vocabulary − granted without hardcoding; `/ui/api/me` was added to the
  session-authed read proxy. The point: a markdown `allowed-tools` list, or an
  ambient default token, is *not* a grant — the card is ground truth.
- **353.2 (#634) — session-chrome badges.** Every thread in the sidebar carries
  a running / idle / needs-input / needs-approval / done badge, derived by a pure
  `sessionChrome(thread, gate)` (first match wins): done (terminal) > needs-input
  (a pending gate *with* a schema) > needs-approval (a gate *without*) > running
  (assigned + `work_started_at`) > idle. No backend change — every input already
  shipped in existing reads.
- **353.3 (#635) — attenuation chrome.** The Tokens tab surfaces the caller's
  grant (the ceiling) and pre-flights a mint: `capsExceedingGrant` flags any
  requested capability beyond the caller's own set and blocks the request with a
  visible `role="alert"` warning — the client-side mirror of the server's
  `validate_subset`. You cannot widen your own grant.
- **353.4 (#636) — WCAG-AA keyboard operability (B1+N7).** The tab bar became a
  real ARIA tablist (roles, roving tabindex, Arrow/Home/End navigation), a skip
  link jumps to `#main-content`, and a `:focus-visible` outline lands on every
  interactive control. `lang="en"` was already present.

## Decisions

- **No SPA; derive in the client.** Every badge and card reads data the backend
  already serves (`/me`, the thread's assignee/clock/state, pending gates). The
  only backend touch in the whole cluster was one additive `WhoAmI` field + one
  proxy route (353.1). This keeps the `/ui` a static page and the Handoff
  "no-SPA" rule intact.
- **`needs-input` vs `needs-approval` = the gate's `schema`.** Maidan has no
  thread-level "awaiting input" signal, so the chrome distinguishes a structured
  elicitation (a gate carrying a JSON `schema`) from a yes/no approval (no
  schema). A data-backed split rather than a new state column.
- **Attenuation is shown *and* enforced early.** The server already rejects a
  widening mint (`validate_subset`); the chrome makes the ceiling visible and
  fails fast, so the operator learns the rule before hitting a 403.

## Surprises

- **`GET /me` already carried the real grant**, so the flagship "capability
  card" needed only an additive field — the hardest part (ground-truth
  capability introspection) was already built (Cluster 337).
- **The seeded fixture thread has a pending schemaless gate**, which made 353.2's
  Playwright assertion deterministic for free — the fixture thread renders
  `needs-approval` without any extra seeding.
- **The strict-unwrap clippy scope**, again: the first local pass ran
  `--tests -D clippy::unwrap_used`, which false-flags test unwraps. The CI
  restriction step is `--lib --bins` only; the split matters (memory
  `maidan-strict-unwrap-lint`).

## Test evidence

- `ui_js_contract` grew four static wiring guards (capability card, session
  chrome, attenuation, WCAG tablist) on top of the Cluster-133/153 checks — the
  required jobs have no browser, so these catch a broken wire.
- `ui_session_e2e` (Rust) — `/ui/api/me` returns granted + `known_capabilities`.
- Four Playwright specs (`session`, `session-chrome`, `attenuation`, `a11y`)
  cover the DOM render, the gate-derived badge, the widening block, and keyboard
  navigation in a real browser.
- Every sub-PR: fmt + strict/all-targets clippy + the above, admin-merged green.

## Forward look

**The identity chrome is complete.** Wave 1 #4 (N8 + B1 + N7) is delivered
without an SPA. Next-ranked is **Wave 1 #5 — H4** (lookback on `wait_for_*` +
evict-on-wait; idempotent side effects before a wait; no occupancy I/O in Drop).

## Acknowledgements

Built as a four-PR stack (#633 → #634 → #635 → #636) on the 350/351/352
mechanics, each rebased onto `main` as its parent merged.
