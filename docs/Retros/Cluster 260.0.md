# Cluster 260.0 retro — a way back

> Tag **`v260.0.0`**. Phase XXIV (post-gate hardening). **Program D (scale &
> durability), part 3.** No new gate tag.

## What shipped

- `scripts/backup.sh` (`pg_dump -Fc` + a tar of the localfs artifact root + a
  manifest) and `scripts/restore.sh` (`pg_restore`, refusing a non-empty target
  without `--force`), plus a **Backup & disaster recovery** runbook in
  `docs/Production.md` — coverage, the secrets that live out of band, RPO/RTO
  guidance, and the step-by-step recovery sequence.

## Surprises / decisions

- **The interesting content is the exclusion list, not the `pg_dump` line.** Anyone
  can dump a database; the operator value is knowing what a restore *won't* bring
  back on its own — `MAIDAN_SESSION_SECRET` (or subscribe-resume tokens stop
  validating), the `FEDERATION_ENCRYPTION_KEY` keyring (or at-rest peer/webhook
  secrets can't be decrypted), and the S3 bucket (which is its own durable store,
  not something to copy into the dump). The runbook leads with that.
- **Content-addressing makes artifact backup forgiving.** Blobs are immutable and
  deduped, so a message referencing a blob that predates the artifact archive stays
  consistent after restore; the only thing a stale artifact backup can miss is a blob
  written *after* it. That property is worth stating — it changes how carefully the
  artifact archive has to be synchronized with the DB dump (answer: not very).
- **Guard the destructive direction.** `backup.sh` is harmless; `restore.sh` can
  destroy a live database, so it refuses a non-empty target unless `--force` — the
  same "don't clobber by accident" instinct as an admin-merge confirmation.
- **Operator tool, not CI code.** Like `loadgen` and `chaos`, these are `bash -n`-
  clean shell scripts run by a human; a CI round-trip test would need
  `pg_dump`/`pg_restore` in the image and is deferred. The runbook is the contract.
- **Published-doc discipline.** `Production.md` is in the mdBook, so I ran a local
  `mdbook build` before shipping — no bare brackets this time.

## Capability table extension

| Change | Where |
|--------|-------|
| `backup.sh` / `restore.sh` + DR runbook | `scripts/*`, `docs/Production.md` |

## Risks identified + still open

- No automated scheduling or CI round-trip test (both deferred, noted).

## Forward look

One Program D item remains: **read-replica routing** — the single-pool `Store` →
reader/writer refactor with read-after-write handling, which needs a real replica to
validate. The largest and riskiest of the set.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 259.0]].
