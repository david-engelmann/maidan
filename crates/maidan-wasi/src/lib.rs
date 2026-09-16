//! Sandboxed WASI host for Maidan slash-command handlers (Wave 3 #36).
//!
//! A workspace installs a **no-network guest** as a slash handler. The guest *is
//! the tool* — it reads a [`WasiInvoke`] as JSON on stdin, writes JSON on
//! stdout, and exits. It is not an agent runtime.
//!
//! # Why an interpreter
//!
//! `wasmi`, not `wasmtime` — see the ADR in `docs/Decisions.md`. Two reasons
//! that matter here: an interpreter has no code generator, so the entire
//! miscompilation-to-sandbox-escape class does not exist; and wasmi's fuel
//! metering is stable across versions, so [`WasiLimits::fuel`] keeps meaning the
//! same thing after a dependency bump.
//!
//! # Why the allowlist is structural
//!
//! [`WASI_ALLOWED_PREVIEW1`] names 17 preview-1 calls — stdin/stdout, args, env,
//! clocks, random — and nothing that touches a path or a socket. This module
//! *implements those and only those*. A guest that imports `path_open` or
//! `sock_connect` fails to **link**; there is no filter to bypass and no
//! filesystem surface to escape from.
//!
//! # What bounds a run
//!
//! Fuel (wasmi's own metering), a linear-memory cap enforced by a
//! [`wasmi::ResourceLimiter`], and an output cap so a guest cannot exhaust host
//! memory through stdout. Wall-clock is the caller's job — `dispatch_slash_command`
//! already applies its own timeout.

use std::sync::{Arc, Mutex};

use maidan_types::wasi::{
    WasiFailureKind, WasiInvoke, WasiLimits, WasiResult, WASI_ALLOWED_PREVIEW1,
    WASI_PREVIEW1_MODULE,
};

/// Largest stdout+stderr we will retain from a guest. A handler returns a slash
/// response; anything larger is a runaway, not an answer.
pub const MAX_OUTPUT_BYTES: usize = 256 * 1024;

/// preview-1 `errno` values we return. Only the ones this host can produce.
mod errno {
    pub const SUCCESS: i32 = 0;
    pub const BADF: i32 = 8;
    pub const INVAL: i32 = 28;
}

/// Guest-visible file descriptors.
mod fd {
    pub const STDIN: i32 = 0;
    pub const STDOUT: i32 = 1;
    pub const STDERR: i32 = 2;
}

/// Everything the guest may touch, and the only place its output accumulates.
struct HostState {
    /// The invoke envelope, served to the guest through `fd_read` on stdin.
    stdin: Vec<u8>,
    stdin_pos: usize,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    /// Set by `proc_exit`, which unwinds the call as a trap.
    exit_code: Option<i32>,
    /// Truncation is recorded rather than silently dropped.
    truncated: bool,
    limiter: MemoryCap,
}

/// Caps guest linear memory at instantiate *and* at every `memory.grow`.
struct MemoryCap {
    max_bytes: usize,
    hit: Arc<Mutex<bool>>,
}

impl wasmi::ResourceLimiter for MemoryCap {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmi_core::LimiterError> {
        if desired > self.max_bytes {
            if let Ok(mut hit) = self.hit.lock() {
                *hit = true;
            }
            return Ok(false);
        }
        Ok(true)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmi_core::LimiterError> {
        Ok(true)
    }

    /// One instance, one memory, one table — a slash handler is a single module
    /// with no nested instantiation.
    fn instances(&self) -> usize {
        1
    }

    fn tables(&self) -> usize {
        1
    }

    fn memories(&self) -> usize {
        1
    }
}

impl HostState {
    fn push_output(&mut self, which: i32, bytes: &[u8]) {
        let buf = if which == fd::STDERR {
            &mut self.stderr
        } else {
            &mut self.stdout
        };
        let room = MAX_OUTPUT_BYTES.saturating_sub(buf.len());
        if room == 0 {
            self.truncated = true;
            return;
        }
        if bytes.len() > room {
            buf.extend_from_slice(&bytes[..room]);
            self.truncated = true;
        } else {
            buf.extend_from_slice(bytes);
        }
    }
}

/// Why a run did not produce a result.
#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("module is not valid wasm: {0}")]
    InvalidModule(String),
    #[error("module imports `{module}::{name}`, which is not on the preview-1 allowlist")]
    BannedImport { module: String, name: String },
    #[error("module has no `_start` export")]
    MissingStart,
}

