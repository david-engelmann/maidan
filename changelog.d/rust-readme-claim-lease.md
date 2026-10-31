### Rust SDK docs

- **Fixed:** The README and crate-level example no longer pass `unwrap_or_default()` for the claim lease, which sent an empty `claim_lease_id` and got a 400. They now renew and write the result only when the claim returned a lease (#1763).
