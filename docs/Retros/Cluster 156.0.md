# Cluster 156.0 retro — production-safety defaults

> Tag **`v156.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Enterprise-hardening arc (arc 1 of the post-v155 program), part 1.

## What shipped

- **SIGTERM graceful shutdown** — the shutdown future now `select!`s on both
  `SIGINT` (`ctrl_c`) and `SIGTERM` on unix, falling back to SIGINT-only if the
  handler can't install; non-unix unchanged. Existing worker `shutdown()`
  sequence runs on either signal.
- **Default `statement_timeout` = 30 s** (`MAIDAN_DB_STATEMENT_TIMEOUT_MS`
  `0` → `30000`). Set `0` to disable; migrations are exempt.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Cluster 157 | Auth fail-closed | `AUTH_DISABLED=1` runs with `MAIDAN_ENV` unset across compose ×5, scale-out-smoke.sh, helm values-ci — a coordinated change that must move in lockstep or the required smoke jobs break. |
| Later arc-1 | Default-on rate limits / body-size cap | Forcing rate limits on risks the scale-out burst smoke; a blind body cap can break artifact multipart upload — needs per-route care. |

## Surprises

- **The "quick wins" weren't all quick.** Two of the four hardening items I'd
  slated as trivial (fail-closed auth, default rate limits) turned out to be
  coordinated changes coupled to the required CI smoke jobs — see the scoping
  note in the roadmap. SIGTERM + statement_timeout are the genuinely
  self-contained pair, so they shipped first.

## Decisions

- **30 s statement_timeout default** (not opt-in): a healthy query never
  approaches it, migrations reset it to 0, and the operator reindex / CLI have
  documented escape hatches — so default-on is safe and closes the DoS vector by
  default rather than only for operators who happen to set the env.
- **Fall back to SIGINT-only** if the SIGTERM handler fails to install, rather
  than aborting startup — availability over strictness for a best-effort signal.

## Capability table extension

| Capability | Where |
|------------|-------|
| SIGTERM drain | `main.rs` |
| Default statement-timeout cap | `config.rs` |

## Risks identified + still open

- **Low.** Both are configurable; the timeout is generous and migration-exempt;
  SIGTERM only adds a drain path. No test or smoke behavior changed.

## Forward look

Arc 1 continues: **157** auth fail-closed (the coordinated manifest/CI-env
change), **158** container-image signing + vuln scan (`release.yml`), then the
flagship **channel/thread RBAC** — the #1 finding (authz is workspace-flat
today). Then arcs 2 (perf + CI/CD), 3 (agentic features), 4 (token round 3).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
