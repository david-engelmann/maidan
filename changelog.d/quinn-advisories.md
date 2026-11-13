### quinn advisories

- **Security:** `quinn-proto` moves to 0.11.19 and `quinn-udp` to 0.5.16 in the root and fuzz lockfiles, clearing RUSTSEC-2026-0523, RUSTSEC-2026-0524 and RUSTSEC-2026-0538. `quinn-proto` 0.11.19 brings `rand` 0.10, `rand_pcg` 0.10 and `chacha20` 0.10, which have `cargo vet` exemptions until an imported audit covers them (#2283).
