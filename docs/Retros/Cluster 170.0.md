# Cluster 170.0 retro — native arm64 release build + trivy image scan

> Tag **`v170.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 2 (perf), part 5 — the CI/CD workflow speedups. **Closes arc 2.**

## What shipped

- **Native arm64 release build.** `release.yml`'s `docker-server` matrix now runs
  each arch on its own native runner (`ubuntu-latest` for amd64,
  `ubuntu-24.04-arm` for arm64) and the QEMU setup step is gone. The arm64
  `maidan-server` image was cross-built under emulation, which made the Rust
  release compile a ~2 h job and dominated the ~2 h 18 m release.
- **trivy image scan.** New `trivy-scan` job runs after the multi-arch manifest,
  scanning `maidan-server` for fixable OS/library `CRITICAL,HIGH` CVEs.
  Report-only (`exit-code: 0`) on introduction.

## What was deferred / not covered

| Item | Why |
|------|-----|
| `docker-postgres` native split | `FROM pgvector/pgvector:pg16`, no compile — emulated arm64 is already fast. |
| Cargo caching | Already present (`Swatinem/rust-cache`). |
| trivy blocking (`exit-code: 1`) | After the first baseline scan is reviewed. |
| build-once smoke image reuse in ci.yml | Marginal (the postgres image is trivial; the server image build is cached); not worth the artifact plumbing. |

## Surprises

- **Only one image was the problem.** The instinct is "make every image build
  native," but the postgres image has no compile step — its emulated arm64 build
  is trivial. The entire ~2 h was the *server* Rust compile under QEMU. Scoping to
  just `docker-server` captures the whole win with the least surface area.

## Decisions

- **Drop QEMU entirely from `docker-server`, don't make it conditional.** Each
  matrix leg builds only its native platform, so binfmt registration is dead
  weight. Removing it is clearer than an `if:` guard.
- **trivy report-only first.** These changes only run on a release tag; a
  blocking scanner introduced blind could red the very release that ships it.
  Report-only surfaces the baseline; promotion to blocking is a one-line follow-up.

## Capability table extension

| Change | Where |
|--------|-------|
| Native `ubuntu-24.04-arm` release build (no QEMU) + trivy server-image scan | `.github/workflows/release.yml` |

## Risks identified + still open

- **Release-only validation.** ci.yml (the 8 required checks) does not exercise
  release.yml, so the arm64-native + trivy path is proven by the `v170.0.0`
  release run itself. The YAML was validated locally and `trivy-action@v0.28.0`
  confirmed to exist; if the arm64 runner or trivy ref misbehaves it's a
  fix-forward (trivy is non-blocking; the GitHub release job depends only on
  `bundle`, so image issues never block the release itself).

## Forward look

**Arc 2 (perf + CI/CD) is complete.** Next: **arc 3 — agentic features**
(structured message content, MCP structured backpressure, HITL approvals over the
elicitation transport, task assignment/handoff), then **arc 4 — token round 3**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
