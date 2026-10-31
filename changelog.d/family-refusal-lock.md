### A refused same-family claim no longer holds the thread

- **Fixed:** on Postgres, a claim or assign that second-family routing refused had already taken the thread's row lock, and kept it until sqlx rolled the dropped transaction back on the connection's next use. A claim-next running in that window skipped the thread (`SKIP LOCKED`) and found nothing, so a seat of another family could miss a review it was allowed to take. The refusal is now checked before the lock; the lock and a second read still serialize a claim against a racing family label.
