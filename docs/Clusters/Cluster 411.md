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
| 411.7 | #1003 | Delegation cannot escalate: every capability is classified as work or authority and only work is delegatable; a borrowed context cannot hold authority whatever its grant says; exchange is one hop and authorised on the real actor |
| 411.8 | #1004 | Attribution everywhere — every event carries actor, subject and grant inside its hashed payload, and every audit row records them, so a delegated action is distinguishable from a direct one and rewriting who did something breaks the chain |
| 411.9 | #1005 | Destroying the record needs authority: purging or erasing a workspace and hard-purging a message move from `workspace:write` to `token:admin`; tombstoning another member's message needs `channel:admin`; and no one but the author can edit a message. With this, 411.7's "delegation lends work, never authority" holds |
| 411.10 | #1008 | Approvals may be borrowed, never self-approved (maintainer's decision): the actor is recorded on the worker ledger, reviews, land-gate passes and approval gates, and every separation-of-duties check tests it; accepting your own approval gate is refused |
| 411.11 | #1006 | Nothing changes without a record: a successful REST mutation or MCP tool call that wrote no event or audit row gets an attributed `mutation` row from the request layer, closing 85 routes that recorded nothing; workspace export is recorded |
| 411.12 | #1007 | Live frames carry attribution: the bus envelope keeps the stored event's actor, subject and grant, so WebSocket and MCP-SSE subscribers see who acted — full and lean frames — without refetching |
| 411.close | close record | Retro, CHANGELOG, Capabilities, Roadmap; the `v411.0.0` tag is the maintainer's |

## Exit criteria

- No ordinary REST or MCP request can select a different acting member.
- Delegation is explicit, expiring, capability-intersected, revocable, and
  attributable to actor, subject, and grant. **Not met at 411.6:** 25 of 31
  privileged audit writes recorded the subject as the actor, and no domain event
  carried the delegate at all. **Met at 411.8** for every durable record, at
  411.11 for changes that recorded nothing, and at 411.12 for live
  WebSocket/SSE frames.
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
