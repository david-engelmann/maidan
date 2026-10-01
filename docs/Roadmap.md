# Roadmap

Direction, not a backlog. What to build next, ranked, is in
[Open Work](Open%20Work.md); what shipped is in [Capabilities](Capabilities.md)
and the root `CHANGELOG.md`; how the project got here is in
[Roadmap history](archive/Roadmap-history.md) and
[Cluster history](Cluster-history.md).

## How work ships

- **Clusters** are planned arcs: a number (411), a short ladder of PRs (411.1,
  411.2, …), and a retro in [`Retros/`](Retros/README.md) that records what was
  surprising, what was deferred and what was learned. Clusters A–H and 1.0 built
  the product; the numbered ladder (1–120) took it to the scale gate; clusters
  121–418 are post-gate hardening and product work.
- **Standalone PRs.** Since Cluster 418, most work lands as a PR taken from the
  ranked Next list in Open Work. Each carries its own PR-level retro and a
  `CHANGELOG.md` `[Unreleased]` entry and names the Open Work row it closes;
  the coordinator updates Open Work after the merge.
- **Releases** are cut when the maintainer chooses, as a `vN.0.0` tag that runs
  `release.yml`. `scripts/check-release-records.sh` refuses a tag without its
  CHANGELOG section and Capabilities record. A retro does not imply a tag: work
  from an untagged cluster ships in the next tag (v23–26, v78–100, v311,
  v350–401, v403 and v411 were never cut).

## Gates

| Gate tag | Version | What it proved |
|---|---|---|
| `maidan-2.0` | `v58.0.0` | The collaboration surface, end to end (`product_completion_gate_e2e`) |
| `maidan-agent-1.0` | `v76.0.0` | The agent substrate: MCP, subscriptions, context (`agent_substrate_gate_e2e`) |
| `maidan-operator-1.0` | `v101.0.0` | The operator surface: the web UI, health, metrics, OpenAPI (`maidan_operator_gate_e2e`) |
| `maidan-scale-1.0` | `v120.0.0` | Scale-out: several replicas, sharded fan-out, SLOs ([gate record](Gates/maidan-scale-1.0.md)) |

No further gate is defined. The next one would be the public launch, which is
the maintainer's call.

## Where it is

The latest release is `v412.0.0`, and `main` is ahead of it. The work in
flight and the ranked plan are the **Now** and **Next** sections of
[Open Work](Open%20Work.md).

## Horizons

0. **Safe to deploy.** Before anyone runs Maidan: every deploy path runs the
   newest release, no chart renders a default credential or the `dev` image,
   and a contract keeps the pins from drifting (both #1156). What remains: the
   pins name `v412.0.0`, from before the week's cross-tenant fixes, until the
   next tag, and the stack's bundled Postgres and MinIO images no longer exist
   (in flight). This is the **Before anyone deploys** section of Open Work, and
   it comes first.
1. **Agents coordinate at workspace scale.** A verdict reaches the worker as an
   event (#1132), a hung agent's time is charged against its budget (#1139),
   the SDKs return typed results and errors (#1129), and stateless MCP
   subscriptions work across replicas (#1131), and an agent can claim the next
   ready thread anywhere in its workspace (#1145). In flight: every way a claim
   ends charges its worked time, and a thread over budget is not handed out.
   Next: workspace-wide waits and queue depth.
2. **Nothing is silently lost, and nothing grows without bound.** Retries share
   a per-host budget (#1122), every audit row belongs to a workspace (#1134),
   both backends have a tested backup path (#1126), and fairness and retention
   are per workspace by default (#1153). In flight: a legal hold that keeps a
   workspace's deliveries, and read notifications that age out (#1165). Next:
   partitioning the append-only tables.
3. **Proof over tests.** The protocol decoders are fuzzed, the auth and bus
   tests are mutation-checked (#1125), the release workflow attests image SBOMs
   from the next tag on (#1119), and
   every HTTP operation is classified as reading or changing state (#1121).
   The nightly store mutation job tests mutants (none had before #1155), and
   `cargo vet` covers the root lockfile (#1155), and every nightly job fails red,
   the fuzz job on every target (#1160, whose first night found two real
   decoder bugs). In flight: the review comments left unanswered on merged PRs,
   one of them a cross-tenant rate-limit bug. Next: Kani proofs, and a named
   regression test per Threat-Model row.
4. **A web UI worth showing.** The board is the one thread surface (#1118),
   errors are inline (#1117), a blank page walks to a connected board (#1123),
   attachments show their names and images (#1135), tokens rotate from the page
   (#1127), Connect an agent finishes with a worker token (#1144), a refused
   close shows on the board (#1147), and the first screen leads with the board
   (#1151). In flight: the token leaving the browser's storage (#1142). Next:
   the rest of the QA pass, the ten changes of the [UI design
   contract](UI%20Design.md), write paths for a signed-in person, typed modules
   with a CSP, and screenshots captured by a script.
5. **Launch** (the maintainer's call): the public site, an in-browser
   playground, and paid self-hosted tiers before any hosted service, with the
   room itself staying open source.

## What Maidan will not become

- **A harness, sandbox or eval gauntlet.** Those live outside the room; Maidan
  is where their work is coordinated and recorded.
- **A workflow engine.** The outbox, the task DAG and the scheduler already
  give the semantics; Restate and Temporal were compared and declined.
- **A knowledge-graph product.** It ships the primitives (typed references, a
  glossary, the event lexicon), not an ontology reasoner.
- **A CRDT or a multi-region active-active store.** The hash-chained log is the
  product; one primary with read replicas is the scaling model.
- **A holder of unearned claims.** No SLA, badge, logo or benchmark appears on a
  launch surface without a dated method behind it.

Row-level security stays deferred (Decisions, Cluster 216): isolation is
enforced in the application and proven by `tenant_isolation_e2e`.
