# Cluster 260.0 — backup / restore + disaster-recovery runbook

> **Program D (scale & durability), part 3.** Phase XXIV post-gate hardening.
> Tag **`v260.0.0`**. No new gate tag.

## Goal

Give an operator a tested path to back up and restore a Maidan deployment — the
"production ready for major companies" table stakes that was missing (only delete
paths existed, no capture/recover). Two scripts + a runbook that says exactly what
is and isn't covered.

## Scope

| Change | Where |
|--------|-------|
| `backup.sh` — `pg_dump -Fc` + tar of the localfs artifact root + a manifest | `scripts/backup.sh` |
| `restore.sh` — `pg_restore` (+ untar), refuses a non-empty target without `--force` | `scripts/restore.sh` |
| "Backup & disaster recovery" runbook: coverage, out-of-band secrets, RPO/RTO, recovery steps | `docs/Production.md` |

## Design decisions

- **Two stores, two mechanisms.** Postgres is the system of record → `pg_dump`
  custom format (`-Fc`, restore-friendly). Artifacts are content-addressed immutable
  blobs → a `tar` of the `localfs` root, or (for `s3`) the bucket *is* the durable
  copy and the runbook says to enable versioning/replication there rather than
  copying blobs into the dump.
- **Secrets are explicitly out of the data backup.** `DATABASE_URL`,
  `MAIDAN_SESSION_SECRET`, the `FEDERATION_ENCRYPTION_KEY` keyring, and SMTP/OIDC
  creds are restored from the secret manager, not the dump. The runbook calls this
  out so a restore isn't silently missing signing continuity.
- **Restore is guarded.** `restore.sh` refuses a non-empty target database unless
  `--force`, so it can't clobber a live deployment by accident; `--force` uses
  `pg_restore --clean --if-exists`.
- **Operator tools, like `loadgen` / `chaos`.** These are shell scripts an operator
  runs, not CI-gated code. Syntax-checked (`bash -n`); the runbook documents the
  verified recovery sequence (provision → secrets → restore → `/health/ready` →
  scale).

## Non-goals / deferred

- Automated scheduled backups (cron/k8s CronJob wiring) — the script is the
  primitive; scheduling is deployment-specific.
- A CI round-trip test of `backup.sh`→`restore.sh` (needs `pg_dump`/`pg_restore` in
  the image) — deferred; the scripts are `bash -n`-clean and the runbook is the
  contract.
- Read-replica routing — the last Program D item.

## Risks

- Docs-and-scripts only, no Rust change. `docs/Production.md` is a published page —
  verified with a local `mdbook build` (linkcheck) to avoid the bracket trap.
