### Usage rollup no longer overflows on large validated rows

- **Fixed:** The usage rollup computes each row's uncached and standard-lane charge without forming 64-bit tokens × rate products, so a validated report near the limit no longer fails the rollup with `bigint out of range` on Postgres or loses precision on SQLite (#1595).
