### The root lockfile takes the fixed block-buffer

- **Security:** the root lockfile moves `block-buffer` from 0.12.0 to 0.12.1, the fixed release for GHSA-qwgh-2vcv-g2f7 (a caught panic in a hasher's compress callback could corrupt the buffer position). The fuzz and Rust SDK lockfiles keep 0.10.4, which has no fixed release; the `osv scan` job accepts it there, with the reason in `.config/osv-scanner.toml`.
