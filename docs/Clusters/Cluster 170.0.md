# Cluster 170.0 — CI/CD: native arm64 release build + trivy image scan

**Theme:** Arc 2 (perf), part 5 — the CI/CD workflow speedups. Closes arc 2.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v170.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Build the arm64 `maidan-server` image on a **native** `ubuntu-24.04-arm` runner instead of QEMU emulation | `.github/workflows/release.yml` (`docker-server`) |
| Add a **trivy** vulnerability scan of the released server image (report-only on introduction) | `.github/workflows/release.yml` (new `trivy-scan` job) |

## Why

- **Native arm64.** `docker-server` cross-built `linux/arm64` under QEMU on an
  amd64 runner. The server Dockerfile does a full `cargo build --release`
  (cargo-chef + workspace compile), and emulated arm64 compilation is
  pathological — it turned that one matrix leg into a ~2 h job that dominated the
  release wall-clock (the whole release ran ~2 h 18 m). Public repos get free
  `ubuntu-24.04-arm` runners, so each matrix leg now builds **only its native
  platform** — no emulation. The QEMU setup step is dropped.
  - `docker-postgres` is left on its single multi-platform build: its Dockerfile
    is `FROM pgvector/pgvector:pg16` with **no compilation**, so the emulated
    arm64 leg is trivially fast and not a bottleneck.
- **trivy.** No image vulnerability scanning existed. A new `trivy-scan` job runs
  after the multi-arch manifest and scans `maidan-server` for fixable OS +
  library CVEs (`CRITICAL,HIGH`, `ignore-unfixed`). Introduced **report-only**
  (`exit-code: 0`) so it surfaces findings in the release logs without gating the
  first release; promotable to blocking once the baseline is reviewed.

## Non-goals

- Splitting `docker-postgres` into a per-arch matrix — no compile, no win.
- Cargo caching — already in place (`Swatinem/rust-cache` across the ci.yml jobs).
- Making trivy blocking — deferred until the first scan baseline is reviewed.

## Exit criteria

- Release builds arm64 natively (no QEMU); trivy scans the server image; the
  `v170.0.0` release run completes materially faster — **verified by the release
  run itself** (these are release-only changes; see below).
- `v170.0.0` tagged.

## Verification & limits

- ci.yml (the 8 required checks) is unchanged and stays green — release.yml is
  not exercised by PR CI.
- The arm64-native build + trivy job are **proven by the `v170.0.0` release run**
  (tag-triggered, post-merge). The YAML is validated locally (`yaml.safe_load`)
  and the `trivy-action@v0.28.0` ref is confirmed to exist.
- Limit: `trivy-scan` is report-only; a fixable CRITICAL will show in the logs
  but not fail the release yet.

## References

- [[Retros/Cluster 170.0]]; `.github/workflows/release.yml`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program`, `maidan-release-workflow-slow`.
