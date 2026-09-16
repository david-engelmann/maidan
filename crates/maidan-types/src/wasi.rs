//! WASI slash-handler ABI (Cluster 396, Wave 3 #36).
//!
//! A workspace installs a **no-network guest** as a slash-command handler.
//! The guest **is the tool** — not an agent runtime. Maidan stores the
//! module (content-addressed artifact SHA) and invokes it with fuel and
//! memory caps.
//!
//! `$type` is the contract. Breaking changes are `/2`.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::ids::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};

/// Observable `$type` for the host→guest invoke envelope (stdin JSON).
pub const WASI_INVOKE_TYPE: &str = "maidan.slash.wasi-invoke/1";

/// Observable `$type` for the runtime→room result envelope.
pub const WASI_RESULT_TYPE: &str = "maidan.slash.wasi-result/1";

/// WASI preview 1 module name. Any other import module is a banned
/// outbound host call.
pub const WASI_PREVIEW1_MODULE: &str = "wasi_snapshot_preview1";

/// Default fuel (interpreter / cranelift fuel units).
pub const WASI_DEFAULT_FUEL: u64 = 25_000_000;

/// Hard upper bound on fuel. Env overrides clamp here.
pub const WASI_MAX_FUEL: u64 = 100_000_000;

/// Default guest linear memory (16 MiB).
pub const WASI_DEFAULT_MEMORY_BYTES: u64 = 16 * 1024 * 1024;

/// Hard upper bound on guest linear memory (64 MiB).
pub const WASI_MAX_MEMORY_BYTES: u64 = 64 * 1024 * 1024;

/// Smallest memory we will grant (one Wasm page).
pub const WASI_MIN_MEMORY_BYTES: u64 = 64 * 1024;

/// Longest failure message we will carry out of a run.
///
/// A failure message is partly guest-derived — a wasm validation error quotes
/// the offending import or export name, and those are attacker-chosen and
/// effectively unbounded. The message is persisted in the triggering message's
/// metadata and broadcast to every subscriber, so it is bounded here rather
/// than trusted to be short.
pub const WASI_MAX_ERROR_BYTES: usize = 512;

/// Largest guest output the *room* will carry, independent of how much the
/// sandbox was willing to buffer.
///
/// The sandbox's own output cap bounds host memory during a run. This bounds
/// something different and more expensive: a slash response is written into the
/// triggering message's metadata and fanned out to every live subscriber, so
/// guest output here is persisted and replicated, not just held. A handler
/// answering a chat message has no legitimate need for more.
pub const WASI_MAX_ROOM_OUTPUT_BYTES: usize = 16 * 1024;

/// Truncate `s` to at most `max` bytes without splitting a UTF-8 character,
/// marking the cut so a reader never mistakes a clipped value for a complete
/// one.
pub fn truncate_utf8(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    const MARK: &str = "… [maidan: truncated]";
    let budget = max.saturating_sub(MARK.len());
    let mut end = budget.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{MARK}", &s[..end])
}

/// Allowlisted `wasi_snapshot_preview1` imports. Sockets, filesystem
/// paths, and unknown names are denied. Stdin/stdout/args/env/clocks
/// only — the guest cannot leave the process.
pub const WASI_ALLOWED_PREVIEW1: &[&str] = &[
    "args_get",
    "args_sizes_get",
    "clock_res_get",
    "clock_time_get",
    "environ_get",
    "environ_sizes_get",
    "fd_close",
    "fd_fdstat_get",
    "fd_prestat_dir_name",
    "fd_prestat_get",
    "fd_read",
    "fd_seek",
    "fd_write",
    "proc_exit",
    "random_get",
    "sched_yield",
];

/// Fuel + memory caps applied at instantiate / invoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WasiLimits {
    pub fuel: u64,
    pub memory_bytes: u64,
}

impl Default for WasiLimits {
    fn default() -> Self {
        Self {
            fuel: WASI_DEFAULT_FUEL,
            memory_bytes: WASI_DEFAULT_MEMORY_BYTES,
        }
    }
}

impl WasiLimits {
    /// Clamp caller-supplied caps into the hard bounds.
    pub fn clamp(fuel: u64, memory_bytes: u64) -> Self {
        Self {
            fuel: fuel.clamp(1, WASI_MAX_FUEL),
            memory_bytes: memory_bytes.clamp(WASI_MIN_MEMORY_BYTES, WASI_MAX_MEMORY_BYTES),
        }
    }
}

