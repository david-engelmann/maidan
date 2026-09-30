// Unit tests for the problem-type errors and forward-compatible decoding,
// against a local httptest server (no Maidan server needed).
package maidan

import (
	"errors"
	"fmt"
	"net/http"
	"net/http/httptest"
	"sort"
	"strings"
	"testing"
)

var serverProblemTypes = []string{
	"not-found", "method-not-allowed", "conflict", "bad-request", "unauthorized",
	"invalid-signature", "forbidden", "payload-too-large", "unsupported-media-type",
	"rate-limited", "bad-gateway", "internal", "overloaded", "idempotency-key-reused",
	"idempotency-key-in-flight", "cursor-too-old", "event-log-broken",
}

func answering(t *testing.T, status int, body string, headers map[string]string) *Client {
	t.Helper()
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		for k, v := range headers {
			w.Header().Set(k, v)
		}
		w.WriteHeader(status)
		_, _ = w.Write([]byte(body))
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	c.MaxRetries = 0
	return c
}

func TestEveryProblemTypeTheServerDocumentsHasItsOwnError(t *testing.T) {
	var got, want []string
	for typ := range ProblemTypes {
		got = append(got, typ)
	}
	for _, typ := range serverProblemTypes {
		want = append(want, ProblemBase+typ)
	}
	sort.Strings(got)
	sort.Strings(want)
	if strings.Join(got, " ") != strings.Join(want, " ") {
		t.Fatalf("ProblemTypes = %v, want %v", got, want)
	}
	seen := map[string]bool{}
	for typ := range ProblemTypes {
		err := NewProblemError(418, []byte(fmt.Sprintf(`{"type":%q,"title":"T","status":418,"detail":"d"}`, typ)))
		name := fmt.Sprintf("%T", err)
		if seen[name] || name == "*maidan.UnknownProblemError" {
			t.Fatalf("%s maps to %s", typ, name)
		}
		seen[name] = true
		var api *APIError
		if !errors.As(err, &api) || api.Type != typ {
			t.Fatalf("%s does not unwrap to its *APIError", name)
		}
	}
}

func TestAnErrorCarriesStatusTypeTitleDetailAndTheRawProblem(t *testing.T) {
	c := answering(t, 404, `{"type":"https://maidan.dev/problems/not-found","title":"Not Found","status":404,"detail":"gone","trace":"added later"}`, nil)
	_, err := c.Threads.Get("t1")
	var nf *NotFoundError
	if !errors.As(err, &nf) {
		t.Fatalf("got %T %v", err, err)
	}
	if nf.Status != 404 || nf.Title != "Not Found" || nf.Detail != "gone" || nf.Problem["trace"] != "added later" {
		t.Fatalf("%+v", nf.APIError)
	}
	if err.Error() != "maidan: request failed: HTTP 404: gone" {
		t.Fatal(err.Error())
	}
}

func TestAnUnknownProblemTypeFallsBackToUnknownProblemError(t *testing.T) {
	c := answering(t, 409, `{"type":"https://maidan.dev/problems/added-next-year","title":"New","status":409,"detail":"d"}`, nil)
	_, err := c.Threads.Get("t1")
	var unknown *UnknownProblemError
	if !errors.As(err, &unknown) || !unknown.IsConflict() || unknown.Type != ProblemBase+"added-next-year" {
		t.Fatalf("got %T %v", err, err)
	}
}

func TestABodyThatIsNotAProblemIsUnknownWithItsTextAsDetail(t *testing.T) {
	c := answering(t, 502, "<html>bad gateway</html>", nil)
	_, err := c.Threads.Get("t1")
	var unknown *UnknownProblemError
	if !errors.As(err, &unknown) || unknown.Type != "" || unknown.Problem != nil || unknown.Detail != "<html>bad gateway</html>" {
		t.Fatalf("got %T %+v", err, err)
	}
}

func TestCursorTooOldCarriesTheSnapshotAndRetryAfterIsKept(t *testing.T) {
	c := answering(t, 409, `{"type":"https://maidan.dev/problems/cursor-too-old","title":"Cursor Too Old","status":409,"detail":"refetch","must_refetch":true,"snapshot":"/workspaces/w/snapshot"}`, nil)
	_, err := c.Workspaces.ListEvents("w", nil)
	var old *CursorTooOldError
	if !errors.As(err, &old) || !old.IsCursorTooOld() || old.Snapshot() != "/workspaces/w/snapshot" {
		t.Fatalf("got %T %v", err, err)
	}
	c = answering(t, 503, `{"type":"https://maidan.dev/problems/overloaded","title":"Service Unavailable","status":503,"detail":"busy"}`, map[string]string{"Retry-After": "7"})
	_, err = c.Threads.Get("t1")
	var busy *OverloadedError
	if !errors.As(err, &busy) || busy.RetryAfter != 7 {
		t.Fatalf("got %T %v", err, err)
	}
}

func TestMembersAModelDoesNotDeclareDoNotBreakAResponse(t *testing.T) {
	c := answering(t, 200, `{"id":"t1","channel_id":"c1","state":"blocked_on_mars","created_at":"2026-09-29T00:00:00.123456789Z","updated_at":"2026-09-29T00:00:00Z","novel":{"deep":1}}`, nil)
	th, err := c.Threads.Get("t1")
	if err != nil || th.ID != "t1" || th.State != "blocked_on_mars" || th.CreatedAt.Nanosecond() != 123456789 {
		t.Fatalf("%+v %v", th, err)
	}
}
