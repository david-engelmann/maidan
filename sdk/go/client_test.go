// Black-box tests against the authenticated server from scripts/sdk-test.sh.
package maidan

import (
	"bytes"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"net/url"
	"os"
	"sync/atomic"
	"testing"
	"time"
)

func TestParseRoomLSN(t *testing.T) {
	if n, ok := ParseRoomLSN("42"); !ok || n != 42 {
		t.Fatalf("decimal: got %d %v", n, ok)
	}
	if n, ok := ParseRoomLSN(" 0 "); !ok || n != 0 {
		t.Fatalf("zero: got %d %v", n, ok)
	}
	if _, ok := ParseRoomLSN("0/3000128"); ok {
		t.Fatal("WAL text must not parse as Room-LSN")
	}
	if _, ok := ParseRoomLSN("-1"); ok {
		t.Fatal("negative must not parse")
	}
	if EventType("message_posted") != "maidan.event.message_posted/1" {
		t.Fatal(EventType("message_posted"))
	}
}

func TestIsCursorTooOld(t *testing.T) {
	tooOld := &APIError{
		Status:  409,
		Type:    ProblemBase + "cursor-too-old",
		Problem: M{"type": ProblemBase + "cursor-too-old", "must_refetch": true},
	}
	if !tooOld.IsConflict() || !tooOld.IsCursorTooOld() {
		t.Fatal("expected 409 must_refetch to be cursor-too-old")
	}
	plain := &APIError{Status: 409, Type: ProblemBase + "conflict"}
	if !plain.IsConflict() || plain.IsCursorTooOld() {
		t.Fatal("plain 409 must not be cursor-too-old")
	}
	if (&APIError{Status: 500, Problem: M{"must_refetch": true}}).IsCursorTooOld() {
		t.Fatal("must_refetch on a non-409 is not cursor-too-old")
	}
}

func TestNormalizeStoredPromotesID(t *testing.T) {
	ws := "ws"
	live := normalizeStored(StoredEvent{
		ID:          42,
		Kind:        "message_posted",
		WorkspaceID: &ws,
		Payload:     json.RawMessage(`{"kind":"message_posted","body":"hi"}`),
	})
	if live["log_id"] != int64(42) || live["body"] != "hi" || live["workspace_id"] != "ws" {
		t.Fatalf("normalize = %#v", live)
	}
}

func testClient(t *testing.T) *Client {
	t.Helper()
	base := os.Getenv("MAIDAN_URL")
	if base == "" {
		t.Skip("MAIDAN_URL not set; run via scripts/sdk-test.sh go")
	}
	return New(base, os.Getenv("MAIDAN_TOKEN"))
}

var seedID atomic.Uint64

// unique names a seeded resource so reruns against one server do not collide.
func unique(prefix string) string {
	return fmt.Sprintf("%s-%d-%d", prefix, time.Now().UnixNano(), seedID.Add(1))
}

// seed creates an isolated queue in the token's bootstrap workspace.
func seed(t *testing.T, c *Client) (wid, memberID string, channel *Channel, thread *Thread) {
	t.Helper()
	wid = os.Getenv("MAIDAN_WORKSPACE")
	var me struct {
		MemberID string `json:"member_id"`
	}
	getJSON(t, c, "/me", &me)
	memberID = me.MemberID
	var err error
	if channel, err = c.Channels.Create(wid, unique("go-sdk"), false); err != nil {
		t.Fatal(err)
	}
	if thread, err = c.Threads.Create(channel.ID, "kickoff"); err != nil {
		t.Fatal(err)
	}
	return
}

func getJSON(t *testing.T, c *Client, path string, v any) {
	t.Helper()
	req, err := http.NewRequest(http.MethodGet, c.BaseURL+path, nil)
	if err != nil {
		t.Fatal(err)
	}
	req.Header.Set("Authorization", "Bearer "+c.Token)
	resp, err := http.DefaultClient.Do(req)
	if err != nil {
		t.Fatal(err)
	}
	defer resp.Body.Close()
	if err := json.NewDecoder(resp.Body).Decode(v); err != nil {
		t.Fatal(err)
	}
}

