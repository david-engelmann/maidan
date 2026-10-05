// Keeping Maidan's bytes in the provider's cache: the boot prefix with a cache
// breakpoint, one cache key per shared-prefix group, and the thread id as a
// gateway session id. Pure functions; the TypeScript, Python and Rust SDKs
// give the same output for the same input (docs/Harness Caching.md).

package maidan

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"regexp"
)

// BootPrefix is the channel's boot prefix, byte for byte as served, and its
// sha256 (hex), for evidence.pack_sha256.
type BootPrefix struct {
	Text   string `json:"text"`
	SHA256 string `json:"sha256"`
}

// NewBootPrefix hashes the served boot bytes; Channels.Boot calls it.
func NewBootPrefix(b []byte) BootPrefix {
	sum := sha256.Sum256(b)
	return BootPrefix{Text: string(b), SHA256: hex.EncodeToString(sum[:])}
}

// CacheError is a cache or gateway helper given an input it cannot place.
type CacheError struct{ Msg string }

func (e *CacheError) Error() string { return e.Msg }

func cacheErr(format string, args ...any) error {
	return &CacheError{Msg: fmt.Sprintf(format, args...)}
}

// CachedPrefix places text (the boot prefix) as the first, cached part of a
// request. Anthropic and Bedrock get an explicit breakpoint and take ttl ("5m"
// or "1h"; "" for the default); OpenAI Responses gets an explicit breakpoint
// (GPT-5.6 and later, 30 minutes); the rest cache a matching prefix on their
// own, so the prefix is only put first. The result marshals to the provider's
// JSON.
func CachedPrefix(provider, text, ttl string) (any, error) {
	if ttl != "" && ttl != "5m" && ttl != "1h" {
		return nil, cacheErr("ttl must be 5m or 1h")
	}
	if ttl != "" && provider != "anthropic" && provider != "bedrock-converse" {
		return nil, cacheErr("%s takes no cache ttl", provider)
	}
	withTTL := func(m M) M {
		if ttl == "1h" {
			m["ttl"] = "1h"
		}
		return m
	}
	switch provider {
	case "anthropic":
		return M{"type": "text", "text": text, "cache_control": withTTL(M{"type": "ephemeral"})}, nil
	case "bedrock-converse":
		return []any{M{"text": text}, M{"cachePoint": withTTL(M{"type": "default"})}}, nil
	case "openai-responses":
		return M{"type": "message", "role": "developer", "content": []any{
			M{"type": "input_text", "text": text, "prompt_cache_breakpoint": M{"mode": "explicit"}},
		}}, nil
	case "gemini":
		return M{"parts": []any{M{"text": text}}}, nil
	case "openai-chat", "deepseek", "mistral", "xai", "vllm":
		return M{"role": "system", "content": text}, nil
	}
	return nil, cacheErr("unknown provider %s", provider)
}

// CacheKey is one key per shared-prefix group, never the same in two
// workspaces: the workspace id is hashed in, so the same group name in two
// workspaces gives two keys, and the key reveals neither.
func CacheKey(workspaceID, group string) (string, error) {
	if workspaceID == "" || group == "" {
		return "", cacheErr("workspace id and group are required")
	}
	sum := sha256.Sum256([]byte(workspaceID + "\n" + group))
	return "maidan-" + hex.EncodeToString(sum[:])[:32], nil
}

// RequestFields says where a value goes in a provider or gateway request.
type RequestFields struct {
	Body    M                 `json:"body,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
}

// CacheKeyFields is where key goes for a provider that takes one, and empty
// fields for one that does not.
func CacheKeyFields(provider, key string) (RequestFields, error) {
	switch provider {
	case "openai-responses", "openai-chat", "mistral", "xai-responses":
		return RequestFields{Body: M{"prompt_cache_key": key}}, nil
	case "xai-chat":
		return RequestFields{Headers: map[string]string{"x-grok-conv-id": key}}, nil
	case "deepseek":
		return RequestFields{Body: M{"user_id": key}}, nil
	case "deepseek-anthropic":
		return RequestFields{Body: M{"metadata": M{"user_id": key}}}, nil
	case "vllm":
		return RequestFields{Body: M{"cache_salt": key}}, nil
	case "anthropic", "bedrock-converse", "gemini":
		return RequestFields{}, nil
	}
	return RequestFields{}, cacheErr("unknown provider %s", provider)
}

var uuidV7 = regexp.MustCompile(`^(?i)[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$`)

// GatewaySession is the thread id as the gateway's session id, so the
// gateway's spend joins the thread's outcome. Helicone's path and name default
// to "/" and "maidan" when empty.
func GatewaySession(gateway, threadID, path, name string) (RequestFields, error) {
	if threadID == "" {
		return RequestFields{}, cacheErr("thread id is required")
	}
	if path == "" {
		path = "/"
	}
	if name == "" {
		name = "maidan"
	}
	switch gateway {
	case "openrouter":
		return RequestFields{Body: M{"session_id": threadID}}, nil
	case "helicone":
		return RequestFields{Headers: map[string]string{
			"Helicone-Session-Id":   threadID,
			"Helicone-Session-Path": path,
			"Helicone-Session-Name": name,
		}}, nil
	case "litellm":
		return RequestFields{Body: M{"litellm_session_id": threadID}}, nil
	case "tensorzero", "tensorzero-native":
		if !uuidV7.MatchString(threadID) {
			return RequestFields{}, cacheErr("TensorZero takes a UUIDv7 episode id; this thread id is not one")
		}
		field := "episode_id"
		if gateway == "tensorzero" {
			field = "tensorzero::episode_id"
		}
		return RequestFields{Body: M{field: threadID}}, nil
	}
	return RequestFields{}, cacheErr("unknown gateway %s", gateway)
}
