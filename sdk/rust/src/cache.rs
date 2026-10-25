//! Keeping Maidan's bytes in the provider's cache: the boot prefix with a cache
//! breakpoint, one cache key per shared-prefix group, and the thread id as a
//! gateway session id. Pure functions; the TypeScript, Python and Go SDKs give
//! the same output for the same input (`docs/Harness Caching.md`).

use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

/// The channel's boot prefix, byte for byte as served, and its sha256 (hex),
/// for `evidence.pack_sha256`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootPrefix {
    pub text: String,
    pub sha256: String,
}

impl BootPrefix {
    /// Hash the served boot text; `client.channels().boot(cid)` calls this.
    pub fn new(text: String) -> Self {
        let sha256 = hex(&Sha256::digest(text.as_bytes()));
        Self { text, sha256 }
    }
}

/// A cache or gateway helper given an input it cannot place.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheError(pub String);

impl fmt::Display for CacheError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CacheError {}

fn err<T>(message: impl Into<String>) -> Result<T, CacheError> {
    Err(CacheError(message.into()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Where a value goes in a provider or gateway request.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RequestFields {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<Map<String, Value>>,
}

fn body(value: Value) -> RequestFields {
    RequestFields {
        body: Some(value),
        headers: None,
    }
}

/// Place `text` (the boot prefix) as the first, cached part of a request.
/// Anthropic and Bedrock get an explicit breakpoint and take `ttl` ("5m" or
/// "1h"); OpenAI Responses gets an explicit breakpoint (GPT-5.6 and later, 30
/// minutes); the rest cache a matching prefix on their own, so the prefix is
/// only put first.
pub fn cached_prefix(provider: &str, text: &str, ttl: Option<&str>) -> Result<Value, CacheError> {
    if let Some(ttl) = ttl {
        if ttl != "5m" && ttl != "1h" {
            return err("ttl must be 5m or 1h");
        }
        if provider != "anthropic" && provider != "bedrock-converse" {
            return err(format!("{provider} takes no cache ttl"));
        }
    }
    let with_ttl = |mut point: Value| {
        if ttl == Some("1h") {
            point["ttl"] = json!("1h");
        }
        point
    };
    Ok(match provider {
        "anthropic" => json!({
            "type": "text",
            "text": text,
            "cache_control": with_ttl(json!({"type": "ephemeral"})),
        }),
        "bedrock-converse" => json!([
            {"text": text},
            {"cachePoint": with_ttl(json!({"type": "default"}))},
        ]),
        "openai-responses" => json!({
            "type": "message",
            "role": "developer",
            "content": [{
                "type": "input_text",
                "text": text,
                "prompt_cache_breakpoint": {"mode": "explicit"},
            }],
        }),
        "gemini" => json!({"parts": [{"text": text}]}),
        "openai-chat" | "deepseek" | "mistral" | "xai" | "vllm" => {
            json!({"role": "system", "content": text})
        }
        other => return err(format!("unknown provider {other}")),
    })
}

/// One key per shared-prefix group, never the same in two workspaces: the
/// workspace id is hashed in, so the same group name in two workspaces gives
/// two keys, and the key reveals neither.
pub fn cache_key(workspace_id: &str, group: &str) -> Result<String, CacheError> {
    if workspace_id.is_empty() || group.is_empty() {
        return err("workspace id and group are required");
    }
    let digest = hex(&Sha256::digest(
        format!("{workspace_id}\n{group}").as_bytes(),
    ));
    Ok(format!("maidan-{}", &digest[..32]))
}

/// Where `key` goes for a provider that takes one; empty fields for one that
/// does not.
pub fn cache_key_fields(provider: &str, key: &str) -> Result<RequestFields, CacheError> {
    Ok(match provider {
        "openai-responses" | "openai-chat" | "mistral" | "xai-responses" => {
            body(json!({"prompt_cache_key": key}))
        }
        "xai-chat" => RequestFields {
            body: None,
            headers: Some(Map::from_iter([("x-grok-conv-id".to_owned(), json!(key))])),
        },
        "deepseek" => body(json!({"user_id": key})),
        "deepseek-anthropic" => body(json!({"metadata": {"user_id": key}})),
        "vllm" => body(json!({"cache_salt": key})),
        "anthropic" | "bedrock-converse" | "gemini" => RequestFields::default(),
        other => return err(format!("unknown provider {other}")),
    })
}

fn is_uuid_v7(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
        && b[14] == b'7'
        && matches!(b[19].to_ascii_lowercase(), b'8' | b'9' | b'a' | b'b')
}

/// The thread id as the gateway's session id, so the gateway's spend joins the
/// thread's outcome. Helicone's `path` and `name` default to "/" and "maidan".
pub fn gateway_session(
    gateway: &str,
    thread_id: &str,
    path: Option<&str>,
    name: Option<&str>,
) -> Result<RequestFields, CacheError> {
    if thread_id.is_empty() {
        return err("thread id is required");
    }
    Ok(match gateway {
        "openrouter" => body(json!({"session_id": thread_id})),
        "helicone" => RequestFields {
            body: None,
            headers: Some(Map::from_iter([
                ("Helicone-Session-Id".to_owned(), json!(thread_id)),
                (
                    "Helicone-Session-Path".to_owned(),
                    json!(path.unwrap_or("/")),
                ),
                (
                    "Helicone-Session-Name".to_owned(),
                    json!(name.unwrap_or("maidan")),
                ),
            ])),
        },
        "litellm" => body(json!({"litellm_session_id": thread_id})),
        "tensorzero" | "tensorzero-native" => {
            if !is_uuid_v7(thread_id) {
                return err("TensorZero takes a UUIDv7 episode id; this thread id is not one");
            }
            let field = if gateway == "tensorzero" {
                "tensorzero::episode_id"
            } else {
                "episode_id"
            };
            body(json!({ field: thread_id }))
        }
        other => return err(format!("unknown gateway {other}")),
    })
}

/// Affinity headers for a self-hosted KV-aware router, from thread and claim
/// structure. Every router keys a child by its parent, so a child lands where its parent's
/// prefix is warm: Dynamo, llm-d, AIBrix and SGLang all get the parent's thread
/// as their one affinity key when there is one. Dynamo's docs say affinity uses
/// `X-Dynamo-Session-ID` alone (with `--router-session-affinity-ttl-secs` set)
/// and `X-Dynamo-Parent-Session-ID` is trace metadata, so a child sends its
/// parent's thread as the session and no parent header (its own id is not a
/// parent). AIBrix also gets `routing-strategy: session-affinity`. SGLang caps
/// the key at 128 bytes and AIBrix at 256; a longer key is an error. Hints only:
/// no router accepts a prefix hash from a client.
pub fn routing_hints(
    router: &str,
    thread_id: &str,
    parent_thread_id: Option<&str>,
) -> Result<RequestFields, CacheError> {
    if thread_id.is_empty() {
        return err("thread id is required");
    }
    let affinity = parent_thread_id
        .filter(|p| !p.is_empty())
        .unwrap_or(thread_id);
    let one = |name: &str, max: usize| {
        if affinity.len() > max {
            return err(format!("routing key is longer than {max} bytes"));
        }
        Ok(Map::from_iter([(name.to_owned(), json!(affinity))]))
    };
    let headers = match router {
        "dynamo" => one("X-Dynamo-Session-ID", usize::MAX)?,
        "llm-d" => one("x-session-id", usize::MAX)?,
        "aibrix" => {
            let mut h = one("x-aibrix-session-key", 256)?;
            h.insert("routing-strategy".to_owned(), json!("session-affinity"));
            h
        }
        "sglang" => one("X-SMG-Routing-Key", 128)?,
        other => return err(format!("unknown router {other}")),
    };
    Ok(RequestFields {
        body: None,
        headers: Some(headers),
    })
}

/// The shortest `salt_key` [`cache_salt`] accepts, in bytes.
pub const MIN_SALT_KEY_BYTES: usize = 32;

/// vLLM's `cache_salt` for one workspace: prefix blocks are shared inside the
/// workspace and never across two. The salt is
/// `HMAC-SHA256(salt_key, "cache_salt\n" + workspace_id)` as unpadded base64url
/// (43 characters), so it cannot be computed from the workspace id alone.
///
/// Set it from a trusted server-side component that holds `salt_key` (at least
/// 32 bytes), never from an end client. It is a cache partition, not an auth
/// token: it decides which requests may share prefix blocks, nothing more.
pub fn cache_salt(workspace_id: &str, salt_key: &[u8]) -> Result<RequestFields, CacheError> {
    if workspace_id.is_empty() {
        return err("workspace id is required");
    }
    if salt_key.len() < MIN_SALT_KEY_BYTES {
        return err(format!(
            "salt key must be at least {MIN_SALT_KEY_BYTES} bytes"
        ));
    }
    let mac = hmac_sha256(salt_key, format!("cache_salt\n{workspace_id}").as_bytes());
    Ok(body(json!({"cache_salt": base64url(&mac)})))
}

/// RFC 2104 HMAC over SHA-256, on the `sha2` the crate already depends on.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| k.map(|b| b ^ byte);
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// RFC 4648 base64url without padding.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | u32::from(b) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(char::from(ALPHABET[(n >> (18 - 6 * i) & 63) as usize]));
        }
    }
    out
}
