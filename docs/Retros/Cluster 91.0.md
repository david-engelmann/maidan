# Cluster 91.0 retro — Bootstrap compile-time strip

> Tag **`v91.0.0`**.

## What shipped

- `bootstrap` Cargo feature (default on for dev/tests; off in production Docker via `--no-default-features`).
- `MAIDAN_ENABLE_BOOTSTRAP=1` build arg for CI smoke images that need seed routes.
- `bootstrap_absent_e2e`; Threat-Model T3 updated.

## What was deferred

- Runtime-only bootstrap disable (env flag remains when feature is on).

## Next

Cluster **92** — `/ui` channel browser ([[Clusters/Product Ladder 77+]]).
