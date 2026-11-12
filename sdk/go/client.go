// Package maidan is the official Go client for Maidan, the operating layer for
// teams of AI agents. It speaks REST + WebSocket (MCP is a URL, not a dependency;
// A2A is a recipe) and is dependency-free — standard library only. See the repo's
// docs/Client Contract.md for the frozen v1 surface.
package maidan

import (
	"bytes"
	"crypto/rand"
	"encoding/json"
	"fmt"
	"io"
	"math"
	mrand "math/rand"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

// Version is the client version, tracked independently of the server.
const Version = "0.3.0"

// RoomLSNHeader is the projector-lag header (HTTP is case-insensitive).
// Distinct from Maidan-Consistency-Token (Postgres WAL LSN).
const RoomLSNHeader = "Maidan-Room-LSN"

// ParseRoomLSN parses Maidan-Room-LSN. Rejects WAL text (0/hex) so this is
// never confused with Maidan-Consistency-Token.
func ParseRoomLSN(s string) (int64, bool) {
	trimmed := strings.TrimSpace(s)
	if trimmed == "" || strings.Contains(trimmed, "/") {
		return 0, false
	}
	n, err := strconv.ParseInt(trimmed, 10, 64)
	if err != nil || n < 0 {
		return 0, false
	}
	return n, true
}

// EventType is the observable $type for an event kind
// (message_posted → maidan.event.message_posted/1).
func EventType(kind string) string {
	return "maidan.event." + kind + "/1"
}

// M is a JSON object: a request body, a subscribe filter, or an event frame.
type M = map[string]any

// Event is a bus event frame delivered to a Subscribe callback.
type Event = map[string]any

// Client is a Maidan v1 client over REST + WebSocket.
type Client struct {
	BaseURL string
	Token   string
	// MCPURL is {BaseURL}/mcp/streamable — a string only, no MCP dependency.
	MCPURL string
	HTTP   *http.Client
	// LastRoomLSN is the last seen Maidan-Room-LSN (event-log high-water).
	// Nil until a stamped response is seen. Not a WAL token.
	LastRoomLSN *int64

	// MaxRetries bounds the retries of a request that failed in transit or
	// answered 408, 429 (honouring Retry-After), 500/502/503/504, or a 409
	// idempotency-key-in-flight. New sets 2; 0 turns retries off. Writes carry
	// one Idempotency-Key across their attempts.
	MaxRetries int
	// Sleep waits between attempts (a test seam; New sets time.Sleep).
	Sleep func(time.Duration)

	Workspaces *WorkspacesService
	Agents     *AgentsService
	Members    *MembersService
	Tokens     *TokensService
	Channels   *ChannelsService
	Threads    *ThreadsService
	Messages   *MessagesService
	Artifacts  *ArtifactsService

	Reviews          *ReviewsService
	ApprovalGates    *ApprovalGatesService
	DelegationGrants *DelegationGrantsService
}

// New builds a client. Empty baseURL/token fall back to MAIDAN_URL / MAIDAN_TOKEN
// (then http://127.0.0.1:8080 / ""). Explicit args win.
func New(baseURL, token string) *Client {
	if baseURL == "" {
		baseURL = os.Getenv("MAIDAN_URL")
	}
	if baseURL == "" {
		baseURL = "http://127.0.0.1:8080"
	}
	baseURL = strings.TrimRight(baseURL, "/")
	if token == "" {
		token = os.Getenv("MAIDAN_TOKEN")
	}
	c := &Client{
		BaseURL: baseURL,
		Token:   token,
		MCPURL:  baseURL + "/mcp/streamable",
		HTTP:    &http.Client{Timeout: 30 * time.Second},

		MaxRetries: 2,
		Sleep:      time.Sleep,
	}
	c.Workspaces = &WorkspacesService{c}
	c.Agents = &AgentsService{c}
	c.Members = &MembersService{c}
	c.Tokens = &TokensService{c}
	c.Channels = &ChannelsService{c}
	c.Threads = &ThreadsService{c}
	c.Messages = &MessagesService{c}
	c.Artifacts = &ArtifactsService{c}
	c.Reviews = &ReviewsService{c}
	c.ApprovalGates = &ApprovalGatesService{c}
	c.DelegationGrants = &DelegationGrantsService{c}
	return c
}

const inFlightType = ProblemBase + "idempotency-key-in-flight"

// MaxPageSize is the most rows the server returns for one page: it clamps a
// larger limit to this. The paging helpers ask for no more, because they stop
// at the first short page, and a clamped page would look like the last one.
const MaxPageSize = 500

// pageSize is the limit a paging helper asks for: n, or 100 when n is not
// positive, and never more than MaxPageSize.
func pageSize(n int) int {
	if n <= 0 {
		return 100
	}
	return min(n, MaxPageSize)
}

// NewIdempotencyKey returns a fresh Idempotency-Key (a random UUID): one per
// logical write, reused by its retries.
func NewIdempotencyKey() string {
	var b [16]byte
	_, _ = rand.Read(b[:])
	b[6] = (b[6] & 0x0f) | 0x40
	b[8] = (b[8] & 0x3f) | 0x80
	return fmt.Sprintf("%x-%x-%x-%x-%x", b[0:4], b[4:6], b[6:8], b[8:10], b[10:16])
}

// RetryDelay is the wait before retry attempt (0-based): the server's
// Retry-After when it sent one (capped at 60s), else 0.5s*2^attempt capped at
// 8s, jittered. rnd returns a value in [0,1).
func RetryDelay(attempt int, retryAfter string, rnd func() float64) time.Duration {
	if retryAfter != "" {
		if f, err := strconv.ParseFloat(retryAfter, 64); err == nil && f >= 0 {
			return time.Duration(math.Min(f, 60) * float64(time.Second))
		}
	}
	base := math.Min(8, 0.5*math.Pow(2, float64(attempt)))
	return time.Duration((base/2 + rnd()*base/2) * float64(time.Second))
}

func retryable(status int, raw []byte) bool {
	switch status {
	case 408, 429, 500, 502, 503, 504:
		return true
	case 409:
		var p struct {
			Type string `json:"type"`
		}
		return json.Unmarshal(raw, &p) == nil && p.Type == inFlightType
	}
	return false
}

func isWrite(method string) bool {
	switch method {
	case http.MethodPost, http.MethodPut, http.MethodPatch, http.MethodDelete:
		return true
	}
	return false
}

func noAnswer(method, path string, attempt int, cause error) error {
	return &TransportError{
		APIError: &APIError{
			Status: 0,
			Detail: fmt.Sprintf("%s %s: no answer after %d tries: %v", method, path, attempt+1, cause),
		},
		Cause: cause,
	}
}

// send issues a request with retries and returns the last answer. A write
// carries one Idempotency-Key across all its attempts, so a retry after a lost
// response gets the first answer back instead of writing twice. A body that
// ends early, and (when jsonAnswer) a 2xx that is not JSON, are the same kind
// of lost answer. No answer after the last try is a TransportError whose
// APIError status is 0.
func (c *Client) send(method, path, contentType string, body []byte, jsonAnswer bool) (*http.Response, []byte, error) {
	key := ""
	if isWrite(method) {
		key = NewIdempotencyKey()
	}
	sleep := c.Sleep
	if sleep == nil {
		sleep = time.Sleep
	}
	for attempt := 0; ; attempt++ {
		var reader io.Reader
		if body != nil {
			reader = bytes.NewReader(body)
		}
		req, err := http.NewRequest(method, c.BaseURL+path, reader)
		if err != nil {
			return nil, nil, err
		}
		req.Header.Set("Authorization", "Bearer "+c.Token)
		if contentType != "" {
			req.Header.Set("Content-Type", contentType)
		}
		if key != "" {
			req.Header.Set("Idempotency-Key", key)
		}
		resp, err := c.HTTP.Do(req)
		if err != nil {
			if attempt >= c.MaxRetries {
				return nil, nil, noAnswer(method, path, attempt, err)
			}
			sleep(RetryDelay(attempt, "", mrand.Float64))
			continue
		}
		raw, readErr := io.ReadAll(resp.Body)
		resp.Body.Close()
		if readErr != nil {
			if attempt >= c.MaxRetries {
				return nil, nil, noAnswer(method, path, attempt, readErr)
			}
			sleep(RetryDelay(attempt, "", mrand.Float64))
			continue
		}
		c.captureRoomLSN(resp.Header)
		notJSON := jsonAnswer && resp.StatusCode >= 200 && resp.StatusCode < 300 &&
			resp.StatusCode != http.StatusNoContent && len(bytes.TrimSpace(raw)) > 0 && !json.Valid(raw)
		if attempt < c.MaxRetries && (notJSON || retryable(resp.StatusCode, raw)) {
			sleep(RetryDelay(attempt, resp.Header.Get("Retry-After"), mrand.Float64))
			continue
		}
		if notJSON {
			return resp, raw, &UnreadableResponseError{APIError: &APIError{
				Status: resp.StatusCode,
				Detail: fmt.Sprintf("%s %s: HTTP %d answer is not JSON", method, path, resp.StatusCode),
			}}
		}
		return resp, raw, nil
	}
}

// do sends a JSON request and returns the raw response body (nil on 204/empty).
func (c *Client) do(method, path string, body any) (json.RawMessage, error) {
	var payload []byte
	contentType := ""
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		payload, contentType = b, "application/json"
	}
	resp, raw, err := c.send(method, path, contentType, payload, true)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode >= 400 {
		return nil, apiError(resp, raw)
	}
	if resp.StatusCode == http.StatusNoContent || len(raw) == 0 {
		return nil, nil
	}
	return raw, nil
}

