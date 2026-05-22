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

## Cryptography

Cryptographic bugs (key handling, signature verification, nonce reuse)
are prioritized above functional security bugs. Flag them as high
severity in your report.
