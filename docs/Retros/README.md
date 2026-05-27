# Retrospectives

One note per cluster, written as the closing PR of that cluster. The
retro is mandatory; the release tag cannot be cut without it.

## Shape

```markdown
# Cluster <X> retro — <Theme>

> Closing wave for Cluster <X> · target tag `v0.X.Y`

## What shipped
- PR #<n>: <title> — <one-line summary>
- ...

## What was deferred

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster <X+1>| <thing> | <reason>   |

## Surprises

Things learned that weren't anticipated by the cluster plan.

## Decisions

Architecture or vocabulary choices that locked differently than the plan
suggested. Each entry should note whether [[Architecture]] needs amending.

## Capability table extension

What new capabilities Maidan now has. Format matches [[Capabilities]]:

| Capability | First available in |
|------------|--------------------|

## Risks identified + mitigated

## Risks identified + still open

## Forward look

What the next cluster will tackle first. Cross-references the next
cluster note.

## Acknowledgements

PR review credit; external contributors.
```

## Index

- [[Cluster A]] — Foundation. Closed at `v0.0.1`.
- [[Cluster B]] — Routing + event bus + MCP. Closed at `v0.1.0`.
- [[Cluster C]] — Search + indexing. Closed at `v0.2.0`.
- [[Cluster G]] — Agent-to-agent federation. Closed at `v0.6.0`.
- [[Cluster H]] — Web UI + MCP stdio + polish. Closed at `v0.7.0`.
- [[Cluster 1.0]] — Production gates. Closed at `v1.0.0`.
- [[Minor 1.1]] — Delivery reliability. Closed at `v1.1.0`.
- [[Minor 1.2]] — Search + embeddings. Closed at `v1.2.0`.
- [[Minor 1.3]] — Semantic search UX. Closed at `v1.3.0`.
- [[Minor 1.4]] — Auth hardening. Closed at `v1.4.0`.
- [[Cluster 2.0]] — OIDC identities and human sessions. Closed at `v2.0.0`.
- [[Cluster 2.1]] — OIDC operator hardening. Closed at `v2.1.0`.
- [[Cluster 3.0]] — Search & subscriber depth. Closed at `v3.0.0`.