/// Run `module_bytes` against `invoke`, bounded by `limits`.
///
/// Never panics and never returns `Err` for guest misbehaviour — a trap, a fuel
/// exhaustion or a banned import is a [`WasiResult`] with the matching
/// [`WasiFailureKind`], because those are answers about the guest rather than
/// host failures.
pub fn run(module_bytes: &[u8], invoke: &WasiInvoke, limits: WasiLimits) -> WasiResult {
    match run_inner(module_bytes, invoke, limits) {
        Ok(result) => result,
        Err(HostError::InvalidModule(e)) => WasiResult::fail(WasiFailureKind::InvalidModule, e),
        Err(e @ HostError::MissingStart) => {
            WasiResult::fail(WasiFailureKind::InvalidModule, e.to_string())
        }
        Err(e @ HostError::BannedImport { .. }) => {
            WasiResult::fail(WasiFailureKind::BannedImport, e.to_string())
        }
    }
}

/// Reject a module whose imports are not all on the allowlist, before any of it
/// is instantiated.
///
/// Linking would reject an unknown import anyway; checking first turns that into
/// a named, actionable error rather than a generic link failure, and keeps the
/// rejection independent of which host functions happen to be registered.
fn check_imports(module: &wasmi::Module) -> Result<(), HostError> {
    for import in module.imports() {
        let module_name = import.module();
        let field = import.name();
        if module_name != WASI_PREVIEW1_MODULE || !WASI_ALLOWED_PREVIEW1.contains(&field) {
            return Err(HostError::BannedImport {
                module: module_name.to_string(),
                name: field.to_string(),
            });
        }
    }
    Ok(())
}

fn run_inner(
    module_bytes: &[u8],
    invoke: &WasiInvoke,
    limits: WasiLimits,
) -> Result<WasiResult, HostError> {
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    let engine = wasmi::Engine::new(&config);

    let module = wasmi::Module::new(&engine, module_bytes)
        .map_err(|e| HostError::InvalidModule(e.to_string()))?;
    check_imports(&module)?;

    let stdin = serde_json::to_vec(invoke).unwrap_or_default();
    let memory_hit = Arc::new(Mutex::new(false));
    let state = HostState {
        stdin,
        stdin_pos: 0,
        stdout: Vec::new(),
        stderr: Vec::new(),
        exit_code: None,
        truncated: false,
        limiter: MemoryCap {
            max_bytes: limits.memory_bytes as usize,
            hit: Arc::clone(&memory_hit),
        },
    };

    let mut store = wasmi::Store::new(&engine, state);
    store.limiter(|s| &mut s.limiter);
    if store.set_fuel(limits.fuel).is_err() {
        return Ok(WasiResult::fail(
            WasiFailureKind::InvalidModule,
            "engine rejected the fuel limit",
        ));
    }

    let mut linker = wasmi::Linker::new(&engine);
    if let Err(e) = register_preview1(&mut linker) {
        return Ok(WasiResult::fail(WasiFailureKind::InvalidModule, e));
    }

    // `instantiate_and_start` runs any `start` section under the same fuel and
    // memory caps as the rest of the guest — a module that burns its budget in
    // `start` is exhausted, not privileged.
    let instance = match linker.instantiate_and_start(&mut store, &module) {
        Ok(i) => i,
        Err(e) => return Ok(classify(&mut store, &memory_hit, &e.to_string())),
    };

    let start = match instance.get_typed_func::<(), ()>(&store, "_start") {
        Ok(f) => f,
        Err(_) => return Err(HostError::MissingStart),
    };

    let outcome = start.call(&mut store, ());
    let data = store.data();
    let stdout = String::from_utf8_lossy(&data.stdout).to_string();
    let mut stderr = String::from_utf8_lossy(&data.stderr).to_string();
    if data.truncated {
        stderr.push_str("\n[maidan: output truncated at 256 KiB]");
    }
    let exit_code = data.exit_code;

    match outcome {
        // A clean return, or `proc_exit(0)` unwinding as a trap.
        Ok(()) => Ok(WasiResult::ok(stdout, stderr)),
        Err(_) if exit_code == Some(0) => Ok(WasiResult::ok(stdout, stderr)),
        Err(e) => {
            let mut failed = classify(&mut store, &memory_hit, &e.to_string());
            // What the guest managed to say before it failed is the most useful
            // thing in the report, so a failure carries it too.
            failed.stdout = stdout;
            failed.stderr = stderr;
            failed.exit_code = exit_code;
            Ok(failed)
        }
    }
}