// doRaw is do for non-JSON bodies/responses (artifact bytes).
func (c *Client) doRaw(method, path string, body []byte) ([]byte, json.RawMessage, error) {
	resp, raw, err := c.send(method, path, "", body, method != http.MethodGet)
	if err != nil {
		return nil, nil, err
	}
	if resp.StatusCode >= 400 {
		return nil, nil, apiError(resp, raw)
	}
	if method == http.MethodGet {
		return raw, nil, nil
	}
	if resp.StatusCode == http.StatusNoContent || len(raw) == 0 {
		return nil, nil, nil
	}
	return nil, raw, nil
}

func (c *Client) captureRoomLSN(h http.Header) {
	if n, ok := ParseRoomLSN(h.Get(RoomLSNHeader)); ok {
		c.LastRoomLSN = &n
	}
}

// decodeJSON decodes a response body. The black-box tests swap in a decoder
// that refuses members a model does not declare, which is how they prove the
// models match what the live server returns.
var decodeJSON = json.Unmarshal

// call sends a JSON request and decodes the answer into a T; nil when the
// server answered 204, an empty body, or JSON null (claim-next with no work).
func call[T any](c *Client, method, path string, body any) (*T, error) {
	raw, err := c.do(method, path, body)
	if err != nil || raw == nil || string(bytes.TrimSpace(raw)) == "null" {
		return nil, err
	}
	var v T
	if err := decodeJSON(raw, &v); err != nil {
		return nil, &UnreadableResponseError{
			APIError: &APIError{
				Status: http.StatusOK,
				Detail: fmt.Sprintf("maidan: decoding %s %s: %v", method, path, err),
			},
			Cause: err,
		}
	}
	return &v, nil
}

