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
[Open Work](Open%20Work.md). `scripts/sdk-test.sh` and `scripts/lease-demo.sh`
run the binary from `CARGO_TARGET_DIR` when it is set.

## Horizons

0. **Safe to deploy.** Before anyone runs Maidan: every deploy path runs the
   newest release, no chart renders a default credential or the `dev` image,
   and a contract keeps the pins from drifting (both #1156). What remains: the
   pins name `v412.0.0`, from before the week's cross-tenant fixes, until the
   next tag, and the stack's bundled Postgres and MinIO images no longer exist
   (in flight). This is the **Before anyone deploys** section of Open Work, and
   it comes first.
1. **A Slack message becomes a reviewed PR.** David's agent loop, decided on
   2026-10-03: a `!change` in Slack opens a Maidan thread, Pi does the coding
   without holding any GitHub credential, Maidan commits Pi's diff to the
   named branch at the base commit Pi reports and opens a draft PR, and
   Soundcheck previews it and marks it ready. Replies stay on the surface the
   work started on: a Slack thread is answered in Slack, a PR comment on the
   PR. Maidan's `github_branch` delivery, the threaded Slack reply and the
   audited allowlist seed, on David's personal PAT, are on `main` (#1245).
   One instance built from `main` in Pi's dev-tools stack is documented, with
   secrets read from files (#1247) and app re-install (#1248). The contract is beatgig/soundcheck's
   `docs/cross-repo/change-flow.md`.
2. **Agents coordinate at workspace scale.** A verdict reaches the worker as an
   event (#1132), a hung agent's time is charged against its budget (#1139),
   the SDKs return typed results and errors (#1129), and stateless MCP
   subscriptions work across replicas (#1131), and an agent can claim the next
   ready thread anywhere in its workspace (#1145). In flight: every way a claim
   ends charges its worked time, and a thread over budget is not handed out.
   Next: workspace-wide waits and queue depth.
3. **Nothing is silently lost, and nothing grows without bound.** Retries share
   a per-host budget (#1122), every audit row belongs to a workspace (#1134),
   both backends have a tested backup path (#1126), and fairness and retention
   are per workspace by default (#1153). A legal hold keeps that workspace's
   deliveries, and read notifications age out (#1165). When
   `MAIDAN_RETENTION_MESSAGES_DAYS` is set, it is the instance ceiling: a
   longer workspace `messages_days` is refused, and the sweep erases messages
   past the cutoff in every workspace that is not held (#1232). Next:
   partitioning the append-only tables.
   A schema change while two versions share one database expands, then contracts in a later release ([Migrations](Migrations.md)).
4. **Proof over tests.** The protocol decoders are fuzzed, the auth and bus
   tests are mutation-checked (#1125), the release workflow attests image SBOMs
   from the next tag on (#1119), and
   every HTTP operation is classified as reading or changing state (#1121).
   The nightly store mutation job tests mutants (none had before #1155), and
   `cargo vet` covers the root lockfile (#1155), and every nightly job fails red,
   the fuzz job on every target (#1160, whose first night found two real
   decoder bugs). In flight: the review comments left unanswered on merged PRs,
   one of them a cross-tenant rate-limit bug. Next: Kani proofs, and a named
   regression test per Threat-Model row. REST `POST /threads/{id}` names
   `action` as `start_review`, `close`, or `archive`.
5. **A web UI worth showing.** The board is the one thread surface (#1118),
   errors are inline (#1117), a blank page walks to a connected board (#1123),
   attachments show their names and images (#1135), tokens rotate from the page
   (#1127), Connect an agent finishes with a worker token (#1144), a refused
   close shows on the board (#1147), and the first screen leads with the board
   (#1151). A pasted token becomes an HttpOnly session (#1142). The signed-in
   line shows a display name, and a workspace can be named with
   `PATCH /workspaces/{id}` (#1203). A signed-in person can edit, upload, paste,
   and start a review or close a task (#1176). The board is ES modules in
   `static/ui`, served from the binary (#1207). `tsc --noEmit --checkJs` can
   check the JSDoc, and it does not run in CI. `/ui` sends a Content-Security-Policy
   (#1213). Keyboard focus reveals the pin toggle (#1217). Inputs, selects, and
   textareas are 16px (#1219). Playwright is still not a required
   check. The specs fail when the board is wrong and pass when the lanes, the
   state word, the empty sentence, and the human refusal are right. Specs for
   prefs, slash commands, delivery replay, token mint and revoke, and DMs
   landed (#1205). A group-DM spec opens a conversation and posts in it (#1230).
   `helpers.test.mjs` does not run in CI. The ten surfaces of the [UI design
   contract](UI%20Design.md) are on main (#1179, #1181, #1182, #1185, #1187,
   #1188, #1190, #1192, #1193, #1196). A rotated token is installed only into
   the connection that requested it (#1229). Opening a group DM asks for three
   members and selects that conversation (#1230). Next, from the UI deep
   dive of 2026-10-03: the human supervising agents is the user, and silence
   is a state. Needs-you says when it could not load (#1251). Next, sign-in
   is honest, review and blocked work always reach a person, agents declare
   their own status, and an approval binds the evidence it showed (Open Work
   Next 2 to 12). Then the review findings and screenshots captured by a script.
6. **Agents pay for what changed.** Maidan's context is byte-stable and
   layered, its MCP surface follows the 2026-07-28 caching rules with small,
   stable tool profiles, and its ledger prices every cache tier and reports
   cost per completed task. Then it coordinates for the cache: warm then fan
   out, claims inside the cache TTL, a batch lane for work with slack, and no
   duplicate runs. The claim is measured by a pre-registered benchmark, not a
   hit rate. This is Program C ([Context Economics](Context%20Economics.md)).
   MCP 2026-07-28 conformance and the worker and reviewer tool profiles are
   on `main` (#1239, #1240). The canonical pack is on `main` (#1241); the
   ledger prices every cache tier and rolls up cost per completed task
   (#1242). Next: workspace queue depth and the SDK normalizers (Open
   Work Next 22 and 23).
   The research behind it is kept in the [archive](archive/Context%20Economics%20research%202026-10/README.md).
7. **Launch** (the maintainer's call): the public site, an in-browser
   playground, and paid self-hosted tiers before any hosted service, with the
   room itself staying open source. An operator can open a
   second workspace with `POST /operator/workspaces`, without `MAIDAN_BOOTSTRAP` (#1208).
   Signup and a hosted console are not started.
   The connected-apps program is the discovery half. The maintainer chose its fast track on 2026-10-03, lanes 1 to 7 with no authorization server (a Gemini CLI extension, a Copilot CLI plugin, the MCP registries and catalogs, a listing asset pack with a demo instance for reviewers, connect recipes, Muse behind the Dawn outcome, and Cursor). Nothing is submitted until the maintainer says go, and every submission is validated first. Self-hosting stays the product, the authorization server and the ChatGPT and full-OAuth Claude listings are parked, and no enterprise track starts until the consumer lanes measure (Open Work Next 14 to Next 21 and Later CA).
   The connected-app dev and test program comes first among the listings work. ChatGPT developer mode and claude.ai accept a server with no authentication, so Maidan is tested inside each client from a dev instance before anything is listed, with approvals kept in the console (Open Work Next 18).

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
