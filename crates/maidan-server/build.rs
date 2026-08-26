fn main() {
    // The runtime version string comes from `MAIDAN_VERSION` at compile time
    // (see `version()` in src/lib.rs), set to the release tag by the release
    // pipeline. `option_env!` is not tracked by cargo's fingerprint, so without
    // this line a warm build cache would bake a stale version into a new release
    // whose source is otherwise unchanged (our releases are often docs-only).
    // Declaring the dependency forces a recompile whenever the tag changes.
    println!("cargo:rerun-if-env-changed=MAIDAN_VERSION");
}
