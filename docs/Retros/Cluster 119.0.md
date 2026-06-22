# Cluster 119.0 retro — Dependency dedupe & currency

> Tag **`v119.0.0`**. First cluster of **Phase XXIII (Supply chain & scale gate)**.

## What shipped

- **thiserror 1 → 2** (119.0.1): workspace bump, source-compatible (no code
  changes). Our crates are on the current major.
- **`deny.toml` duplicate-version gate** (119.0.2): `[bans] multiple-versions`
  flipped **warn → deny**, so a *new* duplicate major fails CI. The unavoidable
  duplicates are explicit, reasoned exceptions — `skip-tree` for the vendored
  subtrees (`aws-config`/`aws-sdk-s3` crypto+HTTP stack, `openidconnect` v4,
  `testcontainers` dev) and a small `skip` list for cross-cutting ecosystem
  transitions (`getrandom`/`rand`, `hashbrown`, `windows-sys`, `itertools`,
  `metrics-util`). `cargo deny check` green.
- **`docs/Dependencies.md`** (119.0.3): the duplicate-version policy, currency
  status of the named majors, the **openidconnect v5 tracking item**, and the
  **edition-2024 evaluation**.

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster 120  | Final `maidan-scale-1.0` gate | Closes Phase XXIII and the 102+ ladder. |
| Track V/X    | Edition 2024 adoption | Compiles, but needs `collapsible_if` fixes + full-suite validation of the `if let` temp-scope change. |
| Upstream     | `hmac` / `base64` dedupe, `rsa` advisory | Blocked on the AWS SDK crypto stack and **openidconnect v5** (unreleased). |

## Surprises

- **"Collapse duplicate majors" was mostly not ours to do.** Of the three named
  crates, only **thiserror** is pulled by our own crates; bumping it to 2 still
  leaves thiserror 1.x in the tree (transitive via `metrics-exporter-prometheus`,
  `tungstenite`, `oauth2`). **hmac** 0.12 **and** 0.13 are entirely inside the
  AWS SDK crypto stack, and **base64** 0.21 comes solely from openidconnect v4.
  So the honest deliverable became a *hard gate + documented tracking*, not a
  tree collapse.
- **openidconnect v5 doesn't exist yet** (latest is `4.0.1`). The ladder's word
  "**track**" was literal: base64 0.21 and the `rsa` advisory can't clear until
  v5 ships, so the advisory ignore stays with a "remove when v5 lands" runbook.
- **Edition 2024 compiles but isn't lint-clean.** `cargo build --workspace`
  succeeds, but `clippy -D warnings` flags `collapsible_if` (let-chains) across
  several crates, and the `if let` temporary-scope change can shift drop/lock
  timing — so adoption needs a focused migration with full e2e validation, not a
  one-line edition flip.
- **cargo-deny "unnecessary skip" caught over-skipping.** The first skip list
  duplicated what `skip-tree` already quarantined (thiserror 1, http 0.2, syn 1);
  cargo-deny flagged them, and trimming gave a minimal, honest config.

## Decisions

- **Hard gate (`deny`) over advisory (`warn`).** This is a scale-gate phase; a
  new duplicate should fail CI and force review. Brittleness from version-pinned
  `skip` entries is the intended cost — `skip-tree` absorbs the high-churn
  vendored subtrees so the residual pinned list is the slow-moving ecosystem
  transitions. No [[Decisions]] change.
- **Keep the `rsa` advisory ignore, with a runbook.** No fixed `rsa` release
  exists; the risk (RS256 id_token verify timing) is accepted and documented,
  and the exact steps to remove it on openidconnect v5 are in
  `docs/Dependencies.md`.
- **Defer edition 2024.** Viable but a separate Track-V/X migration.

## Capability table extension

| Capability | Where |
|------------|-------|
| Duplicate-major CI gate (`multiple-versions = deny`) | `deny.toml` (`lint` job) |
| Dependency currency + duplicate policy doc | `docs/Dependencies.md` |
| Workspace on thiserror 2 | `Cargo.toml` |

## Risks identified + mitigated

- **Silent duplicate-major creep.** Now a hard CI failure outside the documented
  exceptions.

## Risks identified + still open

- **`rsa` timing advisory (RUSTSEC-2023-0071)** remains ignored until
  openidconnect v5; tracked with a removal runbook.
- **deny skip-list churn** on dependency bumps — accepted as the gate's cost.

## Forward look

Phase **XXIII** concludes with **Cluster 120 — the final `maidan-scale-1.0`
gate**, closing the 102+ product ladder.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
