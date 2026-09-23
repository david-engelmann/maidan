# Cluster 411 — grant-backed delegated authority

> Post-gate hardening · target tag `v411.0.0` · umbrella issue #994

## Contract

- An ordinary bearer or session acts only as `auth.member_id`. Acting identity
  never comes from a REST body, path parameter, query, or MCP argument.
- Cross-member action has one explicit path: an administrator authorizes a
  durable delegation grant, then the delegate exchanges it for a short-lived
  token bound to the grant's subject.
- A delegated token lasts 15 minutes by default and never more than one hour.
  Its authority is the intersection of the grant, delegate, and subject
  capability sets; no exchange can widen authority.
- Revoking a grant invalidates every token minted from it, including attenuated
  descendants, using the existing token-revocation cascade.
- Delegated decisions carry actor, subject, and `grant_id` — allowed **and
  refused** alike — and a refused one is written durably, not sampled.
  **Revised 2026-09-23 on David's decision:** an earlier draft kept denials as
  bounded operational evidence, on the Cluster 182/410 write-amplification
  reasoning. That reasoning covers *anonymous* refusals, which an
  unauthenticated stranger can generate without limit. A delegated refusal comes
  from a named agent holding a short-lived, revocable grant, so its volume is
  bounded by something we issued. Sampling at 1-in-64 with no principal in the
  metric would leave a misbehaving delegate unreconstructable, which is the
  opposite of the point. Anonymous denials keep their sampled lane.

## Vocabulary decisions

- **Actor** is the member presenting authority; **subject** is the member the
  delegated token represents. They are equal for ordinary calls.
- A **grant** is the durable consent and policy object. An exchanged token is a
  short-lived credential derived from it, not the consent record itself.
- Grant creation is administrator-only in this cluster. Subject-consent UI and
  resource-level constraints are later work; capability-level scoping ships
  first.
- `member:impersonate` is transitional vocabulary, not a second delegation
  mechanism. It is removed only after the grant-backed replacement works.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 411.1 | #996 | Close the confirmed D-5 holes: member-skill self-scope, auth-bound share-ticket ownership, context/search clamps, and argument-keyed MCP identity drift coverage |
| 411.2 | #997 | `maidan_delegation_grants` models, migrations, and both store backends; no protocol surface |
| 411.3 | #999 | REST `POST /tokens/delegate` + MCP `delegate_token`, bounded lifetime, capability intersection, and revocation cascade |
| 411.4 | #1000 | Breaking removal of caller-chosen acting-identity fields; derive ordinary identity solely from authentication |
| 411.5 | #1001 | Grant create/list/revoke surfaces and actor + subject + `grant_id` authorization evidence, durable for delegated refusals as well as successes |
| 411.6 | #1002 | Remove `member:impersonate` and add the absence contract; split member skills into self-declared routing tags and operator-conferred governance authority; close the bearer exemption on DM and group-DM metadata |
| 411.7 | current | Delegation cannot escalate: every capability is classified as work or authority and only work is delegatable; a borrowed context cannot hold authority whatever its grant says; exchange is one hop and authorised on the real actor |
| 411.8 | planned | Attribution everywhere — every audit row and domain event names the actor, the subject, and the grant, so a delegated action is distinguishable from a direct one and can be traced to what it produced |
| 411.9 | planned | Attestations are personal: whether an approval may be made with a borrowed token (awaiting the maintainer's decision) |
| 411.close | close record | Ledgers, retrospective, and `v411.0.0` tag |

## Exit criteria

- No ordinary REST or MCP request can select a different acting member.
- Delegation is explicit, expiring, capability-intersected, revocable, and
  attributable to actor, subject, and grant. **Not met at 411.6:** 25 of 31
  privileged audit writes recorded the subject as the actor, and no domain event
  carried the delegate at all. 411.8 closes it.
- Grant revocation invalidates direct exchanged tokens and their attenuation
  descendants.
- Both stores and both public protocol surfaces have executable parity evidence.
- Every MCP schema with a `member_id` argument is classified by semantics, so a
  tool cannot evade the identity guard by living in a different source module.
- `member:impersonate` no longer exists in the vocabulary or any capability set.

## Non-goals

- Subject-consent UI, multi-party approval, resource-level conditions, or a
  general policy language.
- Long-lived delegated tokens or capability union.
- Durable rows for every *anonymous* authorization denial — those keep the sampled lane. Delegated denials are durable; see the contract above.
- Compatibility shims for the caller-chosen acting-identity fields; the repo is
  pre-1.0 and the removal is intentionally breaking.
