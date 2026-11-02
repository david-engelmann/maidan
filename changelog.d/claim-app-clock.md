### Claim worked time uses one clock

- **Fixed:** on Postgres, acknowledging a claim and charging its worked time now read the app clock, the same clock that sets claim deadlines, instead of the database's `NOW()`. A database clock that drifts no longer skews worked time or a lapsed claim's charge (#1728).
