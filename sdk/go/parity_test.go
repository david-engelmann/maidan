// The SDK method map (../methods.json) against this client, offline: every
// mapped method exists, and every example sends the request the map names and
// decodes the map's reply with no undeclared member. scripts/check-sdk-parity.py
// runs the same examples through the Python and TypeScript clients.
package maidan

import (
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"reflect"
	"sort"
	"strings"
	"testing"
)

type mapExample struct {
	Request struct {
		Method string          `json:"method"`
		Path   string          `json:"path"`
		Body   json.RawMessage `json:"body"`
	} `json:"request"`
	Reply struct {
		Status int             `json:"status"`
		Body   json.RawMessage `json:"body"`
	} `json:"reply"`
}

type mapEntry struct {
	Op      string      `json:"op"`
	Go      string      `json:"go"`
	Example *mapExample `json:"example"`
}

func loadMethodMap(t *testing.T) []mapEntry {
	t.Helper()
	raw, err := os.ReadFile("../methods.json")
	if err != nil {
		t.Fatal(err)
	}
	var m struct {
		Methods []mapEntry `json:"methods"`
	}
	if err := json.Unmarshal(raw, &m); err != nil {
		t.Fatal(err)
	}
	return m.Methods
}

// parityCalls makes each example's call with the example's arguments.
var parityCalls = map[string]func(c *Client) error{
	"reviews.packet":  func(c *Client) error { _, err := c.Reviews.Packet("t1"); return err },
	"reviews.request": func(c *Client) error { return c.Reviews.Request("t1", "m2") },
	"reviews.submit": func(c *Client) error {
		_, err := c.Reviews.Submit("t1", ReviewRequestChanges, &ReviewOptions{Note: "the retry test still sleeps"})
		return err
	},
	"reviews.list":        func(c *Client) error { _, err := c.Reviews.List("t1"); return err },
	"reviews.status":      func(c *Client) error { _, err := c.Reviews.Status("t1"); return err },
	"approval_gates.list": func(c *Client) error { _, err := c.ApprovalGates.List("w1"); return err },
	"approval_gates.answer": func(c *Client) error {
		_, err := c.ApprovalGates.Answer("g1", "ab12", "decline", nil)
		return err
	},
	"threads.set_footprint": func(c *Client) error {
		_, err := c.Threads.SetFootprint("t1", Footprint{
			Repo: "acme/widgets", Mode: FootprintExclusive,
			Items: []FootprintItem{{Kind: FootprintPath, Value: "sdk/go"}},
		})
		return err
	},
	"threads.get_footprint": func(c *Client) error { _, err := c.Threads.GetFootprint("t1"); return err },
	"delegation_grants.create": func(c *Client) error {
		_, err := c.DelegationGrants.Create("w1", "m1", "m2", []string{"workspace:read"}, "triage the inbox", "2026-11-12T00:00:00Z")
		return err
	},
	"delegation_grants.revoke": func(c *Client) error { _, err := c.DelegationGrants.Revoke("w1", "dg1"); return err },
	"tokens.delegate": func(c *Client) error {
		_, err := c.Tokens.Delegate("dg1", []string{"workspace:read"}, &DelegateOptions{Label: "inbox triage"})
		return err
	},
	"tokens.attenuate": func(c *Client) error {
		_, err := c.Tokens.Attenuate([]string{"workspace:read"}, &DelegateOptions{ExpiresAt: "2026-11-11T06:00:00Z"})
		return err
	},
}

func TestEveryMappedMethodExists(t *testing.T) {
	c := New("http://sdk.invalid", "t")
	for _, m := range loadMethodMap(t) {
		var fn reflect.Value
		if ns, meth, ok := strings.Cut(m.Go, "."); ok {
			field := reflect.ValueOf(c).Elem().FieldByName(ns)
			if !field.IsValid() {
				t.Errorf("%s: Client has no %s", m.Op, ns)
				continue
			}
			fn = field.MethodByName(meth)
		} else {
			fn = reflect.ValueOf(c).MethodByName(m.Go)
		}
		if !fn.IsValid() {
			t.Errorf("%s: no method %s", m.Op, m.Go)
		}
	}
}

func TestEveryExampleSendsTheMappedRequest(t *testing.T) {
	strictDecoding(t)

	var withExample []string
	for _, m := range loadMethodMap(t) {
		if m.Example == nil {
			continue
		}
		withExample = append(withExample, m.Op)
		call, ok := parityCalls[m.Op]
		if !ok {
			t.Errorf("%s has an example in the method map but no call in parityCalls", m.Op)
			continue
		}
		ex := m.Example
		var gotMethod, gotPath string
		var gotBody []byte
		srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
			gotMethod, gotPath = r.Method, r.URL.RequestURI()
			gotBody, _ = io.ReadAll(r.Body)
			if ex.Reply.Status == http.StatusNoContent {
				w.WriteHeader(http.StatusNoContent)
				return
			}
			w.Header().Set("content-type", "application/json")
			w.WriteHeader(ex.Reply.Status)
			_, _ = w.Write(ex.Reply.Body)
		}))
		c := New(srv.URL, "t")
		c.MaxRetries = 0
		err := call(c)
		srv.Close()
		if err != nil {
			t.Errorf("%s: %v", m.Op, err)
			continue
		}
		if gotMethod != ex.Request.Method || gotPath != ex.Request.Path {
			t.Errorf("%s: sent %s %s, the map says %s %s", m.Op, gotMethod, gotPath, ex.Request.Method, ex.Request.Path)
		}
		if !sameBody(gotBody, ex.Request.Body) {
			t.Errorf("%s: sent body %s, the map says %s", m.Op, gotBody, ex.Request.Body)
		}
	}
	var calls []string
	for op := range parityCalls {
		calls = append(calls, op)
	}
	sort.Strings(calls)
	sort.Strings(withExample)
	if !reflect.DeepEqual(calls, withExample) {
		t.Errorf("parityCalls %v, method map examples %v", calls, withExample)
	}
}

// sameBody compares two bodies as JSON values; an empty body equals null.
func sameBody(a, b []byte) bool {
	decode := func(raw []byte) any {
		if len(bytes.TrimSpace(raw)) == 0 {
			return nil
		}
		var v any
		if err := json.Unmarshal(raw, &v); err != nil {
			return string(raw)
		}
		return v
	}
	return reflect.DeepEqual(decode(a), decode(b))
}
