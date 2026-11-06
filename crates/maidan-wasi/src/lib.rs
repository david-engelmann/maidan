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
//! [`WASI_ALLOWED_PREVIEW1`] names 16 preview-1 calls — stdin/stdout, args, env,
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
    /// A guest pointer is outside the linear memory it was given.
    pub const FAULT: i32 = 21;
    pub const INVAL: i32 = 28;
}

/// preview-1 allows a scatter/gather list, not a loop the guest can use to
/// burn host time without spending Wasm fuel.
const MAX_IOVS: usize = 1024;

/// A `funcref` table past this, or whose pointer-sized footprint exceeds the
/// memory cap, is the same failure as growing linear memory past the cap.
const MAX_TABLE_ELEMENTS: usize = 16_384;

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
        desired: usize,
        _maximum: Option<usize>,
    ) -> Result<bool, wasmi_core::LimiterError> {
        let bytes = desired.saturating_mul(std::mem::size_of::<usize>());
        if desired > MAX_TABLE_ELEMENTS || bytes > self.max_bytes {
            if let Ok(mut hit) = self.hit.lock() {
                *hit = true;
            }
            return Ok(false);
        }
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
            );
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

    /// Wasm32 addresses are unsigned. `as usize` on a negative `i32` sign-extends.
    fn guest_ptr(ptr: i32) -> usize {
        ptr as u32 as usize
    }

    fn fits(mem_len: usize, start: usize, len: usize) -> bool {
        start.checked_add(len).is_some_and(|end| end <= mem_len)
    }

    /// One fuel unit per 64 bytes scanned, plus one per record, so a guest
    /// cannot spend a single Wasm instruction on a huge host `memset` or copy.
    fn fuel_for(bytes: usize, records: usize) -> u64 {
        (bytes as u64).div_ceil(64).saturating_add(records as u64)
    }

    fn charge(caller: &mut Caller<'_, HostState>, units: u64) -> bool {
        if units == 0 {
            return true;
        }
        let Ok(fuel) = caller.get_fuel() else {
            return true;
        };
        if fuel < units {
            let _ = caller.set_fuel(0);
            return false;
        }
        caller.set_fuel(fuel - units).is_ok()
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
                if iovs_len < 0 || (iovs_len as usize) > MAX_IOVS {
                    return errno::INVAL;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let mem_len = mem.data(&caller).len();
                let nwritten_at = guest_ptr(nwritten);
                if !fits(mem_len, nwritten_at, 4) {
                    return errno::FAULT;
                }
                let count = iovs_len as usize;
                let mut planned: Vec<(usize, usize)> = Vec::with_capacity(count);
                let mut total: u32 = 0;
                for i in 0..count {
                    let Some(base) = (iovs as u32).checked_add((i as u32).saturating_mul(8)) else {
                        return errno::INVAL;
                    };
                    let base = base as usize;
                    if !fits(mem_len, base, 8) {
                        return errno::FAULT;
                    }
                    let mut head = [0u8; 8];
                    head.copy_from_slice(&mem.data(&caller)[base..base + 8]);
                    let buf = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
                    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
                    if len > 0 && !fits(mem_len, buf, len) {
                        return errno::FAULT;
                    }
                    total = match total.checked_add(len as u32) {
                        Some(sum) => sum,
                        None => return errno::INVAL,
                    };
                    planned.push((buf, len));
                }
                if !charge(&mut caller, fuel_for(total as usize, count)) {
                    return errno::INVAL;
                }
                for (buf, len) in planned {
                    let mut copied = 0;
                    while copied < len {
                        let n = (len - copied).min(4096);
                        let mut tmp = [0u8; 4096];
                        let start = buf + copied;
                        tmp[..n].copy_from_slice(&mem.data(&caller)[start..start + n]);
                        caller.data_mut().push_output(fd_num, &tmp[..n]);
                        copied += n;
                    }
                }
                mem.data_mut(&mut caller)[nwritten_at..nwritten_at + 4]
                    .copy_from_slice(&total.to_le_bytes());
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
                if iovs_len < 0 || (iovs_len as usize) > MAX_IOVS {
                    return errno::INVAL;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let mem_len = mem.data(&caller).len();
                let nread_at = guest_ptr(nread);
                if !fits(mem_len, nread_at, 4) {
                    return errno::FAULT;
                }
                let remaining = {
                    let d = caller.data();
                    d.stdin.len().saturating_sub(d.stdin_pos)
                };
                let count = iovs_len as usize;
                let mut planned: Vec<(usize, usize)> = Vec::with_capacity(count);
                let mut left = remaining;
                let mut total: u32 = 0;
                for i in 0..count {
                    if left == 0 {
                        break;
                    }
                    let Some(base) = (iovs as u32).checked_add((i as u32).saturating_mul(8)) else {
                        return errno::INVAL;
                    };
                    let base = base as usize;
                    if !fits(mem_len, base, 8) {
                        return errno::FAULT;
                    }
                    let mut head = [0u8; 8];
                    head.copy_from_slice(&mem.data(&caller)[base..base + 8]);
                    let buf = u32::from_le_bytes([head[0], head[1], head[2], head[3]]) as usize;
                    let cap = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
                    if cap == 0 {
                        continue;
                    }
                    if !fits(mem_len, buf, cap) {
                        return errno::FAULT;
                    }
                    let take = cap.min(left);
                    total = match total.checked_add(take as u32) {
                        Some(sum) => sum,
                        None => return errno::INVAL,
                    };
                    planned.push((buf, take));
                    left -= take;
                }
                if !charge(&mut caller, fuel_for(total as usize, count)) {
                    return errno::INVAL;
                }
                let mut pos = caller.data().stdin_pos;
                for (buf, take) in planned {
                    let mut copied = 0;
                    while copied < take {
                        let n = (take - copied).min(4096);
                        let mut tmp = [0u8; 4096];
                        tmp[..n]
                            .copy_from_slice(&caller.data().stdin[pos + copied..pos + copied + n]);
                        mem.data_mut(&mut caller)[buf + copied..buf + copied + n]
                            .copy_from_slice(&tmp[..n]);
                        copied += n;
                    }
                    pos += take;
                }
                caller.data_mut().stdin_pos = pos;
                mem.data_mut(&mut caller)[nread_at..nread_at + 4]
                    .copy_from_slice(&total.to_le_bytes());
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
            |_: Caller<'_, HostState>, fd_num: i32| -> i32 {
                if !(fd::STDIN..=fd::STDERR).contains(&fd_num) {
                    return errno::BADF;
                }
                errno::SUCCESS
            },
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
                let start = guest_ptr(buf);
                if !fits(mem.data(&caller).len(), start, 24) {
                    return errno::FAULT;
                }
                let mut stat = [0u8; 24];
                stat[0] = 2; // filetype: character_device
                mem.data_mut(&mut caller)[start..start + 24].copy_from_slice(&stat);
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
                    let mem_len = mem.data(&caller).len();
                    let count_at = guest_ptr(count);
                    let size_at = guest_ptr(size);
                    if !fits(mem_len, count_at, 4) || !fits(mem_len, size_at, 4) {
                        return errno::FAULT;
                    }
                    mem.data_mut(&mut caller)[count_at..count_at + 4].fill(0);
                    mem.data_mut(&mut caller)[size_at..size_at + 4].fill(0);
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
            |mut caller: Caller<'_, HostState>, id: i32, _precision: i64, out: i32| -> i32 {
                if !(0..=3).contains(&id) {
                    return errno::INVAL;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let start = guest_ptr(out);
                if !fits(mem.data(&caller).len(), start, 8) {
                    return errno::FAULT;
                }
                let nanos = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                mem.data_mut(&mut caller)[start..start + 8].copy_from_slice(&nanos.to_le_bytes());
                errno::SUCCESS
            },
        )
        .map_err(|e| e.to_string())?;
    linker
        .func_wrap(
            WASI_PREVIEW1_MODULE,
            "clock_res_get",
            |mut caller: Caller<'_, HostState>, id: i32, out: i32| -> i32 {
                if !(0..=3).contains(&id) {
                    return errno::INVAL;
                }
                let Some(mem) = memory(&mut caller) else {
                    return errno::INVAL;
                };
                let start = guest_ptr(out);
                if !fits(mem.data(&caller).len(), start, 8) {
                    return errno::FAULT;
                }
                mem.data_mut(&mut caller)[start..start + 8]
                    .copy_from_slice(&1_000u64.to_le_bytes());
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
                // `len` is a Wasm `size` (`u32`). A negative `i32` is a huge length.
                let len = (len as u32) as usize;
                let start = guest_ptr(ptr);
                if len > 0 && !fits(mem.data(&caller).len(), start, len) {
                    return errno::FAULT;
                }
                if !charge(&mut caller, fuel_for(len, 0)) {
                    return errno::INVAL;
                }
                if len > 0 {
                    mem.data_mut(&mut caller)[start..start + len].fill(0);
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
