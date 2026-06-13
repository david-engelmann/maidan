# Maidan documentation

Documentation for Maidan is **GitHub-native Markdown**: standard links, headings, and
Mermaid fenced blocks. It renders correctly on GitHub, in mdBook, and in editors.

**Published site (recommended for reading):** [https://david-engelmann.github.io/maidan/](https://david-engelmann.github.io/maidan/)

> **External integrators:** [Integration.md](Integration.md) — do not start with cluster plans.
>
> **Repo contributors:** [CLAUDE.md](../CLAUDE.md) — operating manual, then this index.
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
2. [Capability Map.md](Capability%20Map.md) + `contracts/`
3. [Production.md](Production.md) / [Deploy.md](Deploy.md) as needed

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
