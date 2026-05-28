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
- [[Cluster 4.0]] — Subscriber continuity. Closed at `v4.0.0`.
- [[Cluster 5.0]] — Coverage & search quality. Closed at `v5.0.0`.
- [[Cluster 6.0]] — Delivery reliability. Closed at `v6.0.0`.
- [[Cluster 7.0]] — Bus pointer delivery. Closed at `v7.0.0`.
- [[Cluster 8.0]] — Bus hydrate observability. Closed at `v8.0.0`.
- [[Cluster 9.0]] — Coverage depth. Closed at `v9.0.0`.
- [[Cluster 10.0]] — Postgres transactional outbox. Closed at `v10.0.0`.
- [[Cluster 11.0]] — Coverage 11%. Closed at `v11.0.0`.
- [[Cluster 12.0]] — Outbox relay hardening. Closed at `v12.0.0`.
- [[Cluster 17.0]] — MCP resource fan-out. Closed at `v17.0.0`.
- [[Cluster 18.0]] — SQLite semantic search. Closed at `v18.0.0`.
- [[Cluster 19.0]] — S3 multipart artifacts. Closed at `v19.0.0`.
- [[Cluster 20.0]] — Message router. Closed at `v20.0.0`.
- [[Cluster 21.0]] — A2A agent transport. Closed at `v21.0.0`.
- [[Cluster 22.0]] — Capabilities hardening. Closed at `v22.0.0`.
- [[Cluster 23.0]] — Web UI product. Closed at `v23.0.0` (integration PR #198).
- [[Cluster 24.0]] — Helm deploy. Closed at `v24.0.0` (integration PR #198).
- [[Cluster 25.0]] — Privacy & erasure. Closed at `v25.0.0` (integration PR #198).
- [[Cluster 26.0]] — Product completion gate. Closed at `v26.0.0` (integration PR #198).
- [[Cluster 27.0]] — MCP streamable HTTP. Closed at **`v27.0.0`** (culminating ladder release).
- [[Cluster 28.0]] — Privacy complete (deep purge + audit). Closed at **`v28.0.0`**.
