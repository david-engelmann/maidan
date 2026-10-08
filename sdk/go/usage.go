// Provider usage objects -> the ledger's report_usage shape.
//
// The ledger's input is uncached input on every provider, and cache writes are
// two tiers, 5-minute and 1-hour. Providers disagree on both, so each reader
// below says where its numbers come from. The TypeScript, Python and Rust SDKs
// and the server's own reader agree on the fixtures in sdk/usage-fixtures/.

package maidan

import (
	"bytes"
	"encoding/json"
	"fmt"
	"math"
	"math/big"
	"strings"
)

// TokenUsage is the ledger's token tiers. Input is uncached input.
type TokenUsage struct {
	Input        int64 `json:"input"`
	Output       int64 `json:"output"`
	CacheRead    int64 `json:"cache_read"`
	CacheWrite5m int64 `json:"cache_write_5m"`
	CacheWrite1h int64 `json:"cache_write_1h"`
}

// PriceSnapshot is micro-USD per million tokens, one rate per tier.
type PriceSnapshot struct {
	InputUSDMicrosPerMillion        int64 `json:"input_usd_micros_per_million"`
	OutputUSDMicrosPerMillion       int64 `json:"output_usd_micros_per_million"`
	CacheReadUSDMicrosPerMillion    int64 `json:"cache_read_usd_micros_per_million"`
	CacheWrite5mUSDMicrosPerMillion int64 `json:"cache_write_5m_usd_micros_per_million"`
	CacheWrite1hUSDMicrosPerMillion int64 `json:"cache_write_1h_usd_micros_per_million"`
}

// UsageEvidence is what a normalizer can tell from the response; the rest of
// the ledger's evidence (harness, cache key, packs) is the caller's.
type UsageEvidence struct {
	Provider        string `json:"provider"`
	ServiceTier     string `json:"service_tier,omitempty"`
	CacheMissReason string `json:"cache_miss_reason,omitempty"`
}

// NormalizedUsage is the economic part of a report_usage body.
type NormalizedUsage struct {
	Model    string        `json:"model"`
	Tokens   TokenUsage    `json:"tokens"`
	Evidence UsageEvidence `json:"evidence"`
}

// UsageOptions fills what a response does not say. Model names the model when
// the response does not (Bedrock Converse); Provider overrides the evidence
// provider name (a Chat Completions shape served by Azure).
type UsageOptions struct {
	Model    string
	Provider string
}

// UsageError is a usage object that cannot be read into the ledger's shape.
type UsageError struct{ Msg string }

func (e *UsageError) Error() string { return e.Msg }

func usageErr(format string, args ...any) error {
	return &UsageError{Msg: fmt.Sprintf(format, args...)}
}

type obj = map[string]any

func at(o any, path string) any {
	v := o
	for _, key := range strings.Split(path, ".") {
		m, ok := v.(obj)
		if !ok {
			return nil
		}
		v = m[key]
	}
	return v
}

func count(o any, path string) (int64, error) {
	v := at(o, path)
	if v == nil {
		return 0, nil
	}
	n, ok := v.(json.Number)
	if !ok {
		return 0, usageErr("%s must be a non-negative integer", path)
	}
	i, err := n.Int64()
	if err != nil || i < 0 {
		return 0, usageErr("%s must be a non-negative integer", path)
	}
	return i, nil
}

func text(o any, path string) string {
	s, _ := at(o, path).(string)
	return strings.TrimSpace(s)
}

func add(parts ...int64) (int64, error) {
	var total int64
	for _, p := range parts {
		if p > math.MaxInt64-total {
			return 0, usageErr("token total overflow")
		}
		total += p
	}
	return total, nil
}

// uncached: OpenAI, Gemini, Mistral, xAI and vLLM count cached tokens inside
// the total.
func uncached(total int64, cached ...int64) (int64, error) {
	c, err := add(cached...)
	if err != nil {
		return 0, err
	}
	if total < c {
		return 0, usageErr("input_tokens smaller than the cache tiers it includes")
	}
	return total - c, nil
}

// counts reads several paths, stopping at the first bad one.
func counts(o any, paths ...string) ([]int64, error) {
	out := make([]int64, len(paths))
	for i, p := range paths {
		n, err := count(o, p)
		if err != nil {
			return nil, err
		}
		out[i] = n
	}
	return out, nil
}

func block(r obj, key string) (obj, error) {
	u, ok := r[key].(obj)
	if !ok {
		return nil, usageErr("response has no %s object", key)
	}
	return u, nil
}

type reading struct {
	tokens          TokenUsage
	model           string
	serviceTier     string
	cacheMissReason string
}

