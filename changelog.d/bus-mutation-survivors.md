### Bus mutation survivors

- **Fixed:** New tests catch the mutants the nightly bus 1/4 shard missed: the `MAIDAN_BUS_BROADCAST_CAP` positive-only filter, the `ListenerHealth` error count and last-success stamp, and the Postgres log head that seeds the LISTEN high-water mark. The drain mutant that could only ever time out is now listed in `.cargo/mutants.toml`, with the reason.
