# Cluster 416 retro — Wave 4 #44: UUIDv7 entity ids

> Post-gate hardening · source record (no `v416.0.0` tag yet; the tag is the maintainer's) · PR #1030 + close record

## Outcome

Every id Maidan mints for a row, job, task or request is UUIDv7, so ids sort
by creation time and primary-key inserts stay append-mostly. The values that
are credentials stay random.

| Slice | PR | Result |
|-------|----|--------|
| 416.1 | #1030 | 70 production `Uuid::new_v4()` calls moved to `Uuid::now_v7()`; 8 stay v4; `uuid_v7_contract` and `uuid_v7_ids`. |

## Decisions

- **A credential stays v4.** Token and share-ticket secrets, the OAuth code,
  the MCP session id and the browser session id are values someone presents
  to prove who they are, so they must not be predictable. The browser session
  id is also HMAC-signed, which already prevents forgery; it stays v4 anyway,
  as defense in depth.
- **Ids are minted app-side.** No migration mints ids in SQL, so Postgres 16's
  lack of `uuidv7()` never matters.
- **The allowlist has exact counts, with a reason each.** Adding a v4 call to an
  allowlisted file still has to justify itself, and a stale entry fails too.
- **Existing rows keep their ids.** Nothing is rewritten; only new rows are v7.

## What surprised us

- **The Open Work estimate of about 78 was exact**, which made the audit a
  check rather than a hunt.
- **The first scan found nothing**, because a git pathspec glob did not recurse
  into `src/`. The contract now asserts that it found some v4 calls at all.

## Carried forward

- None.
