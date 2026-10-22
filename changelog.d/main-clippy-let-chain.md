### main lints clean under edition 2024 again

- **Fixed:** the mail worker's suspension check is a let-chain, so clippy's collapsible_if passes on main (#1516 merged after edition 2024 with a nested if).