/// Map a wasmi error into the ABI's failure vocabulary.
///
/// Fuel and the memory cap are checked from *host* state rather than by matching
/// on the error text: both surface as ordinary traps, and a string match would
/// silently reclassify them the first time wasmi reworded a message.
///
/// # Why this order
///
/// The four causes are not mutually exclusive, so precedence is a decision, not
/// an accident. A guest that hits the memory cap typically fails its next
/// allocation and *then* calls `proc_exit(1)` from its own error path — both
/// facts are true, and reporting the exit would hand the author the symptom
/// while hiding the cause. So a host-enforced limit outranks the guest's own
/// verdict: memory, then fuel, then the guest's exit status, then a plain trap
/// as the residual "it broke and we know nothing more specific".
fn classify(
    store: &mut wasmi::Store<HostState>,
    memory_hit: &Arc<Mutex<bool>>,
    message: &str,
) -> WasiResult {
    if memory_hit.lock().map(|h| *h).unwrap_or(false) {
        return WasiResult::fail(
            WasiFailureKind::MemoryLimit,
            "guest exceeded its linear-memory cap",
        );
    }
    if store.get_fuel().map(|f| f == 0).unwrap_or(false) {
        return WasiResult::fail(
            WasiFailureKind::FuelExhausted,
            "guest exhausted its fuel budget",
        );
    }
    // The status is read back out of host state, set by our own `proc_exit`,
    // so this is an observed fact about the guest rather than an inference from
    // an error string. Zero is not a failure cause and falls through.
    match store.data().exit_code {
        Some(code) if code != 0 => {
            return WasiResult::fail(
                WasiFailureKind::ExitNonZero,
                format!("guest exited with status {code}"),
            )
        }
        _ => {}
    }
    WasiResult::fail(WasiFailureKind::Trap, message)
}

