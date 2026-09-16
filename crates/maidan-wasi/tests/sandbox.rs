//! Cluster 399.1: the guest cannot leave, cannot outrun its fuel, and cannot
//! outgrow its memory.
//!
//! Modules are hand-written WAT so each test pins one property with nothing else
//! moving. A sandbox whose bounds are only asserted by "a real guest behaved" is
//! not tested — it is observed.

use maidan_types::{
    wasi::{WasiFailureKind, WasiInvoke, WasiLimits},
    ChannelId, MemberId, MessageId, ThreadId, WorkspaceId,
};

fn invoke() -> WasiInvoke {
    WasiInvoke::new(
        "demo",
        "some args",
        WorkspaceId(uuid::Uuid::from_u128(1)),
        ChannelId(uuid::Uuid::from_u128(2)),
        ThreadId(uuid::Uuid::from_u128(3)),
        MemberId(uuid::Uuid::from_u128(4)),
        MessageId(uuid::Uuid::from_u128(5)),
    )
}

fn wat(src: &str) -> Vec<u8> {
    wat::parse_str(src).expect("valid wat")
}

/// A module importing anything off the allowlist must not run at all.
#[test]
fn a_banned_import_is_refused_before_execution() {
    for (module, name) in [
        ("wasi_snapshot_preview1", "path_open"),
        ("wasi_snapshot_preview1", "sock_connect"),
        ("wasi_snapshot_preview1", "fd_readdir"),
        ("env", "host_escape"),
    ] {
        let m = wat(&format!(
            r#"(module (import "{module}" "{name}" (func $f)) (func (export "_start")))"#
        ));
        let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
        assert!(!out.ok, "{module}::{name} must be refused");
        assert_eq!(
            out.error_kind,
            Some(WasiFailureKind::BannedImport),
            "{module}::{name} should be a banned import, got {:?}",
            out.error_kind
        );
        let err = out.error.clone().unwrap_or_default();
        assert!(
            err.contains(name),
            "the error should name the offending import, got: {err}"
        );
    }
}

/// An infinite loop ends as fuel exhaustion, not a hang and not a generic trap.
#[test]
fn an_infinite_loop_exhausts_fuel() {
    let m = wat(r#"(module (func (export "_start") (loop $l (br $l))))"#);
    let out = maidan_wasi::run(
        &m,
        &invoke(),
        WasiLimits {
            fuel: 100_000,
            memory_bytes: 1 << 20,
        },
    );
    assert!(!out.ok);
    assert_eq!(
        out.error_kind,
        Some(WasiFailureKind::FuelExhausted),
        "a runaway loop must be classified as fuel, not as an opaque trap"
    );
}

/// Growing past the cap is a memory failure, distinguishable from a trap.
#[test]
fn growing_past_the_memory_cap_is_reported_as_such() {
    let m = wat(r#"(module
             (memory (export "memory") 1)
             (func (export "_start")
               (drop (memory.grow (i32.const 100)))
               (drop (i32.load (i32.const 6000000)))))"#);
    let out = maidan_wasi::run(
        &m,
        &invoke(),
        WasiLimits {
            fuel: 10_000_000,
            memory_bytes: 64 * 1024,
        },
    );
    assert!(!out.ok);
    assert_eq!(
        out.error_kind,
        Some(WasiFailureKind::MemoryLimit),
        "the cap must be reported as a memory limit, got {:?}: {:?}",
        out.error_kind,
        out.error
    );
}

/// An explicit trap is a trap — not silently a success.
#[test]
fn an_unreachable_is_a_trap() {
    let m = wat(r#"(module (func (export "_start") unreachable))"#);
    let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
    assert!(!out.ok);
    assert_eq!(out.error_kind, Some(WasiFailureKind::Trap));
}

/// Bytes that are not a module fail as an invalid module, not a panic.
#[test]
fn garbage_is_an_invalid_module() {
    let out = maidan_wasi::run(b"definitely not wasm", &invoke(), WasiLimits::default());
    assert!(!out.ok);
    assert_eq!(out.error_kind, Some(WasiFailureKind::InvalidModule));
}

/// The happy path: a guest writes to stdout and returns cleanly.
#[test]
fn a_guest_can_write_stdout_and_finish() {
    let m = wat(r#"(module
             (import "wasi_snapshot_preview1" "fd_write"
               (func $fd_write (param i32 i32 i32 i32) (result i32)))
             (memory (export "memory") 1)
             (data (i32.const 8) "hello")
             (func (export "_start")
               (i32.store (i32.const 0) (i32.const 8))
               (i32.store (i32.const 4) (i32.const 5))
               (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))))"#);
    let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
    assert!(out.ok, "expected success, got {:?}", out.error);
    assert_eq!(out.stdout, "hello");
    assert!(out.stderr.is_empty());
}

/// `proc_exit(0)` unwinds as a trap but is a clean finish, and output written
/// before it survives.
#[test]
fn proc_exit_zero_is_success_and_keeps_prior_output() {
    let m = wat(r#"(module
             (import "wasi_snapshot_preview1" "fd_write"
               (func $fd_write (param i32 i32 i32 i32) (result i32)))
             (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
             (memory (export "memory") 1)
             (data (i32.const 8) "done")
             (func (export "_start")
               (i32.store (i32.const 0) (i32.const 8))
               (i32.store (i32.const 4) (i32.const 4))
               (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 20)))
               (call $exit (i32.const 0))))"#);
    let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
    assert!(
        out.ok,
        "proc_exit(0) is a clean finish, got {:?}",
        out.error
    );
    assert_eq!(out.stdout, "done");
}

