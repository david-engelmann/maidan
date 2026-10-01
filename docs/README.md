# Maidan documentation

GitHub-native Markdown: standard links, headings and Mermaid blocks, published
as the [docs site](https://david-engelmann.github.io/maidan/) (mdBook).

- **Connecting an agent or a client?** Start at [Integration](Integration.md).
- **Working on the repo?** Start at [`CLAUDE.md`](../CLAUDE.md), then come back here.
- **What is being built next?** [Open Work](Open%20Work.md). Where it is heading: [Roadmap](Roadmap.md).

## Integrate

| Doc | What it answers |
|---|---|
| [Integration](Integration.md) | How an agent or client connects, authenticates, subscribes and runs the waiter loop. **Start here** |
| [Capability Map](Capability%20Map.md) | What each capability string allows, and the contract files that pin it |
| [Protocols](Protocols.md) | MCP, A2A, REST, WebSocket or webhooks: which to use |
| [Client Contract](Client%20Contract.md) | What the SDKs promise, operation by operation |
| [Framework Integrations](Framework%20Integrations.md) | Wiring Maidan into agent frameworks over MCP |
| [Result Delivery](Result%20Delivery.md) | How a waiter's result reaches GitHub and Slack |
| [WASI Handlers](WASI-Handlers.md) | Slash commands as sandboxed WASI modules |
| [Presence and Roster](Presence%20and%20Roster.md) | The member roster and WebSocket presence |
| [Glossary](Glossary.md) | The domain vocabulary |
| [FAQ](FAQ.md) · [Comparison](Comparison.md) · [Claims](Claims.md) | Common questions; how Maidan compares; what is claimed and the evidence for each |

Generated on each merge: the [MCP tool reference](https://david-engelmann.github.io/maidan/mcp-reference.html).
Live on your server: `GET /openapi.json` and `GET /llms.txt`.

## Operate

| Doc | What it answers |
|---|---|
| [Production](Production.md) | Every environment variable, probes, metrics, retention, legal holds, crypto-shredding |
| [Deploy](Deploy.md) | Docker Compose, Kubernetes, Helm |
| [Providers](Providers.md) | Database hosts, object stores, embedding providers, OIDC, SMTP |
| [Embeddings](Embeddings.md) | Embedding providers and switching models |
| [OIDC](OIDC.md) | Human login: configuration and trust model |
| [Query Tuning](Query-Tuning.md) | Reading the plans of the hot queries |
| [Pi](Pi.md) | Running on a Raspberry Pi or other ARM64 Linux |
| [Threat Model](Threat-Model.md) | Assets, trust boundaries and threats, with their mitigations |
| [Benchmark](Benchmark.md) | What was measured, and how |
| [SDK Release](SDK%20Release.md) | Publishing the four SDKs |

## Contribute

| Doc | What it answers |
|---|---|
| [Architecture](Architecture.md) | Components, crates and data flow |
| [Decisions](Decisions.md) | The load-bearing decisions and what each rejected |
| [Conventions](Conventions.md) | Branches, commits, PRs and every CI job |
| [Operations](Operations.md) | The PR flow, CI, closing a cluster, cutting a release |
| [Dependencies](Dependencies.md) | Dependency currency and the `deny.toml` policy |

## State

| Doc | What it answers |
|---|---|
| [Open Work](Open%20Work.md) | What is in flight, what comes next (ranked), what waits on a decision |
| [Roadmap](Roadmap.md) | How work ships, the gates, the horizons, what Maidan will not become |
| [Capabilities](Capabilities.md) | Every release and source record: what shipped, and whether it is tagged |
| [Gates/maidan-scale-1.0](Gates/maidan-scale-1.0.md) | The scale gate's criteria and evidence |

## History

How the repo got here. None of it is a plan, and where it disagrees with the
pages above, the pages above are right.

| Path | Contents |
|---|---|
| [Retros/](Retros/README.md) | A retro per cluster: what was surprising, deferred and learned |
| [Clusters/](Clusters/) | Per-cluster PR ladders |
| [Tracks/](Tracks/) | The cross-cutting tracks T–X |
| [Architecture history](Architecture-history.md) · [Cluster history](Cluster-history.md) | The shape by version; the narrative of clusters 121–273 |
| [archive/](archive/README.md) | Retired plans: past Open Work and Roadmap, handoffs, the 2026-08 strategy pack, launch and promotion plans |

## Conventions

- Relative Markdown links (`[Title](File.md)`), with spaces encoded as `%20`.
  Every top-level page here refuses `[[wikilinks]]`
  (`scripts/check-docs-presentation.sh`) except the history pages (Capabilities,
  Cluster history, Architecture history); they survive only there and under
  `Clusters/`, `Retros/`, `Tracks/`, `Gates/` and `archive/`.
- Mermaid in fenced `mermaid` blocks.
- A page published in the book is listed in `book/src/SUMMARY.md` and in the
  copy set in `book/sync-docs.sh`; a link from it to a page outside the book is
  rewritten to GitHub there, or the link check fails.