// strictDecoding makes every response decode refuse members its model does
// not declare, for the rest of the test. The models tolerate them otherwise
// (forward compatibility), so this is what proves they match the live server.
func strictDecoding(t *testing.T) {
	prev := decodeJSON
	decodeJSON = func(raw []byte, v any) error {
		dec := json.NewDecoder(bytes.NewReader(raw))
		dec.DisallowUnknownFields()
		return dec.Decode(v)
	}
	t.Cleanup(func() { decodeJSON = prev })
}

// must unwraps a call's result; a failed call panics, which fails the test
// with the error and its stack.
func must[T any](v T, err error) T {
	if err != nil {
		panic(err)
	}
	return v
}

func TestHeroLoopPostListContext(t *testing.T) {
	c := testClient(t)
	_, _, _, thread := seed(t, c)
	must(c.Messages.Post(thread.ID, "hello from the go sdk"))
	msgs := must(c.Messages.List(thread.ID, nil))
	found := false
	for _, m := range msgs {
		if m.Body == "hello from the go sdk" {
			found = true
		}
	}
	if !found {
		t.Fatal("posted message not listed")
	}
	if ctx := must(c.Threads.Context(thread.ID, nil)); ctx.Thread.ID != thread.ID {
		t.Fatalf("context is for %s", ctx.Thread.ID)
	}
}

func TestGetResultUnsetIs404(t *testing.T) {
	// Exercise the result route and client error path before a result exists.
	c := testClient(t)
	_, _, _, thread := seed(t, c)
	_, err := c.Threads.GetResult(thread.ID)
	var nf *NotFoundError
	if !errors.As(err, &nf) || nf.Status != 404 {
		t.Fatalf("expected a 404 NotFoundError, got %v", err)
	}
}

func TestClaimReturnsTheThreadFlattenedNotNested(t *testing.T) {
	// The seeded thread is ready, so this claims it. Strict decoding is the
	// shape assertion: a nested "thread" key would fail it.
	c := testClient(t)
	strictDecoding(t)
	_, memberID, channel, thread := seed(t, c)
	claim := must(c.ClaimNextThread(channel.ID, nil))
	if claim == nil {
		t.Fatal("a freshly seeded ready thread should be claimable")
	}
	if claim.ID != thread.ID {
		t.Fatalf("claimed %v, seeded %v", claim.ID, thread.ID)
	}
	if claim.AssigneeID == nil || *claim.AssigneeID != memberID {
		t.Fatalf("assignee %v, member %v", claim.AssigneeID, memberID)
	}
	if claim.ClaimLeaseID == nil {
		t.Fatal("no claim_lease_id — the fencing token RenewClaim needs")
	}
	if claim.Pin.URI == "" || claim.Pin.ContentHash == "" {
		t.Fatalf("expected a content-addressed pin, got %+v", claim.Pin)
	}
}

func TestRenewClaimExtendsTheLeaseWithTheFencingToken(t *testing.T) {
	c := testClient(t)
	_, _, channel, _ := seed(t, c)
	claim := must(c.ClaimNextThread(channel.ID, &ClaimOptions{LeaseSecs: 60}))
	renewed := must(c.RenewClaim(claim.ID, *claim.ClaimLeaseID, 600))
	if !renewed.AssignmentExpiresAt.After(*claim.AssignmentExpiresAt) {
		t.Fatalf("lease not extended: %v -> %v", claim.AssignmentExpiresAt, renewed.AssignmentExpiresAt)
	}
}

