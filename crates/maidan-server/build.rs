//! Builds the public landing (web/landing) into static/landing/dist so the
//! binary can embed it. Docker sets MAIDAN_SKIP_LANDING_BUILD after its own
//! Node stage has already produced that directory.
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // The runtime version string comes from `MAIDAN_VERSION` at compile time
    // (see `version()` in src/lib.rs), set to the release tag by the release
    // pipeline. `option_env!` is not tracked by cargo's fingerprint, so without
    // this line a warm build cache would bake a stale version into a new release
    // whose source is otherwise unchanged (our releases are often docs-only).
    println!("cargo:rerun-if-env-changed=MAIDAN_VERSION");
    println!("cargo:rerun-if-env-changed=MAIDAN_SKIP_LANDING_BUILD");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let web = manifest.join("../../web/landing");
    let dist = manifest.join("static/landing/dist");
    println!("cargo:rerun-if-changed={}", web.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        web.join("index.html").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("vite.config.ts").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        web.join("package-lock.json").display()
    );

    if std::env::var_os("MAIDAN_SKIP_LANDING_BUILD").is_some() {
        if !dist.join("app.js").is_file() {
            panic!(
                "MAIDAN_SKIP_LANDING_BUILD is set but {} is missing",
                dist.join("app.js").display()
            );
        }
        return;
    }

    let stamp = dist.join(".src-hash");
    let hash = source_hash(&web);
    if stamp.is_file() && std::fs::read_to_string(&stamp).ok().as_deref() == Some(hash.as_str()) {
        return;
    }

    let npm = if cfg!(windows) { "npm.cmd" } else { "npm" };
    if !web.join("node_modules").is_dir() {
        let status = Command::new(npm)
            .arg("ci")
            .current_dir(&web)
            .status()
            .unwrap_or_else(|err| panic!("could not run npm ci in web/landing: {err}"));
        if !status.success() {
            panic!("npm ci failed in web/landing");
        }
    }
    let status = Command::new(npm)
        .args(["run", "build"])
        .current_dir(&web)
        .status()
        .unwrap_or_else(|err| panic!("could not run npm run build in web/landing: {err}"));
    if !status.success() {
        panic!("the landing build failed");
    }
    std::fs::write(&stamp, hash).expect("write landing source hash");
}

fn source_hash(web: &Path) -> String {
    let mut acc = 0xcbf29ce484222325u64;
    hash_path(&web.join("src"), &mut acc);
    for name in [
        "index.html",
        "vite.config.ts",
        "package-lock.json",
        "package.json",
    ] {
        hash_path(&web.join(name), &mut acc);
    }
    format!("{acc:016x}")
}

fn hash_path(path: &Path, acc: &mut u64) {
    let Ok(meta) = path.metadata() else {
        return;
    };
    if meta.is_dir() {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        let mut names: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        names.sort();
        for name in names {
            hash_path(&name, acc);
        }
        return;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    *acc = acc
        .wrapping_mul(0x100000001b3)
        .wrapping_add(bytes.len() as u64);
    for byte in bytes {
        *acc = acc.wrapping_mul(0x100000001b3).wrapping_add(byte as u64);
    }
}
