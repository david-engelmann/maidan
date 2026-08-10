# Cluster 183.0 retro — a DoS floor by default, and an honest 413

> Tag **`v183.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc A (security & correctness), part 5.

## What shipped

- A built-in global per-client rate limit (1200 req / 60 s per bearer/IP) that
  applies when `MAIDAN_RATE_LIMIT_MAX` is unset — gated by an `AppState`
  `rate_limit_default_on` flag the *server binary* sets, so it never touches
  tests or library embedders. Explicit env (incl. `0`) always overrides.
- An explicit, env-tunable request body cap (`MAIDAN_MAX_BODY_BYTES`, default
  2 MiB) via `DefaultBodyLimit::max`.
- `ApiError::PayloadTooLarge` so an oversized body renders as `413` +
  `problem+json`, not a flattened `400`.

## Surprises

- **Default-on rate limiting is a test hazard, not just a config change.** The
  in-memory limiter is a process-global `static`, and unauthenticated requests
  all key to `"anonymous"` — so flipping the default on inside the per-request
  env read would have made hundreds of tests share one counter and flake with
  spurious `429`s. The fix was to move the *default* decision onto an `AppState`
  flag that only `main.rs` sets; tests build `AppState::new` → flag off →
  untouched. (The explicit-env path the rate-limit tests exercise is unchanged.)
- **The body cap already existed — invisibly.** axum 0.7 caps its extractors at
  2 MiB by default, so the "add a body cap" task was really "make the existing
  limit explicit + tunable + honest about its status code." Setting the default
  to 2 MiB means zero behaviour change until an operator tunes it.
- **`ApiJson` was swallowing the 413.** It mapped *every* `JsonRejection` to
  `BadRequest`, so an over-limit body returned `400`. Preserving the rejection's
  `PAYLOAD_TOO_LARGE` status needed a new `ApiError` variant — which the
  exhaustive `status`/`title`/`detail`/`problem_type` matches (and one in
  `mcp_quota.rs`) then forced me to handle everywhere.

## Decisions

- **Coarse floor, not fairness.** The default is a per-client abuse floor; the
  per-workspace fairness limit stays independently opt-in (it's a tenancy
  policy, not a safety default).
- **One global cap, artifacts included.** Rather than exempt the artifact routes,
  the 2 MiB default preserves today's behaviour (they were already 2 MiB-capped);
  a deployment expecting large single-shot artifacts raises the env, and
  multipart upload remains the path for big blobs. Simpler than per-route limits.

## Capability table extension

| Change | Where |
|--------|-------|
| Default-on per-client rate limit + explicit/tunable body cap + `413` on oversize | `rate_limit/mod.rs`, `app.rs`, `error.rs` |

## Risks identified + still open

- **Net risk-reducing.** New default protects unconfigured deployments; behaviour
  under an explicit config is unchanged; body cap is behaviour-identical at the
  default. Watch: the CI smoke jobs run the real binary and are now subject to
  the 1200/60s floor — they're functional smokes (well under it), but a future
  high-volume smoke would need `MAIDAN_RATE_LIMIT_MAX=0`.

## Forward look

Arc A closes with **184 — dual-write atomicity**: `publish()` commits the domain
row then appends the event separately (swallowing failures → orphaned rows). That
work also upgrades Cluster 182's best-effort audit toward a transactional one.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
