# Roadmap

Maidan ships in clusters. Each cluster ends with a release tag and a
[[Retros/README|retrospective]]. Within a cluster, work is broken into
PRs tracked by the GitHub issues labelled with that cluster.

## Cluster ladder

| Cluster | Theme                                | Target tag |
|---------|--------------------------------------|------------|
| **A**   | Foundation: workspace, schema, /health | `v0.0.1` ✓ |
| **B**   | Routing + event bus + MCP surface    | `v0.1.0` ✓ |
| **C**   | Search + indexing                    | `v0.2.0` ✓ |
| **D**   | FSM-driven thread lifecycle          | `v0.3.0` ✓ |
| E       | Artifact substrate (S3, types, refs) | `v0.4.0`   |
| F       | Auth, workspaces, capabilities       | `v0.5.0`   |
| G       | Agent-to-Agent transport             | `v0.6.0`   |
| H       | Web UI                               | `v0.7.0`   |
| **1.0** | Production gates met                 | `v1.0.0`   |

## Cross-cutting tracks

These run in parallel with delivery clusters and do not have their own
tags; they raise the bar each time they ship.

| Track | Theme              | Notes                                   |
|-------|--------------------|-----------------------------------------|
| T     | Telemetry + perf   | OTLP, tracing, latency budgets.         |
| U     | Performance work   | Benchmarks, mutation tests, profiling.  |
| V     | Security + privacy | Threat models, GDPR, secret hygiene.    |
| W     | Documentation      | The vault, runbooks, API docs.          |
| X     | Release engineering| Tags, release notes, signed artifacts.  |

## Current cluster

Clusters A, B, C, D complete. See [[Retros/Cluster A]],
[[Retros/Cluster B]], [[Retros/Cluster C]], [[Retros/Cluster D]].
**Cluster E** (artifact substrate) is in progress — see
[[Clusters/Cluster E]].

## Closing a cluster

Each cluster closes with a dedicated retro PR that:

- Creates [[Retros/README|the retro note]] for that cluster.
- Updates [[Capabilities]].
- Updates the root `CHANGELOG.md`.
- Cuts the release tag.

This pattern is mandatory; tags are never cut without a retro.
