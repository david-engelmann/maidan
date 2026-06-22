# Cluster 119.0 — Dependency dedupe & currency

**Theme:** Bring our own deps to current majors, make duplicate majors a hard CI gate, and document what's blocked upstream.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXIII · tag **`v119.0.0`** (opens the phase).

**Predecessor:** `deny.toml` (`multiple-versions = "warn"`); the rsa advisory ignore from earlier OIDC work.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Currency** | thiserror 1 → 2 (our crates on the current major). |
| **Gate** | `deny.toml` `multiple-versions` warn → **deny**, with `skip-tree`/`skip` quarantines for the upstream-blocked dups. |
| **Tracking** | `docs/Dependencies.md`: duplicate policy, openidconnect-v5 / rsa runbook, edition-2024 evaluation. |

## Non-goals

- Collapsing `hmac`/`base64` (vendored: AWS SDK / openidconnect v4 — not ours).
- Clearing the `rsa` advisory now (needs openidconnect v5, unreleased).
- Adopting edition 2024 (separate Track-V/X migration).

## PR ladder (actual)

| # | Title |
|---|--------|
| 119.0.1 | `chore(deps): move workspace to thiserror 2` (#325) |
| 119.0.2 | `chore(deny): make duplicate majors a hard error with quarantined exceptions` (#325) |
| 119.0.3 | `docs(deps): dependency currency + duplicate-version policy; edition 2024 eval` (#325) |
| 119.0.retro | `docs(retro): Cluster 119.0 + v119.0.0 tag prep` |

## Exit criteria

- Collapse duplicate majors where in our control (thiserror) — **met**; hmac/base64 documented as upstream-blocked.
- Tighten `deny.toml` multiple-versions for crypto crates — **met** (warn→deny + reasoned quarantines).
- Track openidconnect v5 to clear the rsa advisory — **met** (documented; v5 unreleased, ignore retained with runbook).
- Evaluate edition 2024 — **met** (compiles; deferred with reasons).
- `v119.0.0` tagged after retro.

## Ordering & risks

- **thiserror first** (currency), then the gate (so the gate reflects the post-bump graph), then docs.
- **Risk — deny brittleness:** version-pinned `skip` entries churn on bumps; `skip-tree` absorbs the vendored subtrees to minimize it.

## References

- [[Clusters/Product Ladder 102+]] Phase XXIII
- [[Retros/Cluster 119.0]], [Dependencies.md](../Dependencies.md)
