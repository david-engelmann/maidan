# Cluster 299.0 retro — SDK interop CI

> Tag **`v299.0.0`**. Phase XXIV (post-gate hardening). SDK interop CI. No new gate tag.

## What shipped

A report-only CI job that proves all four client SDKs interop with a running server, closing
the SDK loop (294–298):

- **`sdk-interop` job in `.github/workflows/ci.yml`** — boots a source-built server (SQLite,
  auth disabled) and runs each SDK's black-box suite against it via `scripts/sdk-test.sh`
  (typescript → python → go → rust). Installs the four toolchains (rust via
  `dtolnay/rust-toolchain@stable` + `Swatinem/rust-cache`, node 20, python 3.12, go 1.22),
  warms the server build once, then runs the four suites sequentially (each boots + tears down
  its own server on the harness's default port).
- **Report-only** (`continue-on-error: true`) and **not a required check** — the server
  behavior the SDKs exercise is already gated by the required Rust e2e tests; this job proves
  the *clients* interop end-to-end without ever blocking a merge. Same posture as the Cluster-289
  `a2a interop` job.

## Surprises / decisions

- **One job, four suites, sequential** — not four parallel jobs. Each `scripts/sdk-test.sh`
  invocation boots + tears down its own server (trap on EXIT), so they can't collide on the
  port, and the shared cargo cache means only the first build is real. Cheaper than four jobs
  each doing a cold server build.
- **Report-only, deliberately.** An SDK suite depends on network/toolchain setup and a live
  server boot — exactly the kind of thing that flakes. Making it required would let infra noise
  block merges; the required Rust e2e tests already own the server contract. This job is the
  *evidence* the clients work, not a gate.
- **No new required check** — adding a job to `ci.yml` doesn't change branch protection; the 8
  required checks are unchanged.

## Capability table extension

New `sdk-interop` report-only CI job. No server capability change.

## Risks identified + still open

- Report-only means a genuine SDK regression won't block a merge — it's visible in the run, not
  enforced. Promoting it to required would need the flake surface (toolchain installs, server
  boot timing) hardened first; logged as a possible follow-up.

## Forward look

The SDK loop is closed (four clients 294–297, publish workflow 298, interop CI 299). Remaining
of the five-arc program: **MCP `2026-07-28`** (protocol upgrade), **durable mail retry queue**,
**Slack / Git projectors**, and **public launch**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 298.0]] and the
SDK arc ([[maidan-sdk-arc]]).
