# Migrations

What a schema change must do when more than one server version runs against
one database. This page does not change the runner. It says which changes are
safe inside the window the runner already creates.

## When this applies

Every replica applies pending migrations on boot. A surge (`maxUnavailable: 0`)
starts the new binary, which migrates, while the previous binary is still
serving. Until those previous replicas stop, both versions read and write the
schema the new binary just applied. `/health/ready` stays false until that
replica has finished migrating, so a load balancer that honors readiness does
not send traffic to a replica that is mid-migration. It does not stop the
previous replicas, and the advisory lock does not teach the previous binary
the new schema.

That overlap is the case this page is for. A single binary, which is how
Maidan is developed, is not. Stopping the previous binary before the new one
boots is not either: that is a cutover, and a cutover may drop and rewrite.

There is no HTTP or MCP compatibility promise before the product's own 1.0
gate (Decisions, F-54). This page does not add one. It is only the rule for
the shared schema during the overlap.

## How a migration is applied

The rules below are written against `crates/maidan-store/src/migrate.rs`.
A change to the runner and a change to this page land together. The contract
is `migration_expand_contract`.

- The server is the only applier. There is no migration Job. Postgres runs
  `run_postgres_migrations`; SQLite runs `run_sqlite_migrations`.
- Postgres takes a session advisory lock, `pg_advisory_lock`, for the whole
  apply. The key is `0x6D69_6772`. That session sets `statement_timeout` and
  `lock_timeout` to 0, so the wait and the DDL are not cancelled. The next
  replica blocks, then sees the versions already recorded and does nothing.
- SQLite has one writer and no advisory lock. Two versions against one SQLite
  file is not a supported deploy. The same change still ships for both
  dialects, because a fresh database of either kind must match.
- Each version is one transaction: the SQL file, then a row in
  `maidan_migrations`, then commit. A failure rolls that version back.
  Versions that already committed stay applied. The runner does not put every
  version in one transaction, and it does not run a file outside a
  transaction. A statement Postgres refuses inside a transaction, including
  `CREATE INDEX CONCURRENTLY` and `VACUUM`, cannot go in a migration file.
- A version already in `maidan_migrations` is skipped. The file is not read
  again. Editing an applied file changes nothing on a database that already
  has that version, and it changes what a fresh database builds. Do not edit
  an applied file. The next change is the next version.
- A `*_down.sql` file is not applied. The register test skips `_down.sql`.
  Only `0001_core_down.sql` exists, and it is manual. A rollout does not run
  down migrations.
- The register is the `include_str!` list in `migrate.rs`, not the directory.
  A file that is not registered does not run. Both dialects get the same
  version number in the same change. `migrations_stay_in_lockstep` fails a
  slug that exists on only one side, apart from the two exceptions named in
  that test. Version numbers 125 and 133 are unused. Leave them unused.

## Expand

The migration that runs while the previous binary is still serving may only
add:

- a new table the previous binary does not read or write
- a column that is nullable, or that has a default, so an insert from the
  previous binary still succeeds
- an index
- a constraint that every existing row already satisfies, and that the
  previous binary's writes still satisfy

Reads in the store name their columns (`row.get("budget")` and the explicit
list in `token_quotas::list`). An added column does not move those names. Do
not change the column list of a statement the previous binary already runs.
A new column that the previous binary must keep seeing, because the new
binary's writes would otherwise be invisible to it, is written by the new
binary as well as the new shape. That second write lasts until the contract.
It does not stay.

The expand does not drop, rename, or retype a column or table. It does not
tighten a constraint the previous binary can still violate. It does not
rewrite a row into a shape the previous binary rejects or misreads.

`0135_member_handle_ci` rewrites handles, and `0136_artifact_erase` drops
`tombstoned_at`. Both shipped as cutovers, with one version running. They are
not the pattern for a surge.

## Contract

The contract runs only after every replica is on the version that understands
the new shape. That is a later release, not the same migration and not the
same rollout.

The contract may drop the old column, drop the old table, rename, tighten a
constraint, or rewrite rows. The binary that is running no longer reads or
writes the old shape. If any replica of the previous version is still up, the
contract waits.

A compatibility shim does not stay past the contract. No view, trigger, or
second write path whose only job is to keep an older version alive after that
release.

## A cutover

Before a fleet runs two versions, a migration may still drop and rewrite in
one step. The pull request says so, and the deploy stops the previous binary
before the new one migrates. Do not call that migration expand/contract.

## Checklist

A pull request that changes the schema while two versions will overlap answers
these in the body:

- which release is the expand, and which later release is the contract
- which statements the previous binary still runs against the changed tables
- that both dialects and the same version number are in the register
- that the SQL file is new, and no applied file was edited
- that the file runs as one transaction
