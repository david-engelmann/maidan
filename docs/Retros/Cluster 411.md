# Cluster 411 retro — delegated authority

> Post-gate hardening · `v411.0.0` · PRs #996/#997/#999/#1000/#1001/#1002/#1003/#1004/#1005/#1006/#1007/#1008 + close record

## Outcome

A token acts as exactly one member, and acting for someone else is explicit,
bounded and on the record. Ordinary requests can no longer name who they act
as. An orchestrator gets a delegation grant per agent, exchanges it for a
short-lived token that *is* the agent, and every use is recorded with the
delegate as actor, the agent as subject, and the grant. Delegation lends work
and never authority. It is one hop, and it cannot be used to approve your own
work. Every durable record, and every live frame, names who acted and for
whom, and a change that records nothing itself is recorded anyway.

| Slice | PR | Result |
|-------|----|--------|
| 411.1 | #996 | Closed the confirmed D-5 holes: member-skill self-scope, auth-bound share-ticket ownership, context and search clamps, argument-keyed MCP identity drift coverage. |
| 411.2 | #997 | `maidan_delegation_grants` store foundation, both backends. |
| 411.3 | #999 | `POST /tokens/delegate` + MCP `delegate_token`: bounded lifetime, capability intersection, revocation cascade. |
| 411.4 | #1000 | **Breaking:** removed every caller-chosen acting-identity field; identity comes only from authentication. |
| 411.5 | #1001 | Grant administration; durable delegated authorization evidence for refusals as well as successes. |
| 411.6 | #1002 | Retired `member:impersonate`; split skills into self-declared routing tags and operator-conferred governance authority; closed the bearer exemption on DM rosters. |
| 411.7 | #1003 | Delegation cannot escalate: capabilities classified as work or authority, only work lendable, borrowed contexts stripped of authority, exchange one hop and authorized on the real actor. |
| 411.8 | #1004 | Every event and audit row records actor, subject and grant, inside the hashed payload, so rewriting who did something breaks the chain. |
| 411.9 | #1005 | Destroying or rewriting the record needs authority: purge and erase need `token:admin`, moderation needs `channel:admin`, and only a message's author can edit it. Also fixed the required `docker compose smoke` check, red on `main` since #973. |
| 411.10 | #1008 | Approvals may be borrowed, never self-approved: the actor is recorded on the worker ledger and every attestation, and every separation-of-duties check tests it. Accepting your own approval gate is refused. |
| 411.11 | #1006 | Nothing changes without a record: 85 REST routes that wrote no event or audit row are recorded by the request layer, structurally. |
| 411.12 | #1007 | Live WebSocket and MCP-SSE frames carry attribution, full and lean. |

## Decisions

- **The acting identity comes from authentication, never from a request.**
  There is no standing act-as-any capability, and no body field that selects a
  member.
- **Delegation lends work, never authority.** A token minted with borrowed
  `token:admin` would outlive the grant, and borrowed authority is supposed to
  be short-lived and revocable. Every capability is classified, and a test
  fails if a new one isn't.
- **The record is written where attribution is bound, not at each call site.**
  The request's attribution scope is read at the one place each backend
  appends an event, writes an audit row, records a worker or stores an
  attestation. The 411.8 audit found 25 of 31 audit sites recording the wrong
  member, and the 411.11 sweep found 85 routes recording nothing. Both are the
  cost of relying on each handler to remember.
- **Destroying the record is authority; withdrawing your own words is not.**
  Purge and erase need `token:admin`. Tombstoning someone else's message needs
  `channel:admin`. Nobody edits another member's words, because the edited
  message would still carry the author's name.
- **Approvals may be borrowed, never self-approved** (maintainer's decision).
  *Own credential only* was rejected as making delegation useless for review
  automation. *Just record it* was rejected as leaving the laundering open.

## What surprised us

- **Three findings were wrong, and all three for the same reason.** "Narrowing
  sheds the grant", "validity rests only on the cascade", and "a delegate can
  redirect email with nothing recorded" each came from reading a handler and
  not the query or middleware behind it. Mutation testing caught the first two:
  a test that still passed with its fix disabled meant there had been nothing
  to fix.
- **A required check was red on `main` for about 30 PRs.** `docker compose
  smoke` had failed at the same step since #973's egress guard, and every PR
  since was admin-merged over it, including this cluster's first eight. The
  cause was one missing environment line. A note to "rerun, don't
  re-diagnose" had been applied to a failure it didn't describe.
- **Approval gates had no self-approval rule at all**, delegated or not. That
  surfaced only because the delegated case forced the question.
- **The moderation gates meant "moderator" and wrote `workspace:write`**, which
  every agent holds. Nothing had ever tested a non-author edit with a real
  token, so the old policy was never asserted and couldn't visibly regress.

## Method

Every fix was mutation-checked: each condition was disabled on its own, and
only its own test was allowed to fail. Over the cluster, more than forty
mutations were run and each was caught. The harness itself had two bugs worth
knowing. It restored files with an older mtime, so cargo kept linking the
mutated build. It also truncated a file before reading the mutation from it.
Two mutations failed to compile, and were replaced, because a mutation that
doesn't compile tests nothing.

## Residual risk and follow-up

- A `mutation` row records *that* a change happened and who made it, not the
  new value. For most routes the current state is the value. Review verdicts
  and approval answers are overwritable, and have no history of earlier values.
- Separation of duties still doesn't address a genuinely colluding pair of
  members. It never did.
- The measurement that found the 85 routes was one-off. A contract test that
  classifies every mutating route, the way capabilities are classified, would
  make the structural guarantee visible in CI rather than implicit.
