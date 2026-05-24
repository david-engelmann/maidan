# Track U — Performance engineering

Cross-cutting track (no version tag). Regression detection and query
tuning after `v1.0.0`.

> **Goal:** Benchmarks in CI (non-blocking or nightly), mutation tests on
> critical paths, and documented query-tuning workflow.
>
> **Not a cluster** — ship as numbered PRs on `main`.

## PRs (proposed)

| #    | Title | Source |
|------|-------|--------|
| U.1  | `bench(maidan-store): criterion hot paths` | Cluster A retro |
| U.2  | `chore(ci): nightly cargo-mutants on maidan-bus + routes` | Cluster B retro |
| U.3  | `docs: EXPLAIN playbook for Postgres search + events` | Open Work |
| U.4  | `chore: 1000-event WS soak in integration` | Cluster B retro |

## Exit criteria

- At least one criterion bench committed with baseline JSON.
- Mutation job documented (nightly, not required on PR).
- Soak test skips gracefully without Docker.

## Out of scope

- HPA manifests (k8s ops, see [[Deploy]]).
- Production load testing as a service.
