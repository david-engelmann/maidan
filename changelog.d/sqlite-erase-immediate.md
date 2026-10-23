### Fixed

- **Fixed:** on SQLite, erasing an artifact (`erase_artifact_audited`) or expiring a media ref now begins its transaction with `BEGIN IMMEDIATE`. Both read the legal-hold state before they write; begun deferred, a commit by another connection between that read and the write failed the erase with "database is locked" at once instead of waiting out `busy_timeout`. This was the intermittent `blob_reap` failure in `crypto_shredding`; a new test runs 40 erases alongside 200 other writes.
