### Claims stamp one clock

- **Fixed:** on Postgres, `claim_next` set `claimed_at` from the server clock and `updated_at` from the database clock, so the two drifted by milliseconds and `report_unacknowledged_claims_postgres` failed intermittently (#1603). Both now come from the same reading.
