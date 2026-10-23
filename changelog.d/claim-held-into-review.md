### A claim held into review lapses like any other

- **Fixed:** the claim reaper only freed lapsed leases on `open` threads, so a claimant that published its result, moved the thread to `in_review` and then died, or failed to release, held its slot for good. Lapsed leases are now reaped on every live thread that is not `closed` or `archived`, with the same `ClaimExpired`. Releasing after the result is published works and stays fenced to the current lease, and a regression test pins both.