/// Host→guest invoke. Passed as stdin JSON; env/argv may mirror the ids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WasiInvoke {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub command: String,
    pub args: String,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub message_id: MessageId,
}

impl WasiInvoke {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        command: impl Into<String>,
        args: impl Into<String>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        author_id: MemberId,
        message_id: MessageId,
    ) -> Self {
        Self {
            type_id: WASI_INVOKE_TYPE.to_string(),
            command: command.into(),
            args: args.into(),
            workspace_id,
            channel_id,
            thread_id,
            author_id,
            message_id,
        }
    }
}

/// Why a guest failed. Wire `snake_case`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WasiFailureKind {
    FuelExhausted,
    MemoryLimit,
    Trap,
    /// The guest called `proc_exit` with a non-zero status. Distinct from
    /// [`Self::Trap`]: the handler *chose* to fail and picked the code, so the
    /// author is looking for their own error path, not a crash.
    ExitNonZero,
    BannedImport,
    InvalidModule,
}

impl WasiFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FuelExhausted => "fuel_exhausted",
            Self::MemoryLimit => "memory_limit",
            Self::Trap => "trap",
            Self::ExitNonZero => "exit_non_zero",
            Self::BannedImport => "banned_import",
            Self::InvalidModule => "invalid_module",
        }
    }
}

/// Runtime→room result. `stdout` is the guest's captured output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WasiResult {
    #[serde(rename = "$type")]
    pub type_id: String,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stdout: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub stderr: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<WasiFailureKind>,
    /// The guest's own `proc_exit` status, when it chose one. Carried
    /// structurally rather than only inside `error`, so a caller can branch on
    /// a handler's exit code without parsing prose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
}

impl WasiResult {
    pub fn ok(stdout: impl Into<String>, stderr: impl Into<String>) -> Self {
        Self {
            type_id: WASI_RESULT_TYPE.to_string(),
            ok: true,
            stdout: stdout.into(),
            stderr: stderr.into(),
            error: None,
            error_kind: None,
            exit_code: None,
        }
    }

    /// Every failure path goes through here, which is why the message is
    /// bounded here rather than at each call site — a constructor that cannot
    /// be bypassed is the only kind that cannot be forgotten.
    pub fn fail(kind: WasiFailureKind, error: impl Into<String>) -> Self {
        Self {
            type_id: WASI_RESULT_TYPE.to_string(),
            ok: false,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(truncate_utf8(&error.into(), WASI_MAX_ERROR_BYTES)),
            error_kind: Some(kind),
            exit_code: None,
        }
    }

