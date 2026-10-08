// The cache helpers against the shared cases in sdk/cache-fixtures/, which the
// other SDKs read too (no server needed).
package maidan

import (
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"reflect"
	"strings"
	"testing"
)

type cacheCase struct {
	Provider      string          `json:"provider"`
	Gateway       string          `json:"gateway"`
	Text          string          `json:"text"`
	TTL           string          `json:"ttl"`
	WorkspaceID   string          `json:"workspace_id"`
	Group         string          `json:"group"`
	ThreadID      string          `json:"thread_id"`
	Expected      json.RawMessage `json:"expected"`
	ExpectedError string          `json:"expected_error"`
}

type cacheCases struct {
	Boot           BootPrefix  `json:"boot"`
	CachedPrefix   []cacheCase `json:"cached_prefix"`
	CacheKey       []cacheCase `json:"cache_key"`
	CacheKeyFields []cacheCase `json:"cache_key_fields"`
	GatewaySession []cacheCase `json:"gateway_session"`
}

func loadCacheCases(t *testing.T) cacheCases {
	t.Helper()
	raw, err := os.ReadFile(filepath.Join("..", "cache-fixtures", "cases.json"))
	if err != nil {
		t.Fatal(err)
	}
	var c cacheCases
	if err := json.Unmarshal(raw, &c); err != nil {
		t.Fatal(err)
	}
	return c
}

// sameJSON compares what got marshals to with the expected JSON.
func sameJSON(t *testing.T, label string, got any, want json.RawMessage) {
	t.Helper()
	b, err := json.Marshal(got)
	if err != nil {
		t.Fatal(err)
	}
	var g, w any
	if err := json.Unmarshal(b, &g); err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(want, &w); err != nil {
		t.Fatal(err)
	}
	if !reflect.DeepEqual(g, w) {
		t.Errorf("%s: got %s, want %s", label, b, want)
	}
}

func wantCacheError(t *testing.T, label string, err error, needle string) {
	t.Helper()
	var ce *CacheError
	if !errors.As(err, &ce) || !strings.Contains(err.Error(), needle) {
		t.Errorf("%s: got %v, want a CacheError containing %q", label, err, needle)
	}
}

func TestBootPrefixKeepsTheServedBytesAndHashesThem(t *testing.T) {
	c := loadCacheCases(t)
	if got := NewBootPrefix([]byte(c.Boot.Text)); got != c.Boot {
		t.Fatalf("got %+v, want %+v", got, c.Boot)
	}
}

func TestCachedPrefixPlacesEachProvidersBreakpoint(t *testing.T) {
	for _, c := range loadCacheCases(t).CachedPrefix {
		got, err := CachedPrefix(c.Provider, c.Text, c.TTL)
		if c.ExpectedError != "" {
			wantCacheError(t, c.Provider, err, c.ExpectedError)
			continue
		}
		if err != nil {
			t.Fatalf("%s: %v", c.Provider, err)
		}
		sameJSON(t, c.Provider, got, c.Expected)
	}
}

func TestCacheKeyIsPerWorkspaceAndGroup(t *testing.T) {
	cases := loadCacheCases(t).CacheKey
	for _, c := range cases {
		got, err := CacheKey(c.WorkspaceID, c.Group)
		var want string
		_ = json.Unmarshal(c.Expected, &want)
		if err != nil || got != want {
			t.Errorf("got %q (%v), want %q", got, err, want)
		}
	}
	if cases[0].Group != cases[1].Group || string(cases[0].Expected) == string(cases[1].Expected) {
		t.Fatal("two workspaces must not share a key for the same group")
	}
	if _, err := CacheKey("", "g"); err == nil {
		t.Fatal("an empty workspace id is refused")
	}
}

func TestCacheKeyFieldsGoWhereEachProviderReadsThem(t *testing.T) {
	for _, c := range loadCacheCases(t).CacheKeyFields {
		got, err := CacheKeyFields(c.Provider, "K")
		if c.ExpectedError != "" {
			wantCacheError(t, c.Provider, err, c.ExpectedError)
			continue
		}
		if err != nil {
			t.Fatalf("%s: %v", c.Provider, err)
		}
		sameJSON(t, c.Provider, got, c.Expected)
	}
}

func TestGatewaySessionIsTheThreadID(t *testing.T) {
	for _, c := range loadCacheCases(t).GatewaySession {
		got, err := GatewaySession(c.Gateway, c.ThreadID, "", "")
		if c.ExpectedError != "" {
			wantCacheError(t, c.Gateway, err, c.ExpectedError)
			continue
		}
		if err != nil {
			t.Fatalf("%s: %v", c.Gateway, err)
		}
		sameJSON(t, c.Gateway, got, c.Expected)
	}
}
