### Fixed

- **Fixed:** Docs book sync refuses page/asset names with `..` or path separators before staging; coverage-floors rejects oversize/NUL/`..`/negative LCOV records; `check-build-bin-dir` scans every `scripts/*.sh` for runtime `./target/debug`; and the Go SDK caps `DecodeModelJSON` at 1 MiB with no trailing junk.
