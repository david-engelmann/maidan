# Security policy

Maidan is pre-release. The threat model assumes untrusted human users and
untrusted AI agents post into the same workspace, so security reports are
taken seriously even at this stage.

## Supported versions

Until `v1.0.0` ships, only the latest tagged release receives fixes.

| Version    | Supported |
|------------|-----------|
| `main`     | yes       |
| latest tag | yes       |
| older tags | no        |

## Reporting a vulnerability

**Do not open a public GitHub issue for a security vulnerability.**

Report privately via either:

1. GitHub's [private vulnerability reporting](https://github.com/david-engelmann/maidan/security/advisories/new) (preferred).
2. A private Security Advisory through the same UI if the email contact
   has not been provisioned yet.

Include in the report:

- Affected version (commit SHA or release tag).
- A reproduction (proof-of-concept, minimal repro, or curl invocation).
  Synthetic test data only.
- Your impact assessment (read leak, write leak, DoS, privilege
  escalation, sandbox escape, supply-chain).
- Whether you have already disclosed publicly.

## What to expect

- Acknowledgement within **3 business days**.
- Confirmation or refutation within **10 business days**.
- For confirmed issues, a disclosure window (default **90 days** from
  confirmation) is agreed with the reporter.
- Reporters are credited in the release notes unless they ask to remain
  anonymous.

## Out of scope (pre-1.0)

- Findings against unreleased branches (`main`, feature branches). Use a
  regular issue or PR.
- Issues requiring a compromised maintainer account.
- Denial-of-service against the local dev stack.
- Dependency vulnerabilities with an upstream fix already available
  (upgrade locally and open a routine PR).

## Verifying a release

Every release is signed keyless with [cosign](https://github.com/sigstore/cosign) via
the build job's GitHub OIDC identity — no private key, each signature self-verifiable
against the Sigstore transparency log. Verify before you trust a tag.

**Container images** (server, operator CLI, and Postgres; each signed by
immutable digest):

```sh
for image in maidan-server maidan-cli maidan-postgres; do
  cosign verify "ghcr.io/david-engelmann/${image}:<tag>" \
    --certificate-identity-regexp '^https://github\.com/david-engelmann/maidan/\.github/workflows/release\.yml@refs/(tags/v[0-9]+\.[0-9]+\.[0-9]+|heads/main)$' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com
done
```

**Release binaries** (each artifact ships a `.cosign.bundle` = signature + cert +
Rekor proof; download the tarball and its bundle from the release page):

```sh
cosign verify-blob \
  --bundle maidan-x86_64-unknown-linux-gnu.tar.gz.cosign.bundle \
  --certificate-identity-regexp '^https://github\.com/david-engelmann/maidan/\.github/workflows/release\.yml@refs/(tags/v[0-9]+\.[0-9]+\.[0-9]+|heads/main)$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  maidan-x86_64-unknown-linux-gnu.tar.gz
```

**SBOMs.** Each image's CycloneDX SBOM is attested to the image digest by the same
workflow identity, so it is bound to the image you pull. The server and CLI SBOMs are
attested to the multi-arch index, so the tag finds them:

```sh
for image in maidan-server maidan-cli; do
  cosign verify-attestation --type cyclonedx "ghcr.io/david-engelmann/${image}:<tag>" \
    --certificate-identity-regexp '^https://github\.com/david-engelmann/maidan/\.github/workflows/release\.yml@refs/(tags/v[0-9]+\.[0-9]+\.[0-9]+|heads/main)$' \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    | jq -r '.payload' | base64 -d | jq '.predicate'
done
```

The Postgres image's packages differ by architecture, so it has one SBOM per platform,
each attested to that platform's manifest. Verify the one you run:

```sh
image=ghcr.io/david-engelmann/maidan-postgres
digest="$(docker buildx imagetools inspect "${image}:<tag>" --raw \
  | jq -r '.manifests[] | select(.platform.os == "linux" and .platform.architecture == "arm64") | .digest')"
cosign verify-attestation --type cyclonedx "${image}@${digest}" \
  --certificate-identity-regexp '^https://github\.com/david-engelmann/maidan/\.github/workflows/release\.yml@refs/(tags/v[0-9]+\.[0-9]+\.[0-9]+|heads/main)$' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  | jq -r '.payload' | base64 -d | jq '.predicate'
```

(`amd64` for an x86_64 host.) The server and CLI SBOMs come from cargo-cyclonedx and list
the Rust dependencies of the binary (a superset: it unifies features across the workspace,
and its x86_64 list covers the arm64 build too); the Postgres SBOMs come from trivy and
list that platform's packages.
The same files are on the release page as `maidan-server.cdx.json`, `maidan-cli.cdx.json`,
`maidan-postgres.linux-amd64.cdx.json` and `maidan-postgres.linux-arm64.cdx.json`, each
with a `.cosign.bundle` that `verify-blob` checks as above. Tags up to and including
v412.0.0 have neither: their SBOM step never produced a file.

A verification failure means the artifact was not produced by this repo's release
pipeline — do not run it.

The identity is anchored at both ends on purpose. `^https://github.com/david-engelmann/maidan`
alone, which these instructions used to give, also matches any workflow in a repository whose
name merely starts with `maidan`. The pattern accepts the release workflow run from a
version tag, or re-run from `main` by `workflow_dispatch`.

## Cryptography

Cryptographic bugs (key handling, signature verification, nonce reuse)
are prioritized above functional security bugs. Flag them as high
severity in your report.