func callList[T any](c *Client, path string) ([]T, error) {
	rows, err := call[[]T](c, http.MethodGet, path, nil)
	if err != nil || rows == nil {
		return nil, err
	}
	return *rows, nil
}

// --- Workspaces ---

// AgentsService adds agents. Needs token:admin.
type AgentsService struct{ c *Client }

// Create adds an agent member and mints its maidan.agent.worker token
// together; the token's Secret is returned here once and never again.
// displayName may be empty.
func (s *AgentsService) Create(workspaceID, handle, displayName string) (*CreatedAgent, error) {
	body := M{"handle": handle}
	if displayName != "" {
		body["display_name"] = displayName
	}
	return call[CreatedAgent](s.c, http.MethodPost, "/workspaces/"+workspaceID+"/agents", body)
}

type MembersService struct{ c *Client }

func (s *MembersService) List(workspaceID string) ([]Member, error) {
	return callList[Member](s.c, "/workspaces/"+workspaceID+"/members")
}

// MintOptions are the optional fields of a token mint. CapabilitySet is a named
// set ("maidan.agent.worker" / "maidan.human.admin"); combined with capabilities
// it is a progressive grant, so the request must be a subset of the set.
type MintOptions struct {
	Label         string
	CapabilitySet string
	ExpiresAt     string
}