// bedrockDetails reads Bedrock Converse's cacheDetails, its writes by TTL.
func bedrockDetails(u obj) (int64, int64, error) {
	raw, present := u["cacheDetails"]
	if !present || raw == nil {
		return 0, 0, nil
	}
	details, ok := raw.([]any)
	if !ok {
		return 0, 0, usageErr("cacheDetails must be an array")
	}
	var five, hour int64
	for _, d := range details {
		detail, ok := d.(obj)
		if !ok {
			return 0, 0, usageErr("cacheDetails entries must be objects")
		}
		tokens, err := count(detail, "inputTokens")
		if err != nil {
			return 0, 0, err
		}
		switch detail["ttl"] {
		case "5m":
			five, err = add(five, tokens)
		case "1h":
			hour, err = add(hour, tokens)
		default:
			return 0, 0, usageErr("cacheDetails ttl must be 5m or 1h")
		}
		if err != nil {
			return 0, 0, err
		}
	}
	return five, hour, nil
}

func chatShape(u obj, reasoningIsExtra bool) (TokenUsage, error) {
	return inclusiveShape(u, "prompt_tokens", "prompt_tokens_details", "completion_tokens",
		"completion_tokens_details", reasoningIsExtra)
}

func responsesShape(u obj, reasoningIsExtra bool) (TokenUsage, error) {
	return inclusiveShape(u, "input_tokens", "input_tokens_details", "output_tokens",
		"output_tokens_details", reasoningIsExtra)
}

func inclusiveShape(u obj, total, inDetails, out, outDetails string, reasoningIsExtra bool) (TokenUsage, error) {
	n, err := counts(u, total, inDetails+".cached_tokens", inDetails+".cache_write_tokens", out)
	if err != nil {
		return TokenUsage{}, err
	}
	var reasoning int64
	if reasoningIsExtra {
		if reasoning, err = count(u, outDetails+".reasoning_tokens"); err != nil {
			return TokenUsage{}, err
		}
	}
	input, err := uncached(n[0], n[1], n[2])
	if err != nil {
		return TokenUsage{}, err
	}
	output, err := add(n[3], reasoning)
	if err != nil {
		return TokenUsage{}, err
	}
	return TokenUsage{Input: input, Output: output, CacheRead: n[1], CacheWrite5m: n[2]}, nil
}

func readAnthropic(r obj) (reading, error) {
	u, err := block(r, "usage")
	if err != nil {
		return reading{}, err
	}
	n, err := counts(u, "input_tokens", "output_tokens", "cache_read_input_tokens",
		"cache_creation.ephemeral_5m_input_tokens", "cache_creation.ephemeral_1h_input_tokens",
		"cache_creation_input_tokens")
	if err != nil {
		return reading{}, err
	}
	five, hour := n[3], n[4]
	if five == 0 && hour == 0 {
		five = n[5]
	}
	return reading{
		tokens:          TokenUsage{Input: n[0], Output: n[1], CacheRead: n[2], CacheWrite5m: five, CacheWrite1h: hour},
		model:           text(r, "model"),
		serviceTier:     text(u, "service_tier"),
		cacheMissReason: text(r, "diagnostics.cache_miss_reason.type"),
	}, nil
}

func readBedrock(r obj) (reading, error) {
	u, err := block(r, "usage")
	if err != nil {
		return reading{}, err
	}
	five, hour, err := bedrockDetails(u)
	if err != nil {
		return reading{}, err
	}
	n, err := counts(u, "inputTokens", "outputTokens", "cacheReadInputTokens", "cacheWriteInputTokens")
	if err != nil {
		return reading{}, err
	}
	if five == 0 && hour == 0 {
		five = n[3]
	}
	return reading{tokens: TokenUsage{Input: n[0], Output: n[1], CacheRead: n[2], CacheWrite5m: five, CacheWrite1h: hour}}, nil
}

func readOpenAI(shape func(obj, bool) (TokenUsage, error)) func(obj) (reading, error) {
	return func(r obj) (reading, error) {
		u, err := block(r, "usage")
		if err != nil {
			return reading{}, err
		}
		t, err := shape(u, false)
		return reading{tokens: t, model: text(r, "model"), serviceTier: text(r, "service_tier")}, err
	}
}

func readGemini(r obj) (reading, error) {
	u, err := block(r, "usageMetadata")
	if err != nil {
		return reading{}, err
	}
	n, err := counts(u, "promptTokenCount", "cachedContentTokenCount", "candidatesTokenCount", "thoughtsTokenCount")
	if err != nil {
		return reading{}, err
	}
	input, err := uncached(n[0], n[1])
	if err != nil {
		return reading{}, err
	}
	output, err := add(n[2], n[3])
	if err != nil {
		return reading{}, err
	}
	return reading{
		tokens:      TokenUsage{Input: input, Output: output, CacheRead: n[1]},
		model:       text(r, "modelVersion"),
		serviceTier: text(u, "serviceTier"),
	}, nil
}

