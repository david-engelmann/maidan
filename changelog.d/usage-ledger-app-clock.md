### Usage ledger clock

- **Fixed:** On Postgres, a usage report's `accepted_at` now comes from the app clock instead of the database's `now()`. Claim-next's warm-prefix window compares it against an app-computed cutoff, so a drifting database clock no longer shifts that window (#1586).