    /// Clamp `stdout`/`stderr` to what the room will persist and broadcast.
    ///
    /// Applied at the slash surface, not inside the sandbox: the sandbox's cap
    /// protects host memory during a run, this protects the event log.
    pub fn clamp_for_room(mut self) -> Self {
        self.stdout = truncate_utf8(&self.stdout, WASI_MAX_ROOM_OUTPUT_BYTES);
        self.stderr = truncate_utf8(&self.stderr, WASI_MAX_ROOM_OUTPUT_BYTES);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WasiTargetError {
    #[error("wasi handler_target must be a 64-char sha256 hex (optional sha256: prefix)")]
    Syntax,
}

/// Normalize a WASI slash `handler_target` to lowercase 64-char hex.
///
/// Accepts raw hex or `sha256:<hex>`. This is an artifact content-hash,
/// not a URL — the guest bytes live in the existing artifact store.
pub fn normalize_wasi_handler_target(target: &str) -> Result<String, WasiTargetError> {
    let trimmed = target.trim();
    let hex = trimmed
        .strip_prefix("sha256:")
        .or_else(|| trimmed.strip_prefix("SHA256:"))
        .unwrap_or(trimmed);
    if hex.len() != 64 {
        return Err(WasiTargetError::Syntax);
    }
    if !hex.as_bytes().iter().all(|b| b.is_ascii_hexdigit()) {
        return Err(WasiTargetError::Syntax);
    }
    Ok(hex.to_ascii_lowercase())
}

/// True when `(module, name)` is an allowed WASI preview 1 import.
///
/// The lookup is a binary search, so [`WASI_ALLOWED_PREVIEW1`] **must stay
/// sorted** — an out-of-order entry would make this silently accept a banned
/// import or reject an allowed one, with no error anywhere. Guarded by
/// `the_preview1_allowlist_is_sorted`.
/// Anything else is a banned network / filesystem / host call.
pub fn is_allowed_wasi_import(module: &str, name: &str) -> bool {
    module == WASI_PREVIEW1_MODULE && WASI_ALLOWED_PREVIEW1.binary_search(&name).is_ok()
}

/// True when the import must be rejected (sockets, path_*, unknown host).
pub fn is_banned_wasi_import(module: &str, name: &str) -> bool {
    !is_allowed_wasi_import(module, name)
}

/// Stable ids for tests / examples (not nil — FK-safe shapes).
pub fn example_wasi_invoke() -> WasiInvoke {
    WasiInvoke::new(
        "echo",
        "hello",
        WorkspaceId(Uuid::from_u128(1)),
        ChannelId(Uuid::from_u128(2)),
        ThreadId(Uuid::from_u128(3)),
        MemberId(Uuid::from_u128(4)),
        MessageId(Uuid::from_u128(5)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handler_kind_wasi_round_trips() {
        use crate::SlashHandlerKind;
        assert_eq!(SlashHandlerKind::Wasi.as_str(), "wasi");
        assert_eq!(
            SlashHandlerKind::parse("wasi"),
            Some(SlashHandlerKind::Wasi)
        );
        assert!(!SlashHandlerKind::Wasi.allowed_for_fsm_hooks());
        assert!(SlashHandlerKind::Http.allowed_for_fsm_hooks());
        assert!(SlashHandlerKind::McpTool.allowed_for_fsm_hooks());
    }

    #[test]
    fn allowed_preview1_is_sorted_for_binary_search() {
        let mut sorted = WASI_ALLOWED_PREVIEW1.to_vec();
        sorted.sort_unstable();
        assert_eq!(WASI_ALLOWED_PREVIEW1, sorted.as_slice());
    }

    #[test]
    fn normalize_sha_accepts_prefix_and_case() {
        let hex = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(normalize_wasi_handler_target(hex).unwrap(), hex);
        assert_eq!(
            normalize_wasi_handler_target(&hex.to_ascii_uppercase()).unwrap(),
            hex
        );
        assert_eq!(
            normalize_wasi_handler_target(&format!("sha256:{hex}")).unwrap(),
            hex
        );
    }

    #[test]
    fn normalize_sha_rejects_short_and_non_hex() {
        assert!(normalize_wasi_handler_target("abc").is_err());
        assert!(normalize_wasi_handler_target(&"g".repeat(64)).is_err());
        assert!(normalize_wasi_handler_target("https://example.test/mod.wasm").is_err());
    }

    #[test]
    fn sockets_and_foreign_modules_are_banned() {
        assert!(is_banned_wasi_import(WASI_PREVIEW1_MODULE, "sock_recv"));
        assert!(is_banned_wasi_import(WASI_PREVIEW1_MODULE, "path_open"));
        assert!(is_banned_wasi_import(
            "wasi:sockets/tcp@0.2.0",
            "start-connect"
        ));
        assert!(is_banned_wasi_import("env", "host_fetch"));
        assert!(is_allowed_wasi_import(WASI_PREVIEW1_MODULE, "fd_write"));
        assert!(is_allowed_wasi_import(WASI_PREVIEW1_MODULE, "args_get"));
    }

    #[test]
    fn limits_clamp_to_hard_bounds() {
        let hi = WasiLimits::clamp(u64::MAX, u64::MAX);
        assert_eq!(hi.fuel, WASI_MAX_FUEL);
        assert_eq!(hi.memory_bytes, WASI_MAX_MEMORY_BYTES);
        let lo = WasiLimits::clamp(0, 1);
        assert_eq!(lo.fuel, 1);
        assert_eq!(lo.memory_bytes, WASI_MIN_MEMORY_BYTES);
        assert_eq!(WasiLimits::default().fuel, WASI_DEFAULT_FUEL);
    }

    #[test]
    fn invoke_and_result_carry_type() {
        let invoke = example_wasi_invoke();
        let json = serde_json::to_value(&invoke).unwrap();
        assert_eq!(json["$type"], WASI_INVOKE_TYPE);
        assert_eq!(json["command"], "echo");
        let ok = WasiResult::ok("{\"text\":\"hi\"}", "");
        let ok_json = serde_json::to_value(&ok).unwrap();
        assert_eq!(ok_json["$type"], WASI_RESULT_TYPE);
        assert_eq!(ok_json["ok"], true);
        assert!(ok_json.get("error").is_none());
        let fail = WasiResult::fail(WasiFailureKind::FuelExhausted, "fuel");
        let fail_json = serde_json::to_value(&fail).unwrap();
        assert_eq!(fail_json["error_kind"], "fuel_exhausted");
        assert_eq!(fail_json["ok"], false);
    }
}

#[cfg(test)]
mod allowlist_order_tests {
    use super::{is_allowed_wasi_import, WASI_ALLOWED_PREVIEW1, WASI_PREVIEW1_MODULE};

    /// `is_allowed_wasi_import` binary-searches the allowlist, so order is
    /// load-bearing: an entry in the wrong place would make a banned import look
    /// allowed (or the reverse) with nothing to notice it.
    #[test]
    fn the_preview1_allowlist_is_sorted() {
        let mut sorted = WASI_ALLOWED_PREVIEW1.to_vec();
        sorted.sort_unstable();
        assert_eq!(
            WASI_ALLOWED_PREVIEW1,
            sorted.as_slice(),
            "WASI_ALLOWED_PREVIEW1 must stay sorted for the binary search to be correct"
        );
    }

    /// Every listed import resolves, and the obvious escapes do not.
    #[test]
    fn the_allowlist_admits_only_itself() {
        for name in WASI_ALLOWED_PREVIEW1 {
            assert!(
                is_allowed_wasi_import(WASI_PREVIEW1_MODULE, name),
                "{name} is on the allowlist but did not resolve"
            );
        }
        for name in [
            "path_open",
            "sock_connect",
            "fd_readdir",
            "path_unlink_file",
        ] {
            assert!(
                !is_allowed_wasi_import(WASI_PREVIEW1_MODULE, name),
                "{name} must not be allowed"
            );
        }
        assert!(
            !is_allowed_wasi_import("env", "args_get"),
            "module is checked too"
        );
    }
}

#[cfg(test)]
mod result_bounds_tests {
    use super::{
        truncate_utf8, WasiFailureKind, WasiResult, WASI_MAX_ERROR_BYTES,
        WASI_MAX_ROOM_OUTPUT_BYTES,
    };

    /// The bound is in bytes but the content is text, and a guest chooses the
    /// text — so the cut must land on a character boundary. Slicing a 3-byte
    /// character in half would panic inside the failure path, turning a handled
    /// guest failure into a host one.
    #[test]
    fn truncation_never_splits_a_character() {
        let wide = "空".repeat(1000);
        for max in [1, 2, 3, 7, 64, 512] {
            let cut = truncate_utf8(&wide, max);
            assert!(cut.len() <= max.max(1) + 64, "grossly over budget at {max}");
            assert!(std::str::from_utf8(cut.as_bytes()).is_ok());
        }
        assert_eq!(
            truncate_utf8("short", 512),
            "short",
            "under budget is intact"
        );
    }

    /// A truncated value must never read as a complete one.
    #[test]
    fn truncation_marks_the_cut() {
        let cut = truncate_utf8(&"x".repeat(5_000), WASI_MAX_ERROR_BYTES);
        assert!(cut.len() <= WASI_MAX_ERROR_BYTES);
        assert!(cut.contains("truncated"), "the cut must be visible: {cut}");
    }

    /// The room bound is separate from, and tighter than, the sandbox's own
    /// output cap: this one governs what gets persisted and fanned out.
    #[test]
    fn a_result_is_clamped_to_what_the_room_will_carry() {
        let noisy = WasiResult::ok("o".repeat(900_000), "e".repeat(900_000)).clamp_for_room();
        assert!(noisy.stdout.len() <= WASI_MAX_ROOM_OUTPUT_BYTES);
        assert!(noisy.stderr.len() <= WASI_MAX_ROOM_OUTPUT_BYTES);
        assert!(noisy.stdout.contains("truncated"));
        assert!(noisy.ok, "clamping is not a failure");
    }

    /// Every failure constructor bounds its message, including one handed an
    /// oversized string directly.
    #[test]
    fn a_failure_message_is_bounded_at_construction() {
        let f = WasiResult::fail(WasiFailureKind::Trap, "t".repeat(100_000));
        assert!(f.error.as_deref().unwrap_or_default().len() <= WASI_MAX_ERROR_BYTES);
    }
}
