# Maidan documentation

Documentation for Maidan is **GitHub-native Markdown**: standard links, headings, and
Mermaid fenced blocks. It renders correctly on GitHub, in mdBook, and in editors.

**Published site:** [https://david-engelmann.github.io/maidan/](https://david-engelmann.github.io/maidan/) (mdBook). A `maidan.world` product domain (landing + `/docs` + `/blog`) is **planned** for the public preview but is not registered/live yet — the cutover plan is in [Promotion.md](Promotion.md); use the GitHub Pages URL today.

> **External integrators:** [Integration.md](Integration.md) — do not start with cluster plans.
>
> **Repo contributors:** [CLAUDE.md](../CLAUDE.md) — operating manual, then this index.
>
> **Post-272 forward work:** the canonical backlog is [Open Work.md](Open%20Work.md) / [Roadmap.md](Roadmap.md). The strategy pack ([Handoff.md](Handoff.md) → Pre-Public Hardening, Path to Impressive, Expansion Bets, Launch, Protocols, Providers) is the rationale and detailed scoping behind those items — read it for the "why," not as a separate backlog.
>
> **Obsidian (optional, local only):** open `docs/` as a vault for graph view. Some
> historical notes still contain `[[wikilinks]]`; prefer the published site or
> [Integration.md](Integration.md) for links that must work on GitHub.

## Integrate with Maidan

| Doc | Audience |
|-----|----------|
| [Integration.md](Integration.md) | Agents, bots, client apps — **start here** |
| [Capability Map.md](Capability%20Map.md) | Capability strings + `contracts/*.json` |
| [Production.md](Production.md) | Probes, env vars, bootstrap, metrics |
| [Embeddings.md](Embeddings.md) | Embedding providers, per-model tables, switching models (reindex) |
| [Providers.md](Providers.md) | Plug-in matrix: DB hosts, S3, embeddings, OIDC, SMTP |
| [Protocols.md](Protocols.md) | Integration wires: MCP, A2A, REST, WS, webhooks — what we speak vs 2026 stack |
| [Deploy.md](Deploy.md) | Docker Compose, Kubernetes, Helm |
| [Pi.md](Pi.md) | Raspberry Pi / ARM64 Linux |
| [Threat-Model.md](Threat-Model.md) | Security assets and controls |
| [Glossary.md](Glossary.md) | Domain vocabulary |

Generated on each merge: [MCP tool reference](https://david-engelmann.github.io/maidan/mcp-reference.html) (from `book/src/mcp-reference.md`).

Live API: `GET /openapi.json` on your server.

## Design and operations (maintainers)

| Doc | Purpose |
|-----|---------|
| [Architecture.md](Architecture.md) | Components and data flow |
| [Capabilities.md](Capabilities.md) | What shipped in each release (append-only) |
| [Decisions.md](Decisions.md) | Architectural decisions (ADRs) |
| [Conventions.md](Conventions.md) | Branch, commit, PR conventions |
| [Operations.md](Operations.md) | PR flow, CI, releases |
| [Dependencies.md](Dependencies.md) | Dependency currency + duplicate-version policy (`deny.toml`) |
| [Gates/maidan-scale-1.0.md](Gates/maidan-scale-1.0.md) | Scale product gate (`v120.0.0`): criteria → evidence |
| [Handoff.md](Handoff.md) | Strategy index for post-272 forward work (feeds [Open Work.md](Open%20Work.md); IDs, try-out matrix, rationale) |
| [Launch.md](Launch.md) | Production-ready extras, public-preview cut, when you may announce |
| [Promotion.md](Promotion.md) | Get the word out: **maidan.world**, docs hub, Show HN, Reddit, LinkedIn, Medium |
| [Pre-Public Hardening.md](Pre-Public%20Hardening.md) | Cleanup/refactor/tests/docs before a public launch |
| [Path to Impressive.md](Path%20to%20Impressive.md) | Strategy: UI assurance, adoption gaps, usefulness bets |
| [Expansion Bets.md](Expansion%20Bets.md) | Researched feature bets after 270-272 (Slack teammate, MCP pack, SDKs, mail queue) |
| [Open Work.md](Open%20Work.md) | Short backlog + risks |
| [Remaining Work.md](Remaining%20Work.md) | Exhaustive backlog matrix |
| [Roadmap.md](Roadmap.md) | Cluster ladder history |
| [Post-1.0.md](Post-1.0.md) | Tracks after v1.0.0 |

## Historical planning (not required for integration)

Cluster kickoff docs and retros document **how the repo was built**, not the runtime contract.

| Path | Contents |
|------|----------|
| [Clusters/](Clusters/) | Per-cluster PR ladders (may use Obsidian wikilinks) |
| [Retros/](Retros/) | Closing retrospectives |
| [Tracks/](Tracks/) | Cross-cutting tracks T–X |
| [Clusters/Product Ladder 77+.md](Clusters/Product%20Ladder%2077+.md) | Operator ladder 77–101 (closed on `main`) |

## Suggested read order

### Integrating with a running server

1. [Integration.md](Integration.md)
2. [Protocols.md](Protocols.md) if choosing MCP vs A2A vs REST vs webhooks
3. [Capability Map.md](Capability%20Map.md) + `contracts/`
4. [Production.md](Production.md) / [Deploy.md](Deploy.md) / [Providers.md](Providers.md) as needed

### Contributing to the repository

1. [CLAUDE.md](../CLAUDE.md)
2. [Architecture.md](Architecture.md)
3. [Operations.md](Operations.md)
4. [Decisions.md](Decisions.md)
5. [Open Work.md](Open%20Work.md)

## Layout

```
docs/
├── README.md              this index
├── Integration.md         canonical external integrator guide
├── Handoff.md             post-D pack pickup (agents start here)
├── Launch.md              public cut + announce
├── Providers.md           host matrix
├── Protocols.md           wire matrix
├── Architecture.md
├── Roadmap.md
├── Capabilities.md
├── Capability Map.md
├── Conventions.md
├── Operations.md
├── Decisions.md
├── Production.md
├── Deploy.md
├── Clusters/              historical planning
└── Retros/                historical retros
```

## Conventions

- **Prefer relative Markdown links** (`[Title](File.md)`) in new and integrator-facing docs.
- **Mermaid** in fenced ` ```mermaid ` blocks (GitHub + mdBook).
- **Filenames** may contain spaces; URL-encode in links (`%20`) when required.
- Older vault notes may use `[[wikilinks]]` for Obsidian only — do not add new wikilinks to integrator-facing pages.
