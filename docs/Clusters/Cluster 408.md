# Cluster 408 — Full-audit remediation: self-scoping, bounds, egress, UI, docs

> Post-gate hardening · target tag `v408.0.0`

The 2026-09-21 full audit is dispositioned in
[Open Work](../Open%20Work.md#full-audit-dispositions--adjusted-roadmap-2026-09-22).
This cluster ships the items dispositioned *adopt* or *adapt*, in the order
recorded there — chosen to stay clear of the concurrent Cluster 406 work.

## Contract

- A token acts on the personal state of the member it was minted for, and no
  one else's, on HTTP and MCP alike. Acting for another member is a distinct,
  explicitly-granted, audited capability — not a side effect of holding an
  ordinary read or write token.
- Work attribution is not personal state. An orchestrator posting or claiming
  *as* a worker is the product model and stays unrestricted.
- Every client-supplied `limit` is bounded before it reaches SQL.
- Server-side fetch of an operator-supplied URL cannot be pointed at link-local
  or private address space.
- Probes point at the shallow liveness endpoint, never the dependency-checking
  one; a transient datastore blip must not restart the pod.
- The `/ui` says what it is doing, what went wrong, and what an empty pane
  means.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 408.1 | current | Member self-scoping (D-5) + client `limit` bounds (F-46) |
| 408.2 | planned | SSRF egress guard (F-47, F-58), probe targets (F-49), proxy-hop trust (F-50), pinned image tags (F-56) |
| 408.3 | planned | `/ui` P1: loading states, approvals refresh, error bodies, empty states |

408.3 is UX polish and is deliberately distinct from Cluster 407's `/ui` work
(#963), which is contract coverage. If 407 lands first, 408.3 builds on it.
| 408.4 | planned | Docs rendering + the locked branding ship-list |
| 408.5 | planned | J-01 land-gate spike, behind a flag, flag-off |
| 408.close | planned | Ledgers and retrospective |

## Non-goals

- Backwards compatibility. Nothing has ever consumed this repo, so a breaking
  change costs nothing and must not weigh against a correct design.
- Branch protection settings. Maintainer-side, flipped in the GitHub UI; noted
  as pending-maintainer rather than blocking a slice.
- Re-opening the logo. The mark is locked; 408.4 ships it, it does not explore.