// TokensService mints per-agent bearers. Needs token:admin — the capability the
// admin token from `maidan init` carries.
type TokensService struct{ c *Client }

// Mint returns the secret ONCE, in the response; it is never retrievable again.
func (s *TokensService) Mint(workspaceID, memberID string, capabilities []string, opts *MintOptions) (*MintedToken, error) {
	if capabilities == nil {
		capabilities = []string{}
	}
	body := M{"capabilities": capabilities}
	if opts != nil {
		if opts.Label != "" {
			body["label"] = opts.Label
		}
		if opts.CapabilitySet != "" {
			body["capability_set"] = opts.CapabilitySet
		}
		if opts.ExpiresAt != "" {
			body["expires_at"] = opts.ExpiresAt
		}
	}
	return call[MintedToken](s.c, http.MethodPost, "/workspaces/"+workspaceID+"/members/"+memberID+"/tokens", body)
}

// List returns token metadata only — never a secret.
func (s *TokensService) List(workspaceID, memberID string) ([]TokenSummary, error) {
	return callList[TokenSummary](s.c, "/workspaces/"+workspaceID+"/members/"+memberID+"/tokens")
}

// DelegateOptions are the optional fields of a delegated or attenuated token.
// An empty ExpiresAt takes the longest expiry allowed.
type DelegateOptions struct {
	ExpiresAt string
	Label     string
}

func (o *DelegateOptions) apply(body M) M {
	if o != nil {
		if o.ExpiresAt != "" {
			body["expires_at"] = o.ExpiresAt
		}
		if o.Label != "" {
			body["label"] = o.Label
		}
	}
	return body
}

// Delegate exchanges a delegation grant for a token, called with the
// delegate's own token. The new token acts for the grant's subject with at
// most the grant's capabilities and the caller's, and lives no longer than
// either; empty capabilities asks for every capability both hold. A delegated
// token cannot delegate again. Needs workspace:read, not token:admin.
func (s *TokensService) Delegate(grantID string, capabilities []string, opts *DelegateOptions) (*DelegatedToken, error) {
	if capabilities == nil {
		capabilities = []string{}
	}
	body := opts.apply(M{"grant_id": grantID, "capabilities": capabilities})
	return call[DelegatedToken](s.c, http.MethodPost, "/tokens/delegate", body)
}

// Attenuate derives a weaker token from the caller's: a subset of its
// capabilities, expiring no later than it does.
func (s *TokensService) Attenuate(capabilities []string, opts *DelegateOptions) (*MintedToken, error) {
	if capabilities == nil {
		capabilities = []string{}
	}
	body := opts.apply(M{"capabilities": capabilities})
	return call[MintedToken](s.c, http.MethodPost, "/tokens/attenuate", body)
}

