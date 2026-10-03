//! Cache hints (SEP-2549). MCP `2026-07-28` requires a `ttlMs` and a
//! `cacheScope` on every result of `server/discover`, `tools/list`,
//! `prompts/list`, `resources/list`, `resources/templates/list` and
//! `resources/read`, and the official TypeScript client caches on them by
//! default. The hint for each is chosen here and nowhere else; the cache-hint
//! table in `docs/Protocols.md` is this module written out, and a contract test
//! holds the two together.
//!
//! `cacheScope` follows tenancy. `"public"` lets a shared gateway hand the
//! result to any caller, so it is used only for a result that is the same bytes
//! whoever asks. Anything derived from the caller's token is `"private"`. A hint
//! never authorizes anything: every call is checked against its token whatever a
//! client has cached.

use serde_json::Value;

/// Who may reuse a cached result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CacheScope {
    /// The same for every caller; any cache, a shared one included, may serve it.
    Public,
    /// Reusable only within the authorization context that fetched it.
    Private,
}

impl CacheScope {
    pub fn as_str(self) -> &'static str {
        match self {
            CacheScope::Public => "public",
            CacheScope::Private => "private",
        }
    }
}

/// A result's freshness (`ttlMs`) and who may reuse it (`cacheScope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CacheHint {
    pub ttl_ms: u64,
    pub scope: CacheScope,
}

/// What a release serves and only a release changes: the catalog is compiled
/// in, and a token's capabilities are fixed when it is minted, so nothing
/// changes these results while a release runs. The TTL is therefore the bound
/// on how long a client keeps a pre-deploy answer after a deploy, which Maidan
/// cannot announce: no stream survives the restart, so `list_changed` never
/// fires. Refetching costs one request and returns the same bytes, so an hour
/// buys nearly all of the saving at a twenty-fourth of the staleness of the
/// TypeScript client's cap.
pub const RELEASE_TTL_MS: u64 = 60 * 60 * 1000;

/// The longest TTL the official TypeScript client honours (`MAX_CACHE_TTL_MS`,
/// 24 h). A content-addressed URI names its bytes, so
/// nothing shorter is needed.
pub const CONTENT_ADDRESSED_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// Workspace and channel records change on a rename, a topic edit or an
/// archive, which are rare. A subscriber hears `notifications/resources/updated`
/// and drops its copy at once; anyone else is at most a minute behind.
pub const RECORD_TTL_MS: u64 = 60 * 1000;

pub const DISCOVER: CacheHint = CacheHint {
    ttl_ms: RELEASE_TTL_MS,
    scope: CacheScope::Public,
};

/// The full catalog is filtered to what the token may call, so two tokens get
/// two lists.
pub const TOOLS_LIST: CacheHint = CacheHint {
    ttl_ms: RELEASE_TTL_MS,
    scope: CacheScope::Private,
};

pub const PROMPTS_LIST: CacheHint = CacheHint {
    ttl_ms: RELEASE_TTL_MS,
    scope: CacheScope::Public,
};

pub const RESOURCE_TEMPLATES_LIST: CacheHint = CacheHint {
    ttl_ms: RELEASE_TTL_MS,
    scope: CacheScope::Public,
};

/// Lists the caller's own workspace, which its token fixes.
pub const RESOURCES_LIST: CacheHint = CacheHint {
    ttl_ms: RELEASE_TTL_MS,
    scope: CacheScope::Private,
};

/// The hint for a `resources/read` of `uri`. Every read is private: access is
/// checked per caller, and an artifact's record carries the caller's own
/// filename for the shared bytes.
pub fn resource_read(uri: &str) -> CacheHint {
    let kind = uri
        .strip_prefix("maidan://")
        .and_then(|rest| rest.split('/').next())
        .unwrap_or("");
    let ttl_ms = match kind {
        "artifacts" => CONTENT_ADDRESSED_TTL_MS,
        "workspaces" | "channels" => RECORD_TTL_MS,
        // A thread's transcript changes with every post, claim and transition.
        _ => 0,
    };
    CacheHint {
        ttl_ms,
        scope: CacheScope::Private,
    }
}

/// Put `hint` on `result`.
pub fn with_hint(mut result: Value, hint: CacheHint) -> Value {
    if let Some(object) = result.as_object_mut() {
        object.insert("ttlMs".into(), hint.ttl_ms.into());
        object.insert("cacheScope".into(), hint.scope.as_str().into());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_content_addressed_read_is_fresh_for_the_longest_ttl_clients_honour() {
        let hint = resource_read(&format!("maidan://artifacts/{}", "a".repeat(64)));
        assert_eq!(hint.ttl_ms, CONTENT_ADDRESSED_TTL_MS);
        assert_eq!(hint.scope, CacheScope::Private);
    }

    #[test]
    fn a_thread_read_is_stale_at_once() {
        let hint = resource_read("maidan://threads/0198d7a4-0000-7000-8000-000000000000");
        assert_eq!(hint.ttl_ms, 0);
        assert_eq!(hint.scope, CacheScope::Private);
    }

    #[test]
    fn workspace_and_channel_records_get_a_short_ttl() {
        for uri in [
            "maidan://workspaces/0198d7a4-0000-7000-8000-000000000000",
            "maidan://channels/0198d7a4-0000-7000-8000-000000000000",
        ] {
            assert_eq!(resource_read(uri).ttl_ms, RECORD_TTL_MS, "{uri}");
        }
    }

    #[test]
    fn a_hint_is_written_as_the_two_spec_fields() {
        let hinted = with_hint(json!({ "tools": [] }), TOOLS_LIST);
        assert_eq!(hinted["ttlMs"], json!(RELEASE_TTL_MS));
        assert_eq!(hinted["cacheScope"], json!("private"));
        assert_eq!(hinted["tools"], json!([]));
    }
}
