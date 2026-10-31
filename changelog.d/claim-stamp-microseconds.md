### A claim's two timestamps are one microsecond

- **Fixed:** Claim, assign, and claim-next stamp `claimed_at` and `updated_at` from one `Utc::now()` truncated to microseconds on both backends. Postgres stores microseconds, so a nanosecond reading (or a second reading taken for the other column) made the unacknowledged-claim event's `claimed_at` disagree with the claim row.