func readDeepSeek(r obj) (reading, error) {
	u, err := block(r, "usage")
	if err != nil {
		return reading{}, err
	}
	if u["prompt_cache_miss_tokens"] == nil {
		return reading{}, usageErr("prompt_cache_miss_tokens is required")
	}
	n, err := counts(u, "prompt_cache_miss_tokens", "completion_tokens", "prompt_cache_hit_tokens")
	if err != nil {
		return reading{}, err
	}
	return reading{tokens: TokenUsage{Input: n[0], Output: n[1], CacheRead: n[2]}, model: text(r, "model")}, nil
}

func readPlainChat(r obj) (reading, error) {
	u, err := block(r, "usage")
	if err != nil {
		return reading{}, err
	}
	t, err := chatShape(u, false)
	return reading{tokens: t, model: text(r, "model")}, err
}

func readXAI(r obj) (reading, error) {
	u, err := block(r, "usage")
	if err != nil {
		return reading{}, err
	}
	shape := chatShape
	if _, ok := u["input_tokens"]; ok {
		shape = responsesShape
	}
	t, err := shape(u, true)
	return reading{tokens: t, model: text(r, "model")}, err
}

type usageReader struct {
	name string
	read func(obj) (reading, error)
}

var usageReaders = map[string]usageReader{
	"anthropic":        {"anthropic", readAnthropic},
	"bedrock-converse": {"aws.bedrock", readBedrock},
	"openai-responses": {"openai", readOpenAI(responsesShape)},
	"openai-chat":      {"openai", readOpenAI(chatShape)},
	"gemini":           {"gcp.gemini", readGemini},
	"deepseek":         {"deepseek", readDeepSeek},
	"mistral":          {"mistral_ai", readPlainChat},
	"xai":              {"x_ai", readXAI},
	"vllm":             {"vllm", readPlainChat},
}

// UsageProviders lists the provider ids NormalizeUsage accepts.
var UsageProviders = []string{
	"anthropic", "bedrock-converse", "openai-responses", "openai-chat",
	"gemini", "deepseek", "mistral", "xai", "vllm",
}

// NormalizeUsage turns one provider response body into the economic part of a
// report_usage body: the model, the ledger's tokens, and the evidence the
// response carries.
func NormalizeUsage(provider string, response []byte, opts UsageOptions) (NormalizedUsage, error) {
	reader, ok := usageReaders[provider]
	if !ok {
		return NormalizedUsage{}, usageErr("unknown provider %s", provider)
	}
	dec := json.NewDecoder(bytes.NewReader(response))
	dec.UseNumber()
	var r obj
	if err := dec.Decode(&r); err != nil || r == nil {
		return NormalizedUsage{}, usageErr("response must be a JSON object")
	}
	got, err := reader.read(r)
	if err != nil {
		return NormalizedUsage{}, err
	}
	model := got.model
	if opts.Model != "" {
		model = strings.TrimSpace(opts.Model)
	}
	if model == "" {
		return NormalizedUsage{}, usageErr("model is required: the response names none, so pass UsageOptions.Model")
	}
	evidence := UsageEvidence{Provider: reader.name, ServiceTier: got.serviceTier, CacheMissReason: got.cacheMissReason}
	if opts.Provider != "" {
		evidence.Provider = opts.Provider
	}
	return NormalizedUsage{Model: model, Tokens: got.tokens, Evidence: evidence}, nil
}

// USDMicros is usd_micros as the ledger checks it: the sum of tokens times
// micro-USD per million, divided by a million and rounded up, in integers.
func USDMicros(t TokenUsage, p PriceSnapshot) (int64, error) {
	pairs := [][2]int64{
		{t.Input, p.InputUSDMicrosPerMillion},
		{t.Output, p.OutputUSDMicrosPerMillion},
		{t.CacheRead, p.CacheReadUSDMicrosPerMillion},
		{t.CacheWrite5m, p.CacheWrite5mUSDMicrosPerMillion},
		{t.CacheWrite1h, p.CacheWrite1hUSDMicrosPerMillion},
	}
	sum := new(big.Int)
	for _, pr := range pairs {
		if pr[0] < 0 || pr[1] < 0 {
			return 0, usageErr("token counts and rates must be non-negative")
		}
		sum.Add(sum, new(big.Int).Mul(big.NewInt(pr[0]), big.NewInt(pr[1])))
	}
	sum.Add(sum, big.NewInt(999_999))
	sum.Quo(sum, big.NewInt(1_000_000))
	if !sum.IsInt64() {
		return 0, usageErr("usd_micros overflow")
	}
	return sum.Int64(), nil
}