/// Register **exactly** the allowlisted preview-1 calls.
///
/// Nothing here opens a path, resolves a name, or touches a socket. The
/// filesystem functions a normal wasi-libc startup calls (`fd_prestat_get`,
/// `fd_prestat_dir_name`) deliberately report "no preopens", which is what makes
/// the guest see an empty filesystem rather than a restricted one.
fn register_preview1(linker: &mut wasmi::Linker<HostState>) -> Result<(), String> {
    use wasmi::{Caller, Extern};

    fn memory(caller: &mut Caller<'_, HostState>) -> Option<wasmi::Memory> {
        match caller.get_export("memory") {
            Some(Extern::Memory(m)) => Some(m),
            _ => None,
        }
    }

    /// Write a little-endian u32 into guest memory.
    fn put_u32(
        caller: &mut Caller<'_, HostState>,
        mem: wasmi::Memory,
        ptr: i32,
        value: u32,
    ) -> Result<(), ()> {
        mem.write(caller, ptr as usize, &value.to_le_bytes())
            .map_err(|_| ())
    }

    fn read_bytes(
        caller: &mut Caller<'_, HostState>,
        mem: wasmi::Memory,
        ptr: i32,
        len: usize,
    ) -> Result<Vec<u8>, ()> {
        let mut buf = vec![0u8; len];
        mem.read(caller, ptr as usize, &mut buf).map_err(|_| ())?;
        Ok(buf)
    }

    // --- stdio -----------------------------------------------------------
    // `fd_write(fd, iovs, iovs_len, nwritten) -> errno`
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_write",
            |mut caller: Caller<'_, HostState>,
             fd_num: i32,
             iovs: i32,
             iovs_len: i32,
             nwritten: i32|
             -> i32 {
                if fd_num != fd::STDOUT && fd_num != fd::STDERR {
                    return errno::BADF;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let mut total: u32 = 0;
                for i in 0..iovs_len.max(0) {
                    let base = iovs + i * 8;
                    let Ok(head) = read_bytes(&mut caller, mem, base, 8) else {
                        return errno::INVAL;
                    };
                    let ptr = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as i32;
                    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
                    let Ok(chunk) = read_bytes(&mut caller, mem, ptr, len) else {
                        return errno::INVAL;
                    };
                    caller.data_mut().push_output(fd_num, &chunk);
                    total = total.saturating_add(len as u32);
                }
                if put_u32(&mut caller, mem, nwritten, total).is_err() {
                    return errno::INVAL;
                }
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;

    // `fd_read(fd, iovs, iovs_len, nread) -> errno` — stdin serves the invoke.
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_read",
            |mut caller: Caller<'_, HostState>,
             fd_num: i32,
             iovs: i32,
             iovs_len: i32,
             nread: i32|
             -> i32 {
                if fd_num != fd::STDIN {
                    return errno::BADF;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let mut total: u32 = 0;
                for i in 0..iovs_len.max(0) {
                    let base = iovs + i * 8;
                    let Ok(head) = read_bytes(&mut caller, mem, base, 8) else {
                        return errno::INVAL;
                    };
                    let ptr = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
                    let cap = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
                    let (pos, remaining) = {
                        let d = caller.data();
                        (d.stdin_pos, d.stdin.len().saturating_sub(d.stdin_pos))
                    };
                    let take = cap.min(remaining);
                    if take == 0 {
                        break;
                    }
                    let chunk = caller.data().stdin[pos..pos + take].to_vec();
                    if mem.write(&mut caller, ptr, &chunk).is_err() {
                        return errno::INVAL;
                    }
                    caller.data_mut().stdin_pos += take;
                    total = total.saturating_add(take as u32);
                }
                if put_u32(&mut caller, mem, nread, total).is_err() {
                    return errno::INVAL;
                }
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;

    // `proc_exit(code)` — unwinds; the caller reads `exit_code` to tell a clean
    // exit from a trap.
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "proc_exit",
            |mut caller: Caller<'_, HostState>, code: i32| {
                caller.data_mut().exit_code = Some(code);
                Err::<(), _>(wasmi::Error::host(HostExit(code)))
            },
        )
        .map_err(|e| e.to_string())?;

    // --- descriptors a wasi-libc startup touches -------------------------
    // No preopens: the guest sees an empty filesystem, not a restricted one.
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_prestat_get",
            |_: Caller<'_, HostState>, _fd: i32, _buf: i32| -> i32 { errno::BADF },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_prestat_dir_name",
            |_: Caller<'_, HostState>, _fd: i32, _path: i32, _len: i32| -> i32 { errno::BADF },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_close",
            |_: Caller<'_, HostState>, _fd: i32| -> i32 { errno::SUCCESS },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_seek",
            |_: Caller<'_, HostState>, _fd: i32, _off: i64, _whence: i32, _out: i32| -> i32 {
                errno::BADF
            },
        )
        .map_err(|e| e.to_string())?;
    // A character device with no seek — what stdio actually is here.
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "fd_fdstat_get",
            |mut caller: Caller<'_, HostState>, fd_num: i32, buf: i32| -> i32 {
                if !(fd::STDIN..=fd::STDERR).contains(&fd_num) {
                    return errno::BADF;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let mut stat = [0u8; 24];
                stat[0] = 2; // filetype: character_device
                if mem.write(&mut caller, buf as usize, &stat).is_err() {
                    return errno::INVAL;
                }
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;

    // --- args / env: deliberately empty ----------------------------------
    // The invoke arrives on stdin. Mirroring ids into argv/env would be a second
    // copy of the same authority with no reader.
    for name in ["args_sizes_get", "environ_sizes_get"] {
        linker
            .func_wrap(
                WASI_PREVIEW1_MODULE,
                name,
                |mut caller: Caller<'_, HostState>, count: i32, size: i32| -> i32 {
                    let Some(mem) = memory(&mut caller) else {
                        return errno::INVAL;
                    };
                    if put_u32(&mut caller, mem, count, 0).is_err()
                        || put_u32(&mut caller, mem, size, 0).is_err()
                    {
                        return errno::INVAL;
                    }
                    errno::SUCCESS
                },
            )
            .map_err(|e| e.to_string())?;
    }
    for name in ["args_get", "environ_get"] {
        linker
            .func_wrap(
                WASI_PREVIEW1_MODULE,
                name,
                |_: Caller<'_, HostState>, _a: i32, _b: i32| -> i32 { errno::SUCCESS },
            )
            .map_err(|e| e.to_string())?;
    }

    // --- clocks, randomness, yield ---------------------------------------
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "clock_time_get",
            |mut caller: Caller<'_, HostState>, _id: i32, _precision: i64, out: i32| -> i32 {
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                if mem
                    .write(&mut caller, out as usize, &nanos.to_le_bytes())
                    .is_err()
                {
                    return errno::INVAL;
                }
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "clock_res_get",
            |mut caller: Caller<'_, HostState>, _id: i32, out: i32| -> i32 {
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                if mem
                    .write(&mut caller, out as usize, &1_000u64.to_le_bytes())
                    .is_err()
                {
                    return errno::INVAL;
                }
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;
    // Deterministic-by-default: a slash handler has no business needing entropy,
    // and a zero fill keeps a run reproducible for the operator debugging it.
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "random_get",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| -> i32 {
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let zeros = vec![0u8; len.max(0) as usize];
                if mem.write(&mut caller, ptr as usize, &zeros).is_err() {
                    return errno::INVAL;
                }
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "sched_yield",
            |_: Caller<'_, HostState>| -> i32 { errno::SUCCESS },
        )
        .map_err(|e| e.to_string())?;

    Ok(())
}

/// Carries `proc_exit`'s status out through the trap that unwinds the call.
#[derive(Debug)]
struct HostExit(i32);

impl std::fmt::Display for HostExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "guest called proc_exit({})", self.0)
    }
}

impl std::error::Error for HostExit {}

impl wasmi::errors::HostError for HostExit {}
