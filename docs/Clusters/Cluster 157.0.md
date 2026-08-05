# Cluster 157.0 — fail-closed `AUTH_DISABLED`

**Theme:** Enterprise-hardening arc (arc 1), part 2. Close the silent-open-door
risk: `AUTH_DISABLED` could disable authentication entirely on any
non-production deployment.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v157.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Pure fail-closed validator + helpers | `crates/maidan-server/src/config.rs` (`validate_insecure_no_auth`, `auth_disabled_requested`, `insecure_no_auth_acknowledged`) |
| Defense-in-depth on the bypass path | `crates/maidan-server/src/auth.rs` (`auth_disabled_from_env`) |
| Ack flag in the no-auth CI manifests | `compose.yaml` (5 services), `helm/maidan/values-ci.yaml` |
| Docs | `docs/Production.md`, `docs/Threat-Model.md` (T2) |

## Why

`AUTH_DISABLED` was rejected **only** when `MAIDAN_ENV=production`. Every other
deployment (staging, or `MAIDAN_ENV` simply unset) that set `AUTH_DISABLED=1`
served **every request** through `AuthContext::bypass()` — no bearer required.
A flag copied from a dev compose file, or an unset env in a hardened deployment,
silently opened the whole workspace. The prod-readiness review ranked this a
top blast-radius item.

Now it is fail-closed: `AUTH_DISABLED` is honored only with the explicit
`MAIDAN_ALLOW_INSECURE_NO_AUTH=1` acknowledgement, and never in production. A
stray `AUTH_DISABLED=1` refuses boot with a clear error rather than serving open.

## Non-goals

- The bootstrap routes (`MAIDAN_BOOTSTRAP`) — separately guarded (compile-time
  strip, Cluster 91) and unchanged here.
- Any change to the capability model — that's the RBAC cluster next.

## PR ladder (actual)

| # | Title |
|---|--------|
| 157.0.1 | `feat(auth): fail-closed AUTH_DISABLED — explicit ack required` (#404) |
| 157.0.retro | `docs(retro): Cluster 157.0 + v157.0.0 tag prep` |

## Exit criteria

- `AUTH_DISABLED` alone refuses boot; ack (non-prod) allows it; prod always
  refuses; required smoke jobs green with the ack added — **met**.
- `v157.0.0` tagged after retro.

## Verification & limits

- Unit: `auth_disabled_is_fail_closed` covers all four `(requested, production,
  acknowledged)` combos on the pure validator.
- The docker-compose / scale-out / otlp / helm smoke jobs (which boot the real
  binary with the ack) passed — the coordinated manifest change is validated
  end-to-end.
- Limit: a dev binary with both flags explicitly set is still open **by design**
  (seed/test); the guard prevents *accidental* exposure, not intentional dev use.

## References

- [[Retros/Cluster 157.0]]; `config.rs`, `auth.rs`, [[Threat-Model]] T2,
  [[Production]] env table. Program: [[Roadmap]] + memory `maidan-next-arc-program`.
