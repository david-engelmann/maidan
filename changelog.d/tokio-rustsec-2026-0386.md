### tokio 1.54.1 for RUSTSEC-2026-0386

- **Fixed:** tokio moves from 1.52.3 to 1.54.1 (and 1.53.1 to 1.54.1 in `fuzz/`) for RUSTSEC-2026-0386, `AsyncFd`'s constructors violating I/O safety. Maidan does not use `AsyncFd`; the advisory failed `cargo deny` for every pull request.