// DelegationGrantsService manages delegation grants: a subject lets a delegate
// act for it, within a capability set, until an expiry. Needs token:admin. The
// delegate exchanges a grant for a token with TokensService.Delegate.
type DelegationGrantsService struct{ c *Client }

// Create grants delegateID the right to act for subjectID. expiresAt is RFC 3339.
func (s *DelegationGrantsService) Create(workspaceID, subjectID, delegateID string, capabilities []string, purpose, expiresAt string) (*DelegationGrant, error) {
	if capabilities == nil {
		capabilities = []string{}
	}
	body := M{
		"subject_id":   subjectID,
		"delegate_id":  delegateID,
		"capabilities": capabilities,
		"purpose":      purpose,
		"expires_at":   expiresAt,
	}
	return call[DelegationGrant](s.c, http.MethodPost, "/workspaces/"+workspaceID+"/delegation-grants", body)
}

// Revoke ends the grant. Tokens already exchanged for it stop working.
func (s *DelegationGrantsService) Revoke(workspaceID, grantID string) (*DelegationGrant, error) {
	return call[DelegationGrant](s.c, http.MethodDelete, "/workspaces/"+workspaceID+"/delegation-grants/"+grantID, nil)
}

// ReviewsService covers who reviews a thread, their verdicts, and whether
// enough approvals are in. Nobody counts as their own reviewer: an owner or
// assignee may submit, but the verdict does not count.
type ReviewsService struct{ c *Client }

// Packet returns the evidence an approval names by its EvidenceRoot.
func (s *ReviewsService) Packet(threadID string) (*ReviewPacket, error) {
	return call[ReviewPacket](s.c, http.MethodGet, "/threads/"+threadID+"/review-packet", nil)
}

// Request designates a member of the thread's workspace as a reviewer.
func (s *ReviewsService) Request(threadID, memberID string) error {
	_, err := s.c.do(http.MethodPost, "/threads/"+threadID+"/reviewers", M{"member_id": memberID})
	return err
}

// ReviewOptions are the optional fields of a verdict. An approval needs
// EvidenceRoot (from Packet); a change request needs Note.
type ReviewOptions struct {
	Note         string
	EvidenceRoot string
	HeadSHA      string
}

// Submit records the caller's verdict. A re-submission replaces the caller's
// current review.
func (s *ReviewsService) Submit(threadID string, decision ReviewDecision, opts *ReviewOptions) (*ThreadReview, error) {
	body := M{"decision": decision}
	if opts != nil {
		if opts.Note != "" {
			body["note"] = opts.Note
		}
		if opts.EvidenceRoot != "" {
			body["evidence_root"] = opts.EvidenceRoot
		}
		if opts.HeadSHA != "" {
			body["head_sha"] = opts.HeadSHA
		}
	}
	return call[ThreadReview](s.c, http.MethodPost, "/threads/"+threadID+"/reviews", body)
}

func (s *ReviewsService) List(threadID string) ([]ThreadReview, error) {
	return callList[ThreadReview](s.c, "/threads/"+threadID+"/reviews")
}

func (s *ReviewsService) Status(threadID string) (*ReviewStatus, error) {
	return call[ReviewStatus](s.c, http.MethodGet, "/threads/"+threadID+"/review-status", nil)
}

// ApprovalGatesService opens and reads approval gates and is their human side.
// Request opens one and Get reads it back; Client.WaitForApprovalDecision waits
// for its answer. List returns the workspace's pending gates, each with the
// RequestState an answer must echo. Accepting needs a person's signed-in
// session or a token holding approval:grant, and nobody accepts a gate they
// asked for; declining or cancelling needs workspace:write.
type ApprovalGatesService struct{ c *Client }

// ApprovalRequestOptions are the optional fields of a new gate. ThreadID holds
// that thread from claim-next while the gate is pending; Risk is "low",
// "medium" or "high" (the server's default when empty).
type ApprovalRequestOptions struct {
	Schema   any
	ThreadID string
	Risk     string
}

