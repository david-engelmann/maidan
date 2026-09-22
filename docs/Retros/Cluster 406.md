# Cluster 406 retro — published boot proof and real loopback OIDC

> Wave 4 row #37 · `v406.0.0` · umbrella #956 · PRs #957/#961/#959 + close record

## Outcome

Maidan now publishes the operator CLI as a separate non-root, multi-architecture
image assembled from the same architecture-matched archive as the release binaries.
The release does not become a GitHub Release until a clean consumer environment
pulls all three tagged images and proves CLI init, Postgres bootstrap, exact-version
server health, bearer authentication, and anonymous rejection. The human-login path
also has an end-to-end test against a real loopback OIDC provider rather than only the
deterministic mock.

| Slice | Evidence | Result |
|-------|----------|--------|
| 406.1 | #957; required CI `bootstrap compile-time strip`; CLI unit/integration suite; `cargo clippy --all-targets --all-features -- -D warnings` | Exact release archive becomes a dedicated distroless/non-root CLI image for amd64 + arm64; injected release version is visible at runtime; server and CLI digests share signing and scanning policy. |
| 406.2 | #961; `bash -n scripts/release-image-smoke.sh`; release job `published server + CLI boot smoke` | Fresh network and database; pull exact tags; CLI init; exact `/health` version; bearer `/me` succeeds; anonymous `/me` returns 401; GitHub Release waits for the proof. |
| 406.3 | #959; `cargo test -p maidan-server --test oidc_loopback_e2e`; existing `oidc_e2e` tests | Production `OidcRuntime` traverses discovery, code + S256 PKCE, token, ES256 JWKS, provisioning, signed session, and provider logout. Unknown state and bad nonce/signature/audience/issuer all fail without a cookie. |

## Decisions

- The CLI remains a separate image. Combining it with the distroless server would
  enlarge the server runtime and blur two entrypoints with different jobs.
- Release images consume the exact per-architecture binary archive. Rebuilding a
  binary in Docker would make the tested/downloadable artifact and image artifact
  different products.
- The registry smoke is downstream of manifest publication and upstream of GitHub
  Release creation. A source checkout or locally built image cannot satisfy it.
- The OIDC test uses an in-process loopback IdP with real ES256 keys and production
  discovery/token validation. It proves the protocol boundary without making a
  browser or an external identity provider part of required CI.

## What surprised us

- The previously published CLI reported `0.0.0`: Clap read the workspace package
  version instead of the injected `MAIDAN_VERSION`. Building an image successfully
  would not have detected that consumer-visible defect.
- An architecture-mismatched local image can make a correct binary look broken.
  The release workflow therefore binds each image job to its matching archive and
  CI asserts executable output rather than only inspecting an image config.
- GitHub Release creation had no dependency on published-image usability. Signing
  and successful builds proved provenance, not that a stranger could initialize and
  authenticate a real deployment.

## Residual risk and follow-up

- The first complete registry proof for this new gate runs when `v406.0.0` is
  published; PR CI validates the script and CLI image, while the tagged workflow is
  the only environment allowed to prove immutable GHCR artifacts.
- Trivy remains report-only. Keyless cosign signatures cover both server and CLI
  digests; promotion policy can tighten vulnerability enforcement independently.
- `latest` remains mutable for convenience. Documentation and the smoke use the
  exact release tag, and production guidance continues to require a pinned tag.
- The loopback provider covers the standards and validation boundary, not each
  vendor's configuration UI. Provider-specific recipes remain documentation work.

## Release ledger

| Item | Value |
|------|-------|
| Tag | `v406.0.0` |
| Roadmap | Wave 4 row #37 closed |
| Database/API compatibility | No schema migration or production API shape change |
| Runtime additions | Separate CLI image; release-only consumer smoke |
| Security boundary | OIDC rejection cases proved; no validation weakened |
