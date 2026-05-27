# Maidan documentation

An [Obsidian](https://obsidian.md/) vault holding the design, roadmap,
and operating conventions for Maidan. Open this folder as a vault in
Obsidian for wikilink navigation, graph view, and backlinks.

> **Agents:** start at [`../CLAUDE.md`](../CLAUDE.md). It's the
> operating manual; this vault is the reference.
>
> **Published site:** [mdBook](https://david-engelmann.github.io/maidan/)
> (built from `book/` + this vault on every merge to `main`).

## Index

- [[Architecture]] — what the system is and how the pieces connect.
- [[Roadmap]] — clusters from foundation to v1.0.
- [[Post-1.0]] — tracks + optional minors after v1.0.0.
- [[Tracks/README]] — cross-cutting tracks T–X.
- [[Capabilities]] — running list of what Maidan can do, by release.
- [[Conventions]] — branch, commit, and PR conventions.
- [[Operations]] — daily commands, PR flow, cluster kickoff + close,
  CI debugging, release workflow troubleshooting.
- [[Decisions]] — every load-bearing architectural decision with
  rationale and the alternative that was rejected.
- [[Open Work]] — every deferred item across retros + standing
  risks. The "what could I work on" backlog.
- [[Deploy]] — Docker + Kubernetes deployment.
- [[Production]] — probes, env, bootstrap; links `GET /openapi.json`.
- [[Threat-Model]] — assets, threats, bootstrap hardening (Track V).
- [[OIDC]] — human login design spike (implementation deferred to `v2.0.0`).
- [[Query-Tuning]] — Postgres `EXPLAIN` playbook (Track U).
- [[Glossary]] — domain vocabulary.
- [[Clusters/Cluster 10.0]] — closed at `v10.0.0` (Postgres transactional outbox).
- [[Clusters/Cluster 9.0]] — closed at `v9.0.0` (coverage depth).
- [[Clusters/Cluster 8.0]] — closed at `v8.0.0` (bus hydrate observability).
- [[Clusters/Cluster 7.0]] — closed at `v7.0.0` (bus pointer delivery).
- [[Clusters/Cluster 6.0]] — closed at `v6.0.0` (delivery reliability).
- [[Clusters/Cluster 5.0]] — closed at `v5.0.0` (coverage & search quality).
- [[Clusters/Cluster 4.0]] — subscriber continuity (`v4.0.0`, closed).
- [[Clusters/Cluster 3.0]] — search & subscriber depth (`v3.0.0`, closed).
- [[Clusters/Cluster 2.0]], [[Clusters/Cluster 2.1]] — recent OIDC waves.
- [[Clusters/Cluster A]], [[Clusters/Cluster B]], [[Clusters/Cluster C]]
  — per-cluster plan docs with PR ladder + risks.
- [[Retros/README]] — closing-wave retrospectives, one per cluster.
  Index lists all completed retros.

## Layout

```
docs/
├── README.md              this file (vault index)
├── Architecture.md        what the system does
├── Roadmap.md             cluster ladder
├── Capabilities.md        what ships in which release
├── Conventions.md         branch + commit + PR conventions
├── Operations.md          daily ops + PR flow + release runbook
├── Decisions.md           load-bearing ADRs
├── Open Work.md           backlog + open risks
├── Deploy.md              Docker + k8s
├── Glossary.md            domain vocabulary
├── Clusters/
│   ├── Cluster A.md
│   ├── Cluster B.md
│   └── Cluster C.md
└── Retros/
    ├── README.md          retro template + index
    ├── Cluster A.md
    ├── Cluster B.md
    └── Cluster C.md
```

## Read order for a new agent

1. [`../CLAUDE.md`](../CLAUDE.md) — operating manual.
2. This file.
3. [[Architecture]] — what's connected to what.
4. [[Roadmap]] — where we are in the cluster ladder.
5. [[Capabilities]] — what already ships.
6. [[Decisions]] — why things are the way they are.
7. [[Operations]] — how to do the next thing.
8. [[Open Work]] — what to do next.
9. Most recent [[Retros/README|cluster retro]] — freshest tensions
   and surprises.

## Conventions inside the vault

- Wikilinks (`[[Note Name]]`) for internal references.
- Filenames use Title Case with spaces; Obsidian resolves them.
- Diagrams use [Mermaid](https://mermaid.js.org/) inside fenced code
  blocks.
- Each note begins with a one-paragraph summary so the Obsidian graph
  hover surfaces the right context.
- Append-only-ish: when a decision reverses, the original entry
  stays + a new entry records the reversal. See [[Decisions]].