// Request opens a gate for a person to answer (message:post), the twin of MCP
// request_approval. The gate is the caller's and starts pending. opts may be nil.
func (s *ApprovalGatesService) Request(workspaceID, prompt string, opts *ApprovalRequestOptions) (*ApprovalGate, error) {
	body := M{"prompt": prompt}
	if opts != nil {
		if opts.Schema != nil {
			body["schema"] = opts.Schema
		}
		if opts.ThreadID != "" {
			body["thread_id"] = opts.ThreadID
		}
		if opts.Risk != "" {
			body["risk"] = opts.Risk
		}
	}
	return call[ApprovalGate](s.c, http.MethodPost, "/workspaces/"+workspaceID+"/approval-gates", body)
}

// Get reads one gate, pending or answered. A gate the caller cannot read is a
// 404, the same as no gate.
func (s *ApprovalGatesService) Get(gateID string) (*ApprovalGate, error) {
	return call[ApprovalGate](s.c, http.MethodGet, "/approval-gates/"+gateID, nil)
}

func (s *ApprovalGatesService) List(workspaceID string) ([]ApprovalGateView, error) {
	return callList[ApprovalGateView](s.c, "/workspaces/"+workspaceID+"/approval-gates")
}

// Answer accepts, declines or cancels a pending gate (action "accept",
// "decline" or "cancel"); content may be nil. A gate already answered is a 409.
func (s *ApprovalGatesService) Answer(gateID, requestState, action string, content any) (*ApprovalGate, error) {
	body := M{"request_state": requestState, "action": action}
	if content != nil {
		body["content"] = content
	}
	return call[ApprovalGate](s.c, http.MethodPost, "/approval-gates/"+gateID+"/answer", body)
}

type WorkspacesService struct{ c *Client }

func (s *WorkspacesService) Get(id string) (*Workspace, error) {
	return call[Workspace](s.c, http.MethodGet, "/workspaces/"+id, nil)
}

// ListEvents is GET /workspaces/{id}/events — projector-shaped HTTP backfill.
func (s *WorkspacesService) ListEvents(id string, query url.Values) ([]StoredEvent, error) {
	return callList[StoredEvent](s.c, "/workspaces/"+id+"/events"+qs(query))
}

// ListEventsAll calls fn for every event after query's after_id, fetching
// query's limit (default 100, at most MaxPageSize) per page. It stops at the
// first error fn returns.
func (s *WorkspacesService) ListEventsAll(id string, query url.Values, fn func(StoredEvent) error) error {
	q := url.Values{}
	for k, v := range query {
		q[k] = append([]string(nil), v...)
	}
	n, _ := strconv.Atoi(q.Get("limit"))
	limit := pageSize(n)
	after, _ := strconv.ParseInt(q.Get("after_id"), 10, 64)
	for {
		q.Set("limit", strconv.Itoa(limit))
		q.Set("after_id", strconv.FormatInt(after, 10))
		page, err := s.ListEvents(id, q)
		if err != nil {
			return err
		}
		for _, row := range page {
			if row.ID > after {
				after = row.ID
			}
			if err := fn(row); err != nil {
				return err
			}
		}
		if len(page) < limit {
			return nil
		}
	}
}

// Import is admin-only (token:admin). bundle is a signed
// maidan.workspace.export/1 envelope; mode "" uses the default (ImportNew).
func (s *WorkspacesService) Import(bundle any, mode ImportMode) (*ImportResult, error) {
	path := "/workspaces/import"
	if mode != "" {
		path += "?mode=" + url.QueryEscape(string(mode))
	}
	return call[ImportResult](s.c, http.MethodPost, path, bundle)
}

// --- Channels ---

type ChannelsService struct{ c *Client }

