# Cluster 133.0 — Repair the /ui write path + guard undefined-helper bugs

**Theme:** Before building UI features, fix the `/ui` console's broken write path
(undefined JS helpers) and add a CI guard so this class of bug — invisible to
`cargo test` because there's no browser in CI — can't recur silently.

**Ladder:** Post-gate — **Phase XXIV** (hardening), tag **`v133.0.0`**, no new
gate tag. Re-scoped from "reactions UI" the moment the broken foundation was
found; it unblocks the UI feature clusters (134+).

---

## The bug

The `/ui` (a single static `index.html`, vanilla HTML/JS, **untested** — no
headless browser in CI) had a refactor casualty: the write handlers referenced
helpers that were never defined.

- `apiWritePath(...)` and `requireAuthForWrite()` — called 3× each, **undefined**.
- `uiApiPath(...)` (a read) and `uiWritePath(...)` (a write) — **undefined** typo'd
  calls (the defined helpers are `uiReadPath` / `apiWritePath`).

Effect: "create channel", "create thread", "post message", and "attach artifact"
all threw `ReferenceError`. `cargo test`/clippy never saw it — they only check
the Rust.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Fix** | Define `apiWritePath` (bearer when a token is set, else the session `/ui/api` proxy) + `requireAuthForWrite` (token **or** session); repoint `uiApiPath`→`uiReadPath`, `uiWritePath`→`apiWritePath`. |
| **Guard** | `tests/ui_js_contract.rs` — dependency-free static check that every bare `ident(` call in the inline script resolves to a definition, a parameter, or a known JS/DOM global. Runs in the `unit tests` CI job. |

## Non-goals

- A full JS linter / headless-browser harness (eslint + node toolchain) — heavy
  for one file; the focused bare-call guard catches the actual bug class.
- The reactions feature (deferred to 134, now on a verified base).

## PR ladder (actual)

| # | Title |
|---|--------|
| 133.0.1 | `fix(ui): repair the broken /ui write path + guard undefined-helper bugs` (#356) |
| 133.0.retro | `docs(retro): Cluster 133.0 + v133.0.0 tag prep` |

## Exit criteria

- All four broken references fixed; the `/ui` write path works (session or bearer).
- The guard flags undefined bare-call helpers in CI — **met** (it caught all four
  before the fix; green after).
- `v133.0.0` tagged after retro.

## Ordering & risks

- **Guard is conservative.** It over-collects params/globals (only *weakens* the
  check), so it never false-fails — it can in principle miss an exotic case, but
  it reliably catches "helper called but never defined", which is the bug here.
- **JS still isn't browser-tested.** This guard catches reference bugs, not
  behavior; UI behavior remains inspection-verified (documented limitation).

## References

- [[Retros/Cluster 133.0]]; `crates/maidan-server/static/index.html`, `tests/ui_js_contract.rs`
