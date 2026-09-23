# Cluster 408 retro — full-audit remediation

> Post-gate hardening · `v408.0.0` · umbrella #971 · PRs #964/#973/#974/#975/#976 + close record

## Outcome

The adopted 2026-09-21 audit work is closed without widening any authority
boundary. Personal member state is self-scoped unless a separately granted and
audited impersonation capability is present; every confirmed client SQL limit
is bounded; outbound operator URLs share one DNS-pinned, no-redirect SSRF
guard; deploy probes, proxy attribution, and image pins fail safe; the browser
and published documentation now communicate state honestly; and the one model
experiment remains flag-off, advisory-only, and measurable.

| Slice | Evidence | Result |
|-------|----------|--------|
| 408.1 | #964; member-identity denial matrices; docs-number contracts | REST and MCP personal-state operations self-scope bearer and session callers. Cross-member action requires `member:impersonate`, is workspace-bound, and is audited. Confirmed client-provided limits are clamped before SQL. |
| 408.2 | #973; egress/proxy tests; manifest checks | Webhooks, slash commands, federation, OIDC discovery, and the experimental advisor use the canonical URL parser, public-address resolution, DNS pinning, and redirect refusal. Kubernetes probes use shallow liveness, proxy hops default to zero, and MinIO inputs are digest-pinned. |
| 408.3 | #974; UI contract + Playwright | Loading, error, and empty states are explicit. Approvals refresh on entry and after mutation, and problem details reach the operator instead of becoming generic failures. |
| 408.4 | #975; mdBook build; presentation contract; screenshot suite | Published Mermaid diagrams render, flattened links and the edit affordance are repaired, and the locked Sweep Reach mark, favicon, social card, palette, `/ui` header, and screenshots ship as one guarded system. |
| 408.5 | #976; exact-contract mock; no-write e2e; evaluation harness | Optional Jev advice returns raw confidence/probabilities, threshold policy, latency, and usage without arming or writing the land gate. Provider failure is local to the advice call. Graduation requires independently labelled calibration. |

## Decisions

- Work attribution remains an orchestrator capability; personal member state
  does not. `member:impersonate` makes the exceptional cross-member case
  explicit and auditable instead of preserving implicit bearer omnipotence.
- URL safety is resolved after DNS and pinned into the client connection. A
  string allow/deny check would still admit DNS rebinding and redirect pivots.
- Browser honesty includes intermediate states. A correct final response is
  insufficient when a human cannot tell whether an approval is loading, stale,
  empty, or failed.
- The selected visual identity is a release contract, not another design
  exploration. Source assets, rendered metadata, UI use, and screenshots are
  checked together.
- The Jev spike is intentionally a separate HTTP advice surface. It cannot
  write the pointer, cannot enter close enforcement, and has no MCP twin while
  experimental.

## What surprised us

- The self-only comments were false for bearer tokens, and the gap was broader
  than the audit described: MCP member arguments could cross workspaces before
  a handler saw an authenticated acting identity.
- One canonical SSRF client needed to cover more than the three filed call
  sites. OIDC discovery and the later model-provider spike inherited the same
  boundary, which is evidence that the abstraction is at the right layer.
- CI had accepted a README health command that could never succeed against the
  released image. A green pipeline proved only its own path, not the published
  stranger-start path.
- A line-oriented citation sweep and a successful mdBook build each missed
  presentation failures: wrapped matches survived the former, while Mermaid
  source and a dark-theme SVG survived the latter without visual QA.
- HTTP capability-map parity caught the new advice route before merge. Two
  inventories can share a blind spot; executable derivation still matters.

## Residual risk and follow-up

- F-52 remains a separately dispositioned configuration-hardening item: fail on
  unknown `MAIDAN_*` variables. It was not in #971's PR ladder or exit criteria
  and is not silently claimed here.
- F-43 is a maintainer-side branch-protection setting. Code and docs cannot
  prove whether “require branches to be up to date” is enabled.
- The Jev provider has exact local contract coverage but no production
  calibration result: the repository has neither a provider credential nor an
  independently labelled dataset. The feature therefore remains off and must
  not graduate on the strength of vendor confidence alone.
- The shared egress guard rejects non-public resolution and redirects, but it
  cannot make an approved public third party trustworthy. Operators still own
  destination and data-governance policy.

## Release ledger

| Item | Value |
|------|-------|
| Tag | `v408.0.0` |
| Roadmap | 2026-09-21 full-audit adopted set closed |
| Database compatibility | No migration |
| API compatibility | Adds `member:impersonate`; adds a default-off `POST /threads/:id/land-gate/advice`; no route removed |
| Runtime behavior | Safer personal-state authorization and egress defaults; UI/docs presentation improvements; advisor absent unless explicitly enabled |
| New release gates | Cross-surface member denial matrices, bounded-limit contracts, egress/proxy tests, UI state checks, docs presentation contract, advisor no-write tests |
