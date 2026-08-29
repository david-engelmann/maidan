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

**Container image** (signed by immutable digest):

```sh
cosign verify ghcr.io/david-engelmann/maidan-server:<tag> \
  --certificate-identity-regexp '^https://github.com/david-engelmann/maidan' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

**Release binary + SBOM** (each artifact ships a `.cosign.bundle` = signature + cert +
Rekor proof; download the tarball and its bundle from the release page):

```sh
cosign verify-blob \
  --bundle maidan-x86_64-unknown-linux-gnu.tar.gz.cosign.bundle \
  --certificate-identity-regexp '^https://github.com/david-engelmann/maidan' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  maidan-x86_64-unknown-linux-gnu.tar.gz
```

A `sbom.json` (CycloneDX) is published and signed the same way. A verification failure
means the artifact was not produced by this repo's release pipeline — do not run it.

## Cryptography

Cryptographic bugs (key handling, signature verification, nonce reuse)
are prioritized above functional security bugs. Flag them as high
severity in your report.
