fn main() {
    // `option_env!` is not part of Cargo's normal source fingerprint. Make a
    // warm release cache rebuild the CLI whenever the injected tag changes.
    println!("cargo:rerun-if-env-changed=MAIDAN_VERSION");
}
