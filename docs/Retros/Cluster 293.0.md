# Cluster 293.0 retro — GitHub repo metadata

> Tag **`v293.0.0`**. Phase XXIV (post-gate hardening). Launch-readiness P1/P2. No new
> gate tag.

## What shipped

Discoverability + contribution polish for the public repo:

- **Homepage** set to the published docs site
  (`https://david-engelmann.github.io/maidan/`) via `gh repo edit`.
- **Topics** (10): `rust`, `multi-agent`, `mcp`, `model-context-protocol`, `a2a`,
  `ai-agents`, `agent-infrastructure`, `agentic`, `postgres`, `websocket`.
- **Issue templates** in `.github/ISSUE_TEMPLATE/`: `bug_report`, `protocol_compat`
  (MCP/A2A conformance, cite the spec section), `benchmark` (a named-config result per
  `docs/Benchmark.md`), plus a `config.yml` with a docs contact link.

## Surprises / decisions

- **Repo description was already the pitch** ("The operating layer for teams of AI
  agents") — left as-is.
- **Markdown issue templates, not YAML forms.** Markdown front-matter templates always
  render (no strict schema to get wrong); the three cover the launch-relevant report
  kinds. `config.yml` keeps blank issues enabled + points at the docs.
- **Homepage → the docs site**, not the repo itself — the mdBook is the natural landing
  page for someone arriving from the topics.
- **Terminal GIF / screenshot deferred** — that needs a recorded asset (a real capture),
  which is a manual step; logged as a small follow-up.

## Capability table extension

Repo metadata / contribution docs — no capability change.

## Risks identified + still open

- Terminal GIF / screenshot for the README + repo card is still a manual follow-up.

## Forward look

Launch-readiness polish is done. Next: the **SDK arc** (294+) — 4-language clients
(TS→Python→Go→Rust) to a usable 0.1.0 against the frozen v1 contract + black-box tests.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 292.0]].
Topics/homepage set via `gh repo edit` under the standing external-action authorization.
