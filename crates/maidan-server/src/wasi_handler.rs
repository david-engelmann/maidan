//! Fetch and run a WASI slash handler (Cluster 399.2).
//!
//! The module bytes live in the existing content-addressed artifact store —
//! `handler_target` is a sha256, not a URL — so installing a handler is an
//! artifact upload plus a registration, and the Cluster-204 access link is what
//! makes a module belong to a workspace.
//!
//! The sandbox itself is [`maidan_wasi`]; this module is only the loader and the
//! envelope mapping.

use maidan_artifacts::Sha256;
use maidan_router::ParsedSlashCommand;
use maidan_types::{
    wasi::{normalize_wasi_handler_target, WasiFailureKind, WasiInvoke, WasiLimits, WasiResult},
    ChannelId, MemberId, MessageId, SlashCommand, ThreadId, WorkspaceId,
};
use serde_json::{json, Value};

use crate::state::AppState;

/// Load a handler's module, bounded by the workspace that registered it.
///
/// The tenancy check is against `command.workspace_id`, **not** the invoking
/// member's — a handler is installed by the workspace and runs on behalf of
/// anyone in it, so the question is "does this workspace own these bytes?".
/// Without the check, a registration could name any sha on the instance and turn
/// a slash command into a cross-tenant artifact reader.
async fn load_module(
    state: &AppState,
    auth: &maidan_auth::AuthContext,
    command: &SlashCommand,
) -> Result<Vec<u8>, WasiResult> {
    let hex = normalize_wasi_handler_target(&command.handler_target)
        .map_err(|e| WasiResult::fail(WasiFailureKind::InvalidModule, e.to_string()))?;
    let sha = Sha256::from_hex(&hex)
        .map_err(|e| WasiResult::fail(WasiFailureKind::InvalidModule, e.to_string()))?;

    // `bypass` is `AUTH_DISABLED`, where the upload path deliberately records no
    // access link at all (`upload_artifact` only refs for a non-bypass caller),
    // so a ref check would reject every module in that mode. Same carve-out as
    // `routes::artifact::ensure_artifact_ref`.
    if auth.bypass {
        return fetch_bytes(state, &sha).await;
    }

    match state
        .store
        .artifact_ref_exists(command.workspace_id, &hex)
        .await
    {
        Ok(true) => {}
        // Missing link and missing artifact are the same answer on purpose
        // (Cluster 204): a registration must not be able to confirm that some
        // other tenant's sha exists.
        Ok(false) => {
            return Err(WasiResult::fail(
                WasiFailureKind::InvalidModule,
                "handler module is not an artifact of this workspace",
            ))
        }
        Err(err) => {
            tracing::warn!(error = %err, "wasi handler: artifact ref lookup failed");
            return Err(WasiResult::fail(
                WasiFailureKind::InvalidModule,
                "handler module could not be resolved",
            ));
        }
    }

    fetch_bytes(state, &sha).await
}

async fn fetch_bytes(state: &AppState, sha: &Sha256) -> Result<Vec<u8>, WasiResult> {
    match state.artifacts.get(sha).await {
        Ok(bytes) => Ok(bytes.to_vec()),
        Err(err) => {
            tracing::warn!(error = %err, "wasi handler: artifact fetch failed");
            Err(WasiResult::fail(
                WasiFailureKind::InvalidModule,
                "handler module bytes are unavailable",
            ))
        }
    }
}

/// Run `command`'s guest and return the slash-response envelope.
#[allow(clippy::too_many_arguments)]
pub async fn dispatch_wasi(
    state: &AppState,
    auth: &maidan_auth::AuthContext,
    command: &SlashCommand,
    parsed: &ParsedSlashCommand,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread_id: ThreadId,
    author_id: MemberId,
    message_id: MessageId,
) -> Value {
    let module = match load_module(state, auth, command).await {
        Ok(bytes) => bytes,
        Err(failure) => return envelope(&failure),
    };

    let invoke = WasiInvoke::new(
        command.name.clone(),
        parsed.args.clone(),
        workspace_id,
        channel_id,
        thread_id,
        author_id,
        message_id,
    );
    let limits = WasiLimits::default();

    // Interpreting a guest is synchronous and CPU-bound — up to `WASI_MAX_FUEL`
    // of work on the calling thread. Left on the async runtime it would block a
    // worker for the duration, so a handful of slow handlers could stall
    // unrelated requests. `spawn_blocking` keeps that off the reactor; the
    // caller's `DISPATCH_TIMEOUT` still bounds how long anyone waits for it.
    let result = tokio::task::spawn_blocking(move || maidan_wasi::run(&module, &invoke, limits))
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(error = %err, "wasi handler: run task failed");
            WasiResult::fail(WasiFailureKind::Trap, "handler execution failed")
        });

    envelope(&result)
}

/// Map a [`WasiResult`] onto the slash-response shape the other handler kinds
/// use, so `/ui` and the message metadata do not need a WASI-specific branch.
///
/// The failure *kind* is carried through rather than flattened into one opaque
/// error: "you ran out of fuel" and "you trapped" are different messages to the
/// person who wrote the handler, and collapsing them is the difference between a
/// fixable report and a shrug.
fn envelope(result: &WasiResult) -> Value {
    if result.ok {
        return json!({
            "ok": true,
            "response": {
                "content": [{ "type": "text", "text": result.stdout }],
                "stderr": result.stderr,
            }
        });
    }
    json!({
        "ok": false,
        "error": result.error.clone().unwrap_or_else(|| "handler failed".into()),
        "error_kind": result.error_kind.map(|k| k.as_str()),
        "stdout": result.stdout,
        "stderr": result.stderr,
    })
}
