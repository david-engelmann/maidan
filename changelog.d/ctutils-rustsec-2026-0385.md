### ctutils 0.4.4 for RUSTSEC-2026-0385

- **Fixed:** `ctutils` is bumped from 0.4.2 to 0.4.4 for RUSTSEC-2026-0385, where some constant-time mask and `u128` comparisons could compile to branches. It comes in through `crypto-bigint`, and `cargo deny` fails every PR until it is bumped.