func (s *ChannelsService) List(workspaceID string) ([]Channel, error) {
	return callList[Channel](s.c, "/workspaces/"+workspaceID+"/channels")
}
func (s *ChannelsService) Create(workspaceID, name string, private bool) (*Channel, error) {
	return call[Channel](s.c, http.MethodPost, "/workspaces/"+workspaceID+"/channels", M{"name": name, "private": private})
}

// Boot is the channel's boot prefix, byte for byte as served, with its sha256.
func (s *ChannelsService) Boot(channelID string) (*BootPrefix, error) {
	b, _, err := s.c.doRaw(http.MethodGet, "/channels/"+channelID+"/boot", nil)
	if err != nil {
		return nil, err
	}
	prefix := NewBootPrefix(b)
	return &prefix, nil
}

// --- Threads ---

type ThreadsService struct{ c *Client }

func (s *ThreadsService) Create(channelID, title string) (*Thread, error) {
	return call[Thread](s.c, http.MethodPost, "/channels/"+channelID+"/threads", M{"title": title})
}
func (s *ThreadsService) Get(id string) (*Thread, error) {
	return call[Thread](s.c, http.MethodGet, "/threads/"+id, nil)
}

// List is GET /channels/{cid}/threads — one page (limit, cursor = last thread id).
func (s *ThreadsService) List(channelID string, query url.Values) ([]Thread, error) {
	return callList[Thread](s.c, "/channels/"+channelID+"/threads"+qs(query))
}

// ListAll calls fn for every live thread in the channel, fetching size
// (default 100, at most MaxPageSize) per request. It stops at the first error
// fn returns.
func (s *ThreadsService) ListAll(channelID string, size int, fn func(Thread) error) error {
	limit := pageSize(size)
	cursor := ""
	for {
		q := url.Values{"limit": {strconv.Itoa(limit)}}
		if cursor != "" {
			q.Set("cursor", cursor)
		}
		page, err := s.List(channelID, q)
		if err != nil {
			return err
		}
		for _, th := range page {
			if err := fn(th); err != nil {
				return err
			}
		}
		if len(page) < limit {
			return nil
		}
		cursor = page[len(page)-1].ID
	}
}
func (s *ThreadsService) Context(id string, query url.Values) (*ThreadContext, error) {
	return call[ThreadContext](s.c, http.MethodGet, "/threads/"+id+"/context"+qs(query), nil)
}

// Transition moves the thread's FSM. action is "start_review", "close" or "archive".
func (s *ThreadsService) Transition(id, action string) (*Thread, error) {
	return call[Thread](s.c, http.MethodPost, "/threads/"+id, M{"action": action})
}

// SetResult writes the result under your live claim: pass the claim's ClaimLeaseID.
func (s *ThreadsService) SetResult(id string, result any, claimLeaseID string) (*ThreadResult, error) {
	return call[ThreadResult](s.c, http.MethodPut, "/threads/"+id+"/result", M{"result": result, "claim_lease_id": claimLeaseID})
}
func (s *ThreadsService) GetResult(id string) (*ThreadResult, error) {
	return call[ThreadResult](s.c, http.MethodGet, "/threads/"+id+"/result", nil)
}

// SetFootprint declares what the thread will touch. claim_next will not hand
// out two threads whose exclusive footprints overlap. Only an open thread
// nobody holds takes one: an ended thread is 409 thread-ended, a deleted one
// 410 gone. The answer is nil when the server stored no footprint.
func (s *ThreadsService) SetFootprint(id string, footprint Footprint) (*Footprint, error) {
	if footprint.Items == nil {
		footprint.Items = []FootprintItem{}
	}
	return call[Footprint](s.c, http.MethodPut, "/threads/"+id+"/footprint", footprint)
}
func (s *ThreadsService) GetFootprint(id string) (*Footprint, error) {
	return call[Footprint](s.c, http.MethodGet, "/threads/"+id+"/footprint", nil)
}

