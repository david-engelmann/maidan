// Unit tests for retries, idempotency keys and auto-paging, against a local
// httptest server (no Maidan server needed).
package maidan

import (
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"strings"
	"sync"
	"testing"
	"time"
)

type answer struct {
	status  int
	body    any
	headers map[string]string
	hangup  bool
}

type recorded struct {
	method, url, key string
}

func fakeServer(t *testing.T, answers []answer) (*Client, *[]recorded, *[]time.Duration) {
	t.Helper()
	var mu sync.Mutex
	calls := []recorded{}
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		mu.Lock()
		calls = append(calls, recorded{r.Method, r.URL.String(), r.Header.Get("Idempotency-Key")})
		if len(answers) == 0 {
			mu.Unlock()
			t.Errorf("unexpected request %s %s", r.Method, r.URL)
			w.WriteHeader(500)
			return
		}
		a := answers[0]
		answers = answers[1:]
		mu.Unlock()
		if a.hangup {
			hj, _ := w.(http.Hijacker)
			conn, _, _ := hj.Hijack()
			conn.Close()
			return
		}
		for k, v := range a.headers {
			w.Header().Set(k, v)
		}
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(a.status)
		_ = json.NewEncoder(w).Encode(a.body)
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	sleeps := []time.Duration{}
	c.Sleep = func(d time.Duration) { sleeps = append(sleeps, d) }
	return c, &calls, &sleeps
}

func TestWriteRetriesLostResponseWithSameKey(t *testing.T) {
	c, calls, _ := fakeServer(t, []answer{{hangup: true}, {status: 201, body: M{"id": "m1"}}})
	m, err := c.Messages.Post("t1", "hi")
	if err != nil || m.ID != "m1" {
		t.Fatalf("%v %v", m, err)
	}
	if len(*calls) != 2 || (*calls)[0].key == "" || (*calls)[0].key != (*calls)[1].key {
		t.Fatalf("calls %+v", *calls)
	}
}

func TestEachWriteOwnKeyReadsNone(t *testing.T) {
	c, calls, _ := fakeServer(t, []answer{{status: 201, body: M{}}, {status: 201, body: M{}}, {status: 200, body: []M{}}})
	_, _ = c.Messages.Post("t1", "a")
	_, _ = c.Messages.Post("t1", "b")
	_, _ = c.Channels.List("w")
	cs := *calls
	if cs[0].key == cs[1].key || cs[2].key != "" {
		t.Fatalf("calls %+v", cs)
	}
}

func TestRateLimitAndServerErrorsBoundedRetries(t *testing.T) {
	c, calls, sleeps := fakeServer(t, []answer{
		{status: 429, body: M{}, headers: map[string]string{"Retry-After": "3"}},
		{status: 503, body: M{}},
		{status: 503, body: M{"detail": "down"}},
	})
	_, err := c.Channels.List("w")
	var apiErr *APIError
	if !errors.As(err, &apiErr) || apiErr.Status != 503 {
		t.Fatalf("err %v", err)
	}
	if len(*calls) != 3 {
		t.Fatalf("calls %d", len(*calls))
	}
	s := *sleeps
	if s[0] != 3*time.Second || s[1] < 500*time.Millisecond || s[1] > time.Second {
		t.Fatalf("sleeps %v", s)
	}
}

func TestInFlight409RetriedPlain409Not(t *testing.T) {
	c, calls, _ := fakeServer(t, []answer{
		{status: 409, body: M{"type": inFlightType}},
		{status: 201, body: M{"id": "c"}},
	})
	if ch, err := c.Channels.Create("w", "n", false); err != nil || ch.ID != "c" || len(*calls) != 2 {
		t.Fatalf("%v %v %d", ch, err, len(*calls))
	}
	c, calls, _ = fakeServer(t, []answer{{status: 409, body: M{"type": "https://maidan.dev/problems/conflict"}}})
	if _, err := c.Channels.Create("w", "n", false); err == nil || len(*calls) != 1 {
		t.Fatalf("%v %d", err, len(*calls))
	}
}

func TestForbiddenNotRetriedAndMaxRetriesZero(t *testing.T) {
	c, calls, _ := fakeServer(t, []answer{{status: 403, body: M{}}})
	if _, err := c.Channels.List("w"); err == nil || len(*calls) != 1 {
		t.Fatalf("%v %d", err, len(*calls))
	}
	c, calls, _ = fakeServer(t, []answer{{status: 503, body: M{}}})
	c.MaxRetries = 0
	if _, err := c.Channels.List("w"); err == nil || len(*calls) != 1 {
		t.Fatalf("%v %d", err, len(*calls))
	}
}

func TestRetryDelay(t *testing.T) {
	zero, one := func() float64 { return 0 }, func() float64 { return 1 }
	if d := RetryDelay(0, "", zero); d != 250*time.Millisecond {
		t.Fatal(d)
	}
	if d := RetryDelay(10, "", one); d != 8*time.Second {
		t.Fatal(d)
	}
	if d := RetryDelay(0, "120", zero); d != 60*time.Second {
		t.Fatal(d)
	}
}

func TestThreadsListAllPagesByCursor(t *testing.T) {
	c, calls, _ := fakeServer(t, []answer{
		{status: 200, body: []M{{"id": "a"}, {"id": "b"}}},
		{status: 200, body: []M{{"id": "c"}}},
	})
	var ids []string
	if err := c.Threads.ListAll("ch", 2, func(th Thread) error { ids = append(ids, th.ID); return nil }); err != nil {
		t.Fatal(err)
	}
	if strings.Join(ids, ",") != "a,b,c" {
		t.Fatal(ids)
	}
	if !strings.HasSuffix((*calls)[1].url, "cursor=b&limit=2") {
		t.Fatal((*calls)[1].url)
	}
}

func TestListEventsAllPagesByAfterID(t *testing.T) {
	c, calls, _ := fakeServer(t, []answer{
		{status: 200, body: []M{{"id": 1}, {"id": 2}}},
		{status: 200, body: []M{{"id": 3}}},
	})
	n := 0
	if err := c.Workspaces.ListEventsAll("w", map[string][]string{"limit": {"2"}}, func(StoredEvent) error { n++; return nil }); err != nil {
		t.Fatal(err)
	}
	if n != 3 || !strings.Contains((*calls)[1].url, "after_id=2") {
		t.Fatalf("%d %s", n, (*calls)[1].url)
	}
}
