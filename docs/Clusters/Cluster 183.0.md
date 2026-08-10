# Cluster 183.0 — security: default-on rate limit + explicit request body cap

**Theme:** Arc A (security & correctness), part 5 — give a deployment that
configures nothing a DoS floor, and make the request body-size cap explicit +
tunable.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v183.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Built-in global per-client rate limit when `MAIDAN_RATE_LIMIT_MAX` is unset | `rate_limit/mod.rs` (`resolve_global`), `state.rs` (`rate_limit_default_on`), `main.rs` |
| Explicit env-tunable request body-size cap (`MAIDAN_MAX_BODY_BYTES`, default 2 MiB) | `app.rs` (`DefaultBodyLimit::max`) |
| Oversized JSON body → `413 Payload Too Large` (not `400`) | `error.rs` (`ApiError::PayloadTooLarge`, `ApiJson`) |

## Why

- **Rate limits were fully opt-in.** With `MAIDAN_RATE_LIMIT_MAX` unset there was
  *no* per-client limit — a fresh deployment had no protection against a runaway
  or abusive client. The research sweep flagged this default-off posture.
- **The body cap was an implicit framework default.** axum 0.7 caps its
  `Bytes`/`Json` extractors at 2 MiB, but that limit was invisible and
  un-tunable, and an oversized body surfaced as a confusing `400`.

## The fix

- `resolve_global(default_on)`: an explicit `MAIDAN_RATE_LIMIT_MAX` always wins
  (including `0`/invalid → disabled); otherwise the built-in default
  (**1200 req / 60 s** per bearer/IP, ~20 req/s) applies when `default_on`. The
  flag lives on `AppState` and is **`false` in `AppState::new`** (so tests and
  library embedders are unaffected) and **`true`** only when the server binary
  boots. The per-workspace fairness limit stays independently opt-in.
- `app.rs` adds `DefaultBodyLimit::max(MAIDAN_MAX_BODY_BYTES)` (default 2 MiB —
  behaviour-identical to axum's implicit default, now explicit + tunable).
- `ApiJson` maps a body-size-limit `JsonRejection` (status `413`) to a new
  `ApiError::PayloadTooLarge` so oversized requests get the correct status +
  `problem+json`, instead of being flattened to `400`.

## Exit criteria

- A server booted with no rate-limit env still enforces a per-client floor; an
  explicit `MAIDAN_RATE_LIMIT_MAX` (incl. `0`) overrides; oversized bodies →
  `413` — **met**.
- `v183.0.0` tagged.

## Verification & limits

- `rate_limit::tests::default_on_applies_a_floor_and_explicit_env_overrides`
  (unit): default-on floor + explicit-override + explicit-`0`-disables.
- `app::body_limit_tests::max_body_bytes_parses_and_falls_back` (unit).
- `body_limit_e2e::oversized_request_body_is_rejected`: a >cap body → `413`, a
  small body → `201`. Existing `rate_limit_e2e` (explicit env path) unchanged.
- Limits: the default floor is per-client (bearer/IP) via the shared in-memory
  window (or Redis in multi-replica); it is a coarse abuse floor, not a fairness
  guarantee (that's the opt-in per-workspace limit). Artifact routes buffer via
  the same cap — a deployment expecting >2 MiB single-shot artifacts must raise
  `MAIDAN_MAX_BODY_BYTES` (multipart upload is the path for large blobs).

## References

- [[Retros/Cluster 183.0]]; `rate_limit/mod.rs`, `app.rs`, `error.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Arc A).