// ClaimOptions are the optional fields of a claim. LeaseSecs 0 takes the
// server's default lease.
type ClaimOptions struct {
	LeaseSecs int64
}

// ClaimNextThread is the hero: readiness/skill/lease-aware claim. Returns nil, nil
// when nothing is claimable.
func (c *Client) ClaimNextThread(channelID string, opts *ClaimOptions) (*ClaimedThread, error) {
	body := M{}
	if opts != nil && opts.LeaseSecs > 0 {
		body["lease_secs"] = opts.LeaseSecs
	}
	claim, err := call[ClaimedThread](c, http.MethodPost, "/channels/"+channelID+"/threads/claim-next", body)
	if err != nil || claim == nil {
		return claim, err
	}
	lease := ""
	if claim.ClaimLeaseID != nil {
		lease = *claim.ClaimLeaseID
	}
	if claim.ID == "" || lease == "" {
		detail := fmt.Sprintf("POST /channels/%s/threads/claim-next: claim without a string id and claim_lease_id", channelID)
		if claim.ID != "" {
			detail += fmt.Sprintf("; thread %s is held until its lease lapses", claim.ID)
		}
		return nil, &UnreadableResponseError{APIError: &APIError{Status: http.StatusOK, Detail: detail}}
	}
	return claim, nil
}

// RenewClaim is the holder-only lease heartbeat.
func (c *Client) RenewClaim(threadID, claimLeaseID string, leaseSecs int64) (*Thread, error) {
	return call[Thread](c, http.MethodPost, "/threads/"+threadID+"/claim/renew", M{
		"claim_lease_id": claimLeaseID,
		"lease_secs":     leaseSecs,
	})
}

// ReleaseOwnClaims is for a waiter that restarted: it releases every live
// claim this token's member holds in the workspace, whose fencing tokens died
// with the old process. Call it once before the first claim, and not while
// another live process shares the token, or that process's claims go too.
// An empty note records the server's default.
func (c *Client) ReleaseOwnClaims(workspaceID, note string) (*ReleasedClaims, error) {
	body := M{}
	if note != "" {
		body["note"] = note
	}
	return call[ReleasedClaims](c, http.MethodPost, "/workspaces/"+workspaceID+"/claims/release-own", body)
}

// --- Messages ---

type MessagesService struct{ c *Client }

func (s *MessagesService) List(threadID string, query url.Values) ([]Message, error) {
	return callList[Message](s.c, "/threads/"+threadID+"/messages"+qs(query))
}
func (s *MessagesService) Post(threadID, body string) (*Message, error) {
	return call[Message](s.c, http.MethodPost, "/threads/"+threadID+"/messages", M{"body": body})
}

// --- Artifacts ---

type ArtifactsService struct{ c *Client }

func (s *ArtifactsService) Upload(data []byte, kind ArtifactKind) (*Artifact, error) {
	path := "/artifacts?kind=" + url.QueryEscape(string(kind))
	_, raw, err := s.c.doRaw(http.MethodPost, path, data)
	if err != nil || raw == nil {
		return nil, err
	}
	var a Artifact
	if err := decodeJSON(raw, &a); err != nil {
		return nil, &UnreadableResponseError{
			APIError: &APIError{Status: http.StatusOK, Detail: fmt.Sprintf("maidan: decoding POST %s: %v", path, err)},
			Cause:    err,
		}
	}
	return &a, nil
}
func (s *ArtifactsService) Get(sha string) ([]byte, error) {
	b, _, err := s.c.doRaw(http.MethodGet, "/artifacts/"+sha, nil)
	return b, err
}
func (s *ArtifactsService) Meta(sha string) (*Artifact, error) {
	return call[Artifact](s.c, http.MethodGet, "/artifacts/"+sha+"/meta", nil)
}

func qs(query url.Values) string {
	if len(query) == 0 {
		return ""
	}
	return "?" + query.Encode()
}
