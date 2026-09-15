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
    BannedImport,
    InvalidModule,
}

impl WasiFailureKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FuelExhausted => "fuel_exhausted",
            Self::MemoryLimit => "memory_limit",
            Self::Trap => "trap",
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
        }
    }

    pub fn fail(kind: WasiFailureKind, error: impl Into<String>) -> Self {
        Self {
            type_id: WASI_RESULT_TYPE.to_string(),
            ok: false,
            stdout: String::new(),
            stderr: String::new(),
            error: Some(error.into()),
            error_kind: Some(kind),
        }
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
