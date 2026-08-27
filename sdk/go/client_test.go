// Black-box test for the Maidan Go client against a running server (MAIDAN_URL,
// auth disabled). Run via `scripts/sdk-test.sh go`, which boots a server. These
// scenarios also exercise the server's REST + WS surface.
package maidan

import (
	"bytes"
	"encoding/json"
	"errors"
	"net/http"
	"os"
	"testing"
	"time"
)

func testClient(t *testing.T) *Client {
	t.Helper()
	base := os.Getenv("MAIDAN_URL")
	if base == "" {
		t.Skip("MAIDAN_URL not set; run via scripts/sdk-test.sh go")
	}
	return New(base, os.Getenv("MAIDAN_TOKEN"))
}

// seed creates a workspace + member (raw bootstrap route — member creation isn't
// in the SDK surface) + channel + thread.
func seed(t *testing.T, c *Client) (ws, member, channel, thread M) {
	t.Helper()
	var err error
	if ws, err = c.Workspaces.Create("go-sdk"); err != nil {
		t.Fatal(err)
	}
	body, _ := json.Marshal(M{"handle": "sdk-agent", "kind": "agent"})
	resp, err := http.Post(c.BaseURL+"/workspaces/"+ws["id"].(string)+"/members", "application/json", bytes.NewReader(body))
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if err := json.NewDecoder(resp.Body).Decode(&member); err != nil {
		t.Fatal(err)
	}
	if channel, err = c.Channels.Create(ws["id"].(string), "general", false); err != nil {
		t.Fatal(err)
	}
	if thread, err = c.Threads.Create(channel["id"].(string), "kickoff"); err != nil {
		t.Fatal(err)
	}
	return
}

func TestHeroLoopPostListContext(t *testing.T) {
	c := testClient(t)
	_, member, _, thread := seed(t, c)
	if _, err := c.Messages.Post(thread["id"].(string), member["id"].(string), "hello from the go sdk"); err != nil {
		t.Fatal(err)
	}
	msgs, err := c.Messages.List(thread["id"].(string), nil)
	if err != nil {
		t.Fatal(err)
	}
	found := false
	for _, m := range msgs {
		if m["body"] == "hello from the go sdk" {
			found = true
		}
	}
	if !found {
		t.Fatal("posted message not listed")
	}
	if _, err := c.Threads.Context(thread["id"].(string), nil); err != nil {
		t.Fatal(err)
	}
}

func TestGetResultUnsetIs404(t *testing.T) {
	// A full SetResult round-trip needs a real produced_by member (auth-enabled;
	// the server's thread_result_e2e proves it). Under the auth-disabled harness
	// the acting member is nil, so exercise the result route + client error path.
	c := testClient(t)
	_, _, _, thread := seed(t, c)
	_, err := c.Threads.GetResult(thread["id"].(string))
	var apiErr *APIError
	if !errors.As(err, &apiErr) || apiErr.Status != 404 {
		t.Fatalf("expected 404 APIError, got %v", err)
	}
}

func TestClaimNextReturnsClaimableOrNil(t *testing.T) {
	c := testClient(t)
	_, member, channel, _ := seed(t, c)
	if _, err := c.ClaimNextThread(channel["id"].(string), M{"member_id": member["id"]}); err != nil {
		t.Fatal(err)
	}
}

func TestErrorsSurfaceStatus(t *testing.T) {
	c := testClient(t)
	_, err := c.Threads.Get("00000000-0000-0000-0000-000000000000")
	var apiErr *APIError
	if !errors.As(err, &apiErr) || apiErr.Status < 400 {
		t.Fatalf("expected APIError >=400, got %v", err)
	}
}

func TestSubscribeDeliversAMessage(t *testing.T) {
	c := testClient(t)
	ws, member, _, thread := seed(t, c)
	got := make(chan Event, 1)
	sub, err := c.Subscribe(M{"workspace_id": ws["id"], "kinds": []string{"message_posted"}}, func(e Event) {
		if e["thread_id"] == thread["id"] {
			select {
			case got <- e:
			default:
			}
		}
	}, nil)
	if err != nil {
		t.Fatal(err)
	}
	defer sub.Close()

	time.Sleep(200 * time.Millisecond) // let the subscription attach
	if _, err := c.Messages.Post(thread["id"].(string), member["id"].(string), "ws ping"); err != nil {
		t.Fatal(err)
	}
	select {
	case e := <-got:
		if e["kind"] != "message_posted" {
			t.Fatalf("unexpected kind %v", e["kind"])
		}
	case <-time.After(10 * time.Second):
		t.Fatal("did not receive the message_posted event")
	}
}
