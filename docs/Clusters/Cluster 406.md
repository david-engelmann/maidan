# Cluster 406 — Wave 4 #37: published boot proof and real loopback OIDC

> Post-gate hardening · target tag `v406.0.0` · umbrella issue #956

## Contract

- Publish a version-aligned, non-root, multi-architecture `maidan-cli` image
  separately from the distroless `maidan-server` image. Sign both immutable
  image digests through the release workflow.
- Prove the images consumers actually pull from GHCR can initialize a fresh
  Postgres database, boot, become healthy, and accept the one-time admin
  credential. A source-built stand-in is not release evidence.
- Exercise the real OIDC discovery, authorization-code, token, JWKS, and
  session path against test-only loopback infrastructure. Preserve state,
  nonce, PKCE, issuer, audience, and signature verification.
- Keep the developer quickstart and deterministic mock-OIDC coverage. These
  additions prove the published and standards-based paths rather than
  replacing fast local tests.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 406.1 | current | Published CLI runtime image, release provenance, and operator docs |
| 406.2 | current | Post-publication GHCR server + CLI boot/init/auth smoke |
| 406.3 | current | Real loopback OIDC discovery/code/token/JWKS/session e2e |
| 406.close | planned | Ledgers, executable evidence, and retrospective |

## Non-goals

- Bundling the operator CLI into the server image.
- Publishing or operating an identity provider.
- Weakening OIDC validation or making `MAIDAN_OIDC_MOCK` production-legal.
- Treating a local image build or the mutable `latest` tag as release proof.
