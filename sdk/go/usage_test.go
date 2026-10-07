// The usage normalizers against the shared fixtures in sdk/usage-fixtures/,
// which the other SDKs and the server's ledger read too (no server needed).
package maidan

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"testing"
)

type usageFixture struct {
	Name     string          `json:"name"`
	Provider string          `json:"provider"`
	Recorded string          `json:"recorded"`
	Response json.RawMessage `json:"response"`
	Options  struct {
		Model    string `json:"model"`
		Provider string `json:"provider"`
	} `json:"options"`
	PriceSnapshot PriceSnapshot `json:"price_snapshot"`
	Expected      struct {
		NormalizedUsage
		USDMicros *int64 `json:"usd_micros"`
	} `json:"expected"`
	ExpectedError string `json:"expected_error"`
}

func loadUsageFixtures(t *testing.T, dir string) []usageFixture {
	t.Helper()
	paths, err := filepath.Glob(filepath.Join("..", "usage-fixtures", dir, "*.json"))
	if err != nil || len(paths) == 0 {
		t.Fatalf("no fixtures in %s: %v", dir, err)
	}
	sort.Strings(paths)
	out := make([]usageFixture, 0, len(paths))
	for _, p := range paths {
		raw, err := os.ReadFile(p)
		if err != nil {
			t.Fatal(err)
		}
		var f usageFixture
		dec := json.NewDecoder(strings.NewReader(string(raw)))
		if err := dec.Decode(&f); err != nil {
			t.Fatalf("%s: %v", p, err)
		}
		out = append(out, f)
	}
	return out
}

func normalizeFixture(f usageFixture) (NormalizedUsage, error) {
	return NormalizeUsage(f.Provider, f.Response, UsageOptions{Model: f.Options.Model, Provider: f.Options.Provider})
}

func TestNormalizeUsageReadsEveryProviderFixture(t *testing.T) {
	fixtures := loadUsageFixtures(t, "providers")
	constructed := loadUsageFixtures(t, "constructed")
	for _, f := range fixtures {
		if f.Recorded != "verbatim" && f.Recorded != "transcribed" {
			t.Errorf("%s: a provider fixture is a recorded response, not %q", f.Name, f.Recorded)
		}
	}
	for _, f := range constructed {
		if f.Recorded != "constructed" {
			t.Errorf("%s: a constructed case says so, not %q", f.Name, f.Recorded)
		}
	}
	for _, f := range append(append([]usageFixture{}, fixtures...), constructed...) {
		t.Run(f.Name, func(t *testing.T) {
			got, err := normalizeFixture(f)
			if err != nil {
				t.Fatal(err)
			}
			if got != f.Expected.NormalizedUsage {
				t.Fatalf("got %+v\nwant %+v", got, f.Expected.NormalizedUsage)
			}
			if f.Expected.USDMicros != nil {
				usd, err := USDMicros(got.Tokens, f.PriceSnapshot)
				if err != nil || usd != *f.Expected.USDMicros {
					t.Fatalf("usd_micros %d (%v), want %d", usd, err, *f.Expected.USDMicros)
				}
			}
		})
	}
	for _, p := range UsageProviders {
		found := false
		for _, f := range fixtures {
			found = found || f.Provider == p
		}
		if !found {
			t.Errorf("no recorded fixture for %s", p)
		}
	}
}

func TestNormalizeUsageRefusesEveryInvalidFixture(t *testing.T) {
	for _, f := range loadUsageFixtures(t, "invalid") {
		t.Run(f.Name, func(t *testing.T) {
			_, err := normalizeFixture(f)
			var ue *UsageError
			if !errors.As(err, &ue) || !strings.Contains(err.Error(), f.ExpectedError) {
				t.Fatalf("got %v, want a UsageError containing %q", err, f.ExpectedError)
			}
		})
	}
}

func TestUSDMicrosPricesEveryChargeFixtureAsTheLedgerDoes(t *testing.T) {
	raw, err := os.ReadFile(filepath.Join("..", "usage-fixtures", "charges.json"))
	if err != nil {
		t.Fatal(err)
	}
	var charges struct {
		Cases []struct {
			Name          string        `json:"name"`
			Tokens        TokenUsage    `json:"tokens"`
			PriceSnapshot PriceSnapshot `json:"price_snapshot"`
			USDMicros     int64         `json:"usd_micros"`
		} `json:"cases"`
	}
	if err := json.Unmarshal(raw, &charges); err != nil {
		t.Fatal(err)
	}
	for _, c := range charges.Cases {
		got, err := USDMicros(c.Tokens, c.PriceSnapshot)
		if err != nil || got != c.USDMicros {
			t.Errorf("%s: got %d (%v), want %d", c.Name, got, err, c.USDMicros)
		}
	}
}

func TestNormalizeUsageNeedsAModelWhenTheResponseNamesNone(t *testing.T) {
	response := []byte(`{"usage":{"input_tokens":1,"output_tokens":1}}`)
	if _, err := NormalizeUsage("anthropic", response, UsageOptions{}); err == nil || !strings.Contains(err.Error(), "model is required") {
		t.Fatalf("got %v", err)
	}
	got, err := NormalizeUsage("anthropic", response, UsageOptions{Model: "m"})
	if err != nil || got.Model != "m" {
		t.Fatalf("got %+v, %v", got, err)
	}
}

func TestNormalizeUsageRefusesBadCountsAndUnknownProviders(t *testing.T) {
	if _, err := NormalizeUsage("nope", []byte(`{}`), UsageOptions{}); err == nil || !strings.Contains(err.Error(), "unknown provider") {
		t.Fatalf("got %v", err)
	}
	for _, bad := range []string{"-1", "1.5", `"7"`, "true"} {
		response := []byte(`{"model":"m","usage":{"prompt_tokens":` + bad + `}}`)
		_, err := NormalizeUsage("openai-chat", response, UsageOptions{})
		if err == nil || !strings.Contains(err.Error(), "prompt_tokens must be a non-negative integer") {
			t.Errorf("%s: got %v", bad, err)
		}
	}
}

func TestNormalizeUsageEvidenceProviderCanBeOverridden(t *testing.T) {
	response := []byte(`{"model":"m","usage":{"prompt_tokens":3,"completion_tokens":1}}`)
	got, err := NormalizeUsage("openai-chat", response, UsageOptions{Provider: "azure.ai.openai"})
	if err != nil || got.Evidence.Provider != "azure.ai.openai" {
		t.Fatalf("got %+v, %v", got, err)
	}
}
