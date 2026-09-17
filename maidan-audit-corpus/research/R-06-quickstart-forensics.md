# R-06 — Quickstart forensics

## The three variants

The README presents three getting-started paths. Each was read end-to-end and its auth/bootstrap story extracted:

1. **One-liner** — downloads and runs a release artifact. Cannot mint tokens; the operator must obtain a token through a separate, under-documented step.
2. **Docker** — compose-based. Labeled "one command" but requires three steps with manual token copy/paste between them. Pins image `:v339.0.0` (F-03); the quickstart overlay pins binary v312.0.0 (F-02).
3. **No-Docker (cargo)** — never states the Rust toolchain prerequisite; the insecure variant's required flag pair is inside a collapsed `<details>` block most readers won't open.

## The auth-story divergence

Each variant bootstraps auth differently, and `MAIDAN_BOOTSTRAP=1` (a bootstrap-mode env flag) is referenced without a single clear explanation of what it does, when it's needed, and when it must be turned off. A new operator following two variants gets two different mental models of how Maidan auth works.

## The auth-on transition

Quickstart became auth-on in **Cluster 313**. This date matters because:

- `examples/a2a_interop.py` still assumes auth-disabled quickstart (F-36) — its premise expired ~87 clusters ago.
- Any doc or example written before Cluster 313 that touches auth setup is suspect until re-verified. The audit did not exhaustively check every pre-313 doc for this.

## Why none of this was caught

No CI job runs the README's copied commands. The commands reference flags, files, and env vars that drift independently (same mechanism as R-03's pins and R-04's claim shape). The recommendation in INIT-05 — extract-and-run or at least extract-and-lint README commands in CI — addresses the mechanism, not just the current instances.

**Third-pass note (2026-09-16):** PR #905 is the case study for *why* read-time checks are insufficient. The v312.0.0 pin "looked fine in the file" — it took booting the stack and reading `/health` to discover the binary was 88 clusters behind the source, and `docker compose config` could not have shown it either (the compose default silently wins over the Dockerfile ARG). Any CI verification of quickstart pins must boot the stack and assert on `/health`, not diff the files. See INIT-05's "#905 retro" section.
