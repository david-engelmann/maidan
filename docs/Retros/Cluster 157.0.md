# Cluster 157.0 retro — fail-closed `AUTH_DISABLED`

> Tag **`v157.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Enterprise-hardening arc (arc 1), part 2 — the security fix.

## What shipped

- **`AUTH_DISABLED` is fail-closed.** It now takes effect only when
  `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` is also set, and never in production. A pure
  `validate_insecure_no_auth(requested, production, acknowledged)` enforces it in
  `Config::from_env` (refuses boot loudly), and `auth_disabled_from_env()` — the
  function that actually flips `AuthContext::bypass()` — re-checks the same
  invariant as defense-in-depth.
- **Coordinated manifests:** the ack flag was added beside `AUTH_DISABLED` in all
  five `compose.yaml` services and `helm/maidan/values-ci.yaml`, so the required
  smoke jobs (which run the real binary without auth) stay green.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| n/a | Intentional dev-open mode | Kept working (both flags set) — the guard targets *accidental* exposure, not seed/test. |
| Cluster 159+ | Per-channel/thread authz | The capability *model* is untouched here; RBAC is the flagship next. |

## Surprises

- **The blast radius was every non-prod env, not just staging.** The original
  guard keyed only on `MAIDAN_ENV=production`; since `MAIDAN_ENV` is unset in the
  default/dev path (and in all the CI smoke manifests), "not production" was the
  common case — so `AUTH_DISABLED=1` alone opened the server in exactly the
  environments people copy configs from. The ack flag inverts the default to
  "auth on unless you *say* otherwise."

## Decisions

- **Explicit ack flag over MAIDAN_ENV inference.** A positive,
  deliberately-alarming opt-in (`MAIDAN_ALLOW_INSECURE_NO_AUTH`) can't be
  triggered by a missing or typo'd env value; inferring "dev" from `MAIDAN_ENV`
  would re-introduce the same "unset = open" footgun.
- **Two-layer enforcement.** Startup validation gives operators a loud, early
  failure; the check inside `auth_disabled_from_env` guarantees the bypass can
  never engage even if a future caller constructs config differently.

## Capability table extension

| Capability | Where |
|------------|-------|
| Fail-closed `AUTH_DISABLED` (explicit ack, never prod) | `config.rs`, `auth.rs` |

## Risks identified + still open

- **Low, and net-negative risk** (removes a footgun). The only behavior change is
  that `AUTH_DISABLED=1` without the ack now refuses boot — intended. All
  no-auth CI/dev manifests were updated in lockstep and the smoke jobs confirm it.

## Forward look

Arc 1 concludes with **158** (cosign-sign + trivy-scan the container images —
release blobs are signed but images aren't) and then the **flagship channel/
thread RBAC** (the #1 finding: authz is workspace-flat). Then arcs 2 (perf +
CI/CD), 3 (agentic features), 4 (token round 3).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