/// A non-zero exit is a failure that still reports what the guest managed to say.
#[test]
fn a_nonzero_exit_fails_but_keeps_output() {
    let m = wat(r#"(module
             (import "wasi_snapshot_preview1" "proc_exit" (func $exit (param i32)))
             (func (export "_start") (call $exit (i32.const 3))))"#);
    let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
    assert!(!out.ok);
    assert!(
        out.error.clone().unwrap_or_default().contains('3'),
        "the exit status should be reported, got {:?}",
        out.error
    );
}

/// The guest reads its invoke envelope from stdin, and it round-trips.
#[test]
fn the_invoke_envelope_is_readable_on_stdin() {
    let m = wat(r#"(module
             (import "wasi_snapshot_preview1" "fd_read"
               (func $fd_read (param i32 i32 i32 i32) (result i32)))
             (import "wasi_snapshot_preview1" "fd_write"
               (func $fd_write (param i32 i32 i32 i32) (result i32)))
             (memory (export "memory") 2)
             (func (export "_start")
               (i32.store (i32.const 0) (i32.const 100))
               (i32.store (i32.const 4) (i32.const 4096))
               (drop (call $fd_read (i32.const 0) (i32.const 0) (i32.const 1) (i32.const 8)))
               (i32.store (i32.const 12) (i32.const 100))
               (i32.store (i32.const 16) (i32.load (i32.const 8)))
               (drop (call $fd_write (i32.const 1) (i32.const 12) (i32.const 1) (i32.const 20)))))"#);
    let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
    assert!(out.ok, "expected success, got {:?}", out.error);
    let echoed: serde_json::Value =
        serde_json::from_str(&out.stdout).expect("stdin should have carried the invoke JSON");
    assert_eq!(echoed["command"], "demo");
    assert_eq!(echoed["args"], "some args");
    assert_eq!(echoed["$type"], maidan_types::wasi::WASI_INVOKE_TYPE);
}

/// Every name on the allowlist actually links.
///
/// The allowlist and the host implementations are two lists that have to agree,
/// and nothing else makes them. A name allowlisted but never registered passes
/// `check_imports` and then dies in the linker — reported as an
/// `invalid_module`, which reads as "your wasm is broken" to the one person who
/// did nothing wrong. This is the test that fails instead.
#[test]
fn every_allowlisted_import_resolves() {
    let sigs: &[(&str, &str)] = &[
        ("args_get", "(param i32 i32) (result i32)"),
        ("args_sizes_get", "(param i32 i32) (result i32)"),
        ("clock_res_get", "(param i32 i32) (result i32)"),
        ("clock_time_get", "(param i32 i64 i32) (result i32)"),
        ("environ_get", "(param i32 i32) (result i32)"),
        ("environ_sizes_get", "(param i32 i32) (result i32)"),
        ("fd_close", "(param i32) (result i32)"),
        ("fd_fdstat_get", "(param i32 i32) (result i32)"),
        ("fd_prestat_dir_name", "(param i32 i32 i32) (result i32)"),
        ("fd_prestat_get", "(param i32 i32) (result i32)"),
        ("fd_read", "(param i32 i32 i32 i32) (result i32)"),
        ("fd_seek", "(param i32 i64 i32 i32) (result i32)"),
        ("fd_write", "(param i32 i32 i32 i32) (result i32)"),
        ("proc_exit", "(param i32)"),
        ("random_get", "(param i32 i32) (result i32)"),
        ("sched_yield", "(result i32)"),
    ];
    assert_eq!(
        sigs.len(),
        maidan_types::wasi::WASI_ALLOWED_PREVIEW1.len(),
        "a name was added to the allowlist without a signature here"
    );
    for (name, _) in sigs {
        assert!(
            maidan_types::wasi::WASI_ALLOWED_PREVIEW1.contains(name),
            "{name} is not on the allowlist"
        );
    }

    let imports: String = sigs
        .iter()
        .map(|(n, sig)| format!(r#"(import "wasi_snapshot_preview1" "{n}" (func ${n} {sig}))"#))
        .collect::<Vec<_>>()
        .join("\n");
    let m = wat(&format!(
        r#"(module {imports} (memory (export "memory") 1) (func (export "_start")))"#
    ));

    let out = maidan_wasi::run(&m, &invoke(), WasiLimits::default());
    assert!(
        out.ok,
        "a module importing the whole allowlist must link and run, got {:?} / {:?}",
        out.error_kind, out.error
    );
}
