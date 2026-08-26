# Cluster 276.0 retro — the binary tells the truth about its version

> Tag **`v276.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P0:
> runtime version truthfulness.** No new gate tag.

## What shipped

The released binary and image reported `0.0.0` from `/health` (the external review
caught it by running the `v272` binary). The `version()` override already existed
(`option_env!("MAIDAN_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))`); the release
pipeline simply never set `MAIDAN_VERSION`. Now it does, on every build path:

- **`build.rs`** (new, `maidan-server`) declares `cargo:rerun-if-env-changed=MAIDAN_VERSION`.
  `option_env!` is not part of cargo's fingerprint, so without this a warm build cache
  would bake a stale version into a new release whose source is unchanged (our releases
  are frequently docs-only). This forces a recompile when the tag changes.
- **Native binary builds** (`release.yml` `build` job) resolve the tag and set
  `MAIDAN_VERSION` on the `cargo build` step (x86_64-linux, macos-arm64).
- **Cross build** (aarch64-linux) gets `MAIDAN_VERSION` forwarded into the `cross`
  container via a new **`Cross.toml`** `[build.env] passthrough`.
- **Docker image** takes `ARG MAIDAN_VERSION` + `ENV` in the builder stage right before
  `cargo build` (so a changed tag invalidates the layer and recompiles), fed by
  `build-args` from the `docker-server` job.

Cargo `version = "0.0.0"` intentionally stays — the workspace is `publish = false`
(no crates.io), so the release identity is the tag, carried at runtime by `MAIDAN_VERSION`.

## Surprises / decisions

- **The fix is 90% pipeline, not code.** The one code file is a five-line `build.rs`
  whose only job is cache-correctness. The real bug was that four build paths each
  needed the tag in the compiler's environment, and each has a different way in
  (step env, `cross` passthrough, Docker `ARG`/`ENV`).
- **Cache-correctness was the subtle part.** The naive fix (set the env in the release
  build) silently fails on a docs-only release: `option_env!` isn't fingerprinted, so
  cargo reuses the cached object with the old version, and Docker reuses the cached
  build layer. The `build.rs` `rerun-if-env-changed` and the ENV-before-build layer
  placement close both holes.
- **Proven locally end-to-end:** `MAIDAN_VERSION=v276-verify cargo build -p maidan-server`
  then `strings … | grep v276-verify` finds it (1 occurrence); an unset build falls
  back to the crate version. The release paths are proven by the `v276.0.0` release run
  itself (like the Cluster-170 native-arm change).

## Capability table extension

| Change | Where |
|--------|-------|
| `build.rs` cache-correctness (`rerun-if-env-changed=MAIDAN_VERSION`) | `crates/maidan-server/build.rs` |
| Tag baked into native + cross binaries | `.github/workflows/release.yml`, `Cross.toml` |
| Tag baked into the server image | `crates/maidan-server/Dockerfile`, `release.yml` docker-server build-args |

## Risks identified + still open

- **No automated assertion yet** that binary/health/image-label/tag agree — the fix is
  self-proven by the release run. A release-time step (run the built image, curl
  `/health`, assert `version == tag`) is the follow-up (logged in Open Work).
- Release-pipeline changes are not exercised by PR CI (only the `build.rs` is); like
  Cluster 170, the proof is the release run. Verify the `v276.0.0` binary/image report
  `v276.0.0` after the tag is cut.

## Forward look

Next launch-readiness P0 is the **SQLite first-write `database is locked`** finding.
WAL + a 5 s `busy_timeout` + per-connection pragmas are already in place (107/166), so
the review's lock is not the obvious missing-pragma case — that cluster starts with
reproducing it before choosing a fix.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 275.0]]. Finding from the external launch-readiness review (Cluster 274).