func TestClaimNextReturnsNilOnceDrained(t *testing.T) {
	c := testClient(t)
	_, _, channel, _ := seed(t, c)
	must(c.ClaimNextThread(channel.ID, nil))
	if drained := must(c.ClaimNextThread(channel.ID, nil)); drained != nil {
		t.Fatalf("expected nil on an empty queue, got %+v", drained)
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
	wid, _, _, thread := seed(t, c)
	got := make(chan Event, 1)
	sub, err := c.Subscribe(M{"workspace_id": wid, "kinds": []string{"message_posted"}}, func(e Event) {
		if e["thread_id"] == thread.ID {
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
	must(c.Messages.Post(thread.ID, "ws ping"))
	select {
	case e := <-got:
		if e["kind"] != "message_posted" {
			t.Fatalf("unexpected kind %v", e["kind"])
		}
	case <-time.After(10 * time.Second):
		t.Fatal("did not receive the message_posted event")
	}
}

func TestProvisioningSeedsAMemberAndMintsAScopedToken(t *testing.T) {
	// The first thing an integrator does after `maidan init`.
	c := testClient(t)
	wid := os.Getenv("MAIDAN_WORKSPACE")
	handle := unique("provisioned")

	member := must(c.Members.Create(wid, handle, MemberAgent, ""))
	if member.Handle != handle || member.Kind != MemberAgent {
		t.Fatalf("unexpected member %+v", member)
	}
	found := false
	for _, m := range must(c.Members.List(wid)) {
		if m.ID == member.ID {
			found = true
		}
	}
	if !found {
		t.Fatal("created member not listed")
	}

	minted := must(c.Tokens.Mint(wid, member.ID, []string{"workspace:read"}, &MintOptions{Label: "scoped worker"}))
	if minted.Secret == "" {
		t.Fatal("the secret is returned once, in the mint response")
	}
	// Strict decoding refuses a "secret" member, so a leak fails the list.
	strictDecoding(t)
	listed := must(c.Tokens.List(wid, member.ID))
	if len(listed) != 1 || listed[0].ID != minted.ID || listed[0].Label == nil || *listed[0].Label != "scoped worker" {
		t.Fatalf("listed %+v", listed)
	}
}

func TestThreadsListAllWalksEveryPage(t *testing.T) {
	c := testClient(t)
	_, _, channel, thread := seed(t, c)
	made := map[string]bool{thread.ID: true}
	for i := 0; i < 4; i++ {
		made[must(c.Threads.Create(channel.ID, fmt.Sprintf("t%d", i))).ID] = true
	}
	seen := 0
	err := c.Threads.ListAll(channel.ID, 2, func(th Thread) error {
		if !made[th.ID] {
			return fmt.Errorf("unexpected thread %v", th.ID)
		}
		seen++
		return nil
	})
	if err != nil || seen != len(made) {
		t.Fatalf("seen %d of %d: %v", seen, len(made), err)
	}
}

func TestEveryDocumentedOperationReturnsItsDeclaredModel(t *testing.T) {
	c := testClient(t)
	strictDecoding(t)
	wid, memberID, channel, thread := seed(t, c)

	if ws := must(c.Workspaces.Get(wid)); ws.ID != wid || ws.CreatedAt.IsZero() {
		t.Fatalf("workspace %+v", ws)
	}
	if len(must(c.Members.List(wid))) == 0 {
		t.Fatal("no members")
	}
	if len(must(c.Channels.List(wid))) == 0 {
		t.Fatal("no channels")
	}
	if th := must(c.Threads.Get(thread.ID)); th.State != ThreadOpen || th.ChannelID != channel.ID {
		t.Fatalf("thread %+v", th)
	}
	if len(must(c.Threads.List(channel.ID, nil))) != 1 {
		t.Fatal("expected the seeded thread")
	}

	msg := must(c.Messages.Post(thread.ID, "typed"))
	if msg.AuthorID != memberID || msg.PostedAt.IsZero() {
		t.Fatalf("message %+v", msg)
	}
	must(c.Messages.List(thread.ID, nil))

	art := must(c.Artifacts.Upload([]byte("typed bytes"), ArtifactAttachment))
	if art.SizeBytes != int64(len("typed bytes")) || art.Kind != ArtifactAttachment {
		t.Fatalf("artifact %+v", art)
	}
	must(c.Artifacts.Meta(art.SHA256))
	if b := must(c.Artifacts.Get(art.SHA256)); string(b) != "typed bytes" {
		t.Fatalf("artifact bytes %q", b)
	}

	claim := must(c.ClaimNextThread(channel.ID, &ClaimOptions{LeaseSecs: 60}))
	must(c.RenewClaim(claim.ID, *claim.ClaimLeaseID, 120))

	res := must(c.Threads.SetResult(thread.ID, M{"ok": true}))
	var payload struct{ OK bool }
	if err := json.Unmarshal(res.Result, &payload); err != nil || !payload.OK || res.ProducedBy != memberID {
		t.Fatalf("result %+v %v", res, err)
	}
	must(c.Threads.GetResult(thread.ID))
	if th := must(c.Threads.Transition(thread.ID, "start_review")); th.State != ThreadInReview {
		t.Fatalf("state %s", th.State)
	}

	ctx := must(c.Threads.Context(thread.ID, nil))
	if len(ctx.Fsm.Transitions) == 0 || len(ctx.Messages) == 0 {
		t.Fatalf("context %+v", ctx)
	}

	events := must(c.Workspaces.ListEvents(wid, url.Values{"limit": {"50"}}))
	if len(events) == 0 || events[0].Type != EventType(events[0].Kind) {
		t.Fatalf("events %+v", events)
	}
	if err := c.Workspaces.ListEventsAll(wid, url.Values{"limit": {"25"}}, func(StoredEvent) error { return nil }); err != nil {
		t.Fatal(err)
	}

	fresh := must(c.Members.Create(wid, unique("typed"), "", ""))
	must(c.Tokens.Mint(wid, fresh.ID, []string{"workspace:read"}, nil))
	must(c.Tokens.List(wid, fresh.ID))

	var bundle json.RawMessage
	getJSON(t, c, "/workspaces/"+wid+"/export", &bundle)
	imported := must(c.Workspaces.Import(bundle, ImportNew))
	if imported.Mode != ImportNew || imported.WorkspaceID == wid {
		t.Fatalf("import %+v", imported)
	}
}

func TestTheServersProblemTypesArriveAsTheirErrorTypes(t *testing.T) {
	c := testClient(t)
	wid, _, _, thread := seed(t, c)
	var bundle json.RawMessage
	getJSON(t, c, "/workspaces/"+wid+"/export", &bundle)
	check := func(name string, err error, target any, status int, typ string) {
		t.Helper()
		if !errors.As(err, target) {
			t.Fatalf("%s: expected %T, got %T %v", name, target, err, err)
		}
		var api *APIError
		if !errors.As(err, &api) {
			t.Fatalf("%s: not an *APIError", name)
		}
		if api.Status != status || api.Type != ProblemBase+typ || api.Problem["type"] != api.Type || api.Title == "" || api.Detail == "" {
			t.Fatalf("%s: %+v", name, api)
		}
	}
	var (
		nf  *NotFoundError
		un  *UnauthorizedError
		br  *BadRequestError
		fb  *ForbiddenError
		cfl *ConflictError
	)
	_, err := c.Threads.Get("00000000-0000-0000-0000-000000000000")
	check("not found", err, &nf, 404, "not-found")
	_, err = New(c.BaseURL, "maid_not_a_token").Workspaces.Get(wid)
	check("bad token", err, &un, 401, "unauthorized")
	_, err = c.Threads.Transition(thread.ID, "fly")
	check("bad action", err, &br, 400, "bad-request")
	// Bootstrap creates only the first workspace; `maidan init` already made it.
	_, err = c.Workspaces.Create("second")
	check("second workspace", err, &fb, 403, "forbidden")
	_, err = c.Workspaces.Import(bundle, ImportRestore)
	check("restore over itself", err, &cfl, 409, "conflict")
}
