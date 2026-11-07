// Unit tests for retries, idempotency keys and auto-paging, against a local
// httptest server (no Maidan server needed).
package maidan

import (
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"net/url"
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
	c, calls, _ = fakeServer(t, []answer{{status: 409, body: M{"type": "https://maidan.world/problems/conflict"}}})
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

// The server clamps limit to 500. A helper that asked for more would get a
// page of 500, read it as short, and stop with rows left.
func TestPagingHelpersAskForNoMoreThanTheServersPageSize(t *testing.T) {
	threads := make([]M, MaxPageSize)
	for i := range threads {
		threads[i] = M{"id": fmt.Sprintf("t%d", i)}
	}
	c, calls, _ := fakeServer(t, []answer{
		{status: 200, body: threads},
		{status: 200, body: []M{{"id": "last"}}},
	})
	n := 0
	if err := c.Threads.ListAll("ch", 1000, func(Thread) error { n++; return nil }); err != nil {
		t.Fatal(err)
	}
	if n != MaxPageSize+1 || !strings.Contains((*calls)[0].url, "limit=500") {
		t.Fatalf("%d threads, first request %s", n, (*calls)[0].url)
	}

	events := make([]M, MaxPageSize)
	for i := range events {
		events[i] = M{"id": i + 1}
	}
	c, calls, _ = fakeServer(t, []answer{
		{status: 200, body: events},
		{status: 200, body: []M{{"id": MaxPageSize + 1}}},
	})
	n = 0
	if err := c.Workspaces.ListEventsAll("w", map[string][]string{"limit": {"1000"}}, func(StoredEvent) error { n++; return nil }); err != nil {
		t.Fatal(err)
	}
	if n != MaxPageSize+1 || !strings.Contains((*calls)[0].url, "limit=500") {
		t.Fatalf("%d events, first request %s", n, (*calls)[0].url)
	}

	for in, want := range map[int]int{0: 100, -1: 100, 7: 7, 500: 500, 501: 500} {
		if got := pageSize(in); got != want {
			t.Fatalf("pageSize(%d) = %d, want %d", in, got, want)
		}
	}
}

func cutShort(w http.ResponseWriter) {
	hj, _ := w.(http.Hijacker)
	conn, bufrw, _ := hj.Hijack()
	_, _ = bufrw.WriteString("HTTP/1.1 200 OK\r\nContent-Length: 200\r\nContent-Type: application/json\r\n\r\n{\"id\":\"")
	bufrw.Flush()
	conn.Close()
}

func TestClaimRetriesCutShortBodyOnce(t *testing.T) {
	claims := map[string]string{}
	var mu sync.Mutex
	var keys []string
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		key := r.Header.Get("Idempotency-Key")
		mu.Lock()
		keys = append(keys, key)
		first := len(keys) == 1
		if _, ok := claims[key]; !ok {
			claims[key] = fmt.Sprintf("thr-%d", len(claims)+1)
		}
		id := claims[key]
		mu.Unlock()
		if first {
			cutShort(w)
			return
		}
		body := fmt.Sprintf(`{"id":%q,"claim_lease_id":"lease-1","state":"open","channel_id":"c"}`, id)
		w.Header().Set("Content-Type", "application/json")
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, body)
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	c.Sleep = func(time.Duration) {}
	claim, err := c.ClaimNextThread("c", nil)
	if err != nil || claim.ID != "thr-1" || claim.ClaimLeaseID == nil || *claim.ClaimLeaseID != "lease-1" {
		t.Fatalf("%+v %v", claim, err)
	}
	mu.Lock()
	defer mu.Unlock()
	if len(claims) != 1 || len(keys) != 2 || keys[0] == "" || keys[0] != keys[1] {
		t.Fatalf("claims %v keys %v", claims, keys)
	}
}

func TestArtifactGetRetriesCutShortBody(t *testing.T) {
	var n int
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		n++
		if n == 1 {
			cutShort(w)
			return
		}
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, "hello-artifact-bytes!!")
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	c.Sleep = func(time.Duration) {}
	got, err := c.Artifacts.Get("abc")
	if err != nil || string(got) != "hello-artifact-bytes!!" {
		t.Fatalf("%q %v", got, err)
	}
	if n != 2 {
		t.Fatalf("requests %d", n)
	}
}

func TestCutShortClaimThatNeverRecoversIsUnreadable(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "text/html")
		w.WriteHeader(http.StatusOK)
		_, _ = io.WriteString(w, "<html>proxy</html>")
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	c.Sleep = func(time.Duration) {}
	_, err := c.ClaimNextThread("c", nil)
	var unread *UnreadableResponseError
	var api *APIError
	if !errors.As(err, &unread) || !errors.As(err, &api) || api.Status != http.StatusOK {
		t.Fatalf("%v", err)
	}
}

func TestExhaustedTransportUnwrapsToStatusZero(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		hj, _ := w.(http.Hijacker)
		conn, _, _ := hj.Hijack()
		conn.Close()
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	c.MaxRetries = 0
	c.Sleep = func(time.Duration) {}
	_, err := c.ClaimNextThread("c", nil)
	var api *APIError
	var uerr *url.Error
	if !errors.As(err, &api) || api.Status != 0 || !errors.As(err, &uerr) {
		t.Fatalf("%v", err)
	}
}

func TestDriftedClaimIsUnreadable(t *testing.T) {
	for _, body := range []string{`{}`, `{"id":"thr"}`, `{"id":1,"claim_lease_id":"lease"}`} {
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			w.Header().Set("Content-Type", "application/json")
			w.WriteHeader(http.StatusOK)
			_, _ = io.WriteString(w, body)
		}))
		c := New(srv.URL, "t")
		c.MaxRetries = 0
		c.Sleep = func(time.Duration) {}
		_, err := c.ClaimNextThread("c", nil)
		var unread *UnreadableResponseError
		var api *APIError
		if !errors.As(err, &unread) || !errors.As(err, &api) {
			t.Fatalf("%s: %v", body, err)
		}
		srv.Close()
	}
}

func TestFinalCutShortUnwrapsUnexpectedEOF(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		cutShort(w)
	}))
	t.Cleanup(srv.Close)
	c := New(srv.URL, "t")
	c.MaxRetries = 0
	c.Sleep = func(time.Duration) {}
	_, err := c.Artifacts.Get("abc")
	var api *APIError
	if !errors.As(err, &api) || api.Status != 0 || !errors.Is(err, io.ErrUnexpectedEOF) {
		t.Fatalf("%v", err)
	}
}
