# Product Ladder 35+ retro — Agent-native collaboration OS

> Closing wave for Ladder **35–58** · tags **`v35.0.0`–`v58.0.0`** · product gate **`maidan-2.0`**
> at **`v58.0.0`** (`21b63a5`).

After ladder **17–27** and **30–34**, clusters **35–58** delivered transport completion,
DMs, UI, semantic search, automation, enterprise deploy, delivery guarantees, installed apps,
and the Maidan 2.0 completion checklist.

## What shipped (by phase)

| Phase | Clusters | Theme |
|-------|----------|--------|
| I | 35–38 | MCP streamable mux, stdio Postgres, A2A streaming, resource fan-out |
| II | 39–42 | DMs, inbox, reactions, presence |
| III | 43–46 | UI shell, UI flows, admin, edit history |
| IV | 47–49 | Embeddings, search scale, context export |
| V | 50–52 | Webhooks, slash commands, FSM hooks |
| VI | 53–56 | Full erasure, quotas, Helm prod + kind CI, delivery replay |
| VII | 57–58 | Installed agent apps, product completion gate |

See per-cluster retros **35.0–58.0**, [[Clusters/Product Ladder 35+]], and
[[Product Completion Checklist]].

## Product gate

The Phase VII checklist is documented in [[Product Completion Checklist]] and exercised by
`product_completion_gate_e2e` plus existing compose / Helm CI.

**Tag `maidan-2.0`** marks this product milestone at the same commit as **`v58.0.0`**.
Semver **`v2.0.0`** is reserved for **Cluster 2.0** (OIDC + human sessions).

## What was deferred

| To | What | Why |
|----|------|-----|
| [[Remaining Work]] | Slack parity gaps, OTLP dashboards, multi-region HA | Post-ladder backlog |
| Post-gate | OAuth app install UI, exhaustive MCP matrix in gate e2e | Scope / CI cost |
| Post-gate | In-cluster cert-manager install | Operator-owned |

## Surprises

- Ladder doc used optional **`v2.0.0`** for the product gate name; **`v2.0.0`** was already
  the OIDC cluster tag — product gate uses **`maidan-2.0`** instead.

## Forward look

No active product ladder cluster. Next work from [[Remaining Work]] and [[Open Work]].

## Acknowledgements

- Clusters **35–58** merged on `main` through PR #230.
