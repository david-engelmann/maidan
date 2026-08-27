// Package maidan is the official Go client for Maidan, the operating layer for
// teams of AI agents. It speaks REST + WebSocket (MCP is a URL, not a dependency;
// A2A is a recipe) and is dependency-free — standard library only. See the repo's
// docs/Client Contract.md for the frozen v1 surface.
package maidan

import (
	"bytes"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"net/url"
	"os"
	"strconv"
	"strings"
	"time"
)

// Version is the client version, tracked independently of the server.
const Version = "0.1.0"

// M is a decoded JSON object. Responses are returned as M (or []M) so unknown
// fields are preserved and ignored (forward-compat), per the contract.
type M = map[string]any

// Event is a bus event frame delivered to a Subscribe callback.
type Event = map[string]any

// APIError is a failed request: it carries the HTTP status and the parsed body.
type APIError struct {
	Status     int
	Body       any
	RetryAfter float64 // seconds, from Retry-After on a 429
}

func (e *APIError) Error() string {
	return fmt.Sprintf("maidan: request failed: HTTP %d", e.Status)
}

// IsConflict reports a 409.
func (e *APIError) IsConflict() bool { return e.Status == 409 }

// IsForbidden reports a 403 (missing capability / channel access — not retryable).
func (e *APIError) IsForbidden() bool { return e.Status == 403 }

// IsRateLimited reports a 429 (server rate limit).
func (e *APIError) IsRateLimited() bool { return e.Status == 429 }

// Client is a Maidan v1 client over REST + WebSocket.
type Client struct {
	BaseURL string
	Token   string
	// MCPURL is {BaseURL}/mcp/streamable — a string only, no MCP dependency.
	MCPURL string
	HTTP   *http.Client

	Workspaces *WorkspacesService
	Channels   *ChannelsService
	Threads    *ThreadsService
	Messages   *MessagesService
	Artifacts  *ArtifactsService
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
	}
	c.Workspaces = &WorkspacesService{c}
	c.Channels = &ChannelsService{c}
	c.Threads = &ThreadsService{c}
	c.Messages = &MessagesService{c}
	c.Artifacts = &ArtifactsService{c}
	return c
}

// do sends a JSON request and returns the raw response body (nil on 204/empty).
func (c *Client) do(method, path string, body any) (json.RawMessage, error) {
	var reader io.Reader
	if body != nil {
		b, err := json.Marshal(body)
		if err != nil {
			return nil, err
		}
		reader = bytes.NewReader(b)
	}
	req, err := http.NewRequest(method, c.BaseURL+path, reader)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.Token)
	if body != nil {
		req.Header.Set("Content-Type", "application/json")
	}
	resp, err := c.HTTP.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
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
	var reader io.Reader
	if body != nil {
		reader = bytes.NewReader(body)
	}
	req, err := http.NewRequest(method, c.BaseURL+path, reader)
	if err != nil {
		return nil, nil, err
	}
	req.Header.Set("Authorization", "Bearer "+c.Token)
	resp, err := c.HTTP.Do(req)
	if err != nil {
		return nil, nil, err
	}
	defer resp.Body.Close()
	raw, _ := io.ReadAll(resp.Body)
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

func apiError(resp *http.Response, raw []byte) *APIError {
	var parsed any
	if len(raw) > 0 {
		if err := json.Unmarshal(raw, &parsed); err != nil {
			parsed = string(raw)
		}
	}
	e := &APIError{Status: resp.StatusCode, Body: parsed}
	if resp.StatusCode == http.StatusTooManyRequests {
		if ra := resp.Header.Get("Retry-After"); ra != "" {
			if f, err := strconv.ParseFloat(ra, 64); err == nil {
				e.RetryAfter = f
			}
		}
	}
	return e
}

func decodeObj(raw json.RawMessage) (M, error) {
	if raw == nil {
		return nil, nil
	}
	var m M
	if err := json.Unmarshal(raw, &m); err != nil {
		return nil, err
	}
	return m, nil
}

func decodeArr(raw json.RawMessage) ([]M, error) {
	if raw == nil {
		return nil, nil
	}
	var a []M
	if err := json.Unmarshal(raw, &a); err != nil {
		return nil, err
	}
	return a, nil
}

func (c *Client) getObj(path string) (M, error) {
	raw, err := c.do(http.MethodGet, path, nil)
	if err != nil {
		return nil, err
	}
	return decodeObj(raw)
}

func (c *Client) postObj(path string, body any) (M, error) {
	raw, err := c.do(http.MethodPost, path, body)
	if err != nil {
		return nil, err
	}
	return decodeObj(raw)
}

// --- Workspaces ---

type WorkspacesService struct{ c *Client }

func (s *WorkspacesService) Create(name string) (M, error) {
	return s.c.postObj("/workspaces", M{"name": name})
}
func (s *WorkspacesService) Get(id string) (M, error) {
	return s.c.getObj("/workspaces/" + id)
}

// Import is admin-only (token:admin). mode "" uses the default (new).
func (s *WorkspacesService) Import(bundle any, mode string) (M, error) {
	path := "/workspaces/import"
	if mode != "" {
		path += "?mode=" + url.QueryEscape(mode)
	}
	return s.c.postObj(path, bundle)
}

// --- Channels ---

type ChannelsService struct{ c *Client }

func (s *ChannelsService) List(workspaceID string) ([]M, error) {
	raw, err := s.c.do(http.MethodGet, "/workspaces/"+workspaceID+"/channels", nil)
	if err != nil {
		return nil, err
	}
	return decodeArr(raw)
}
func (s *ChannelsService) Create(workspaceID, name string, private bool) (M, error) {
	return s.c.postObj("/workspaces/"+workspaceID+"/channels", M{"name": name, "private": private})
}

// --- Threads ---

type ThreadsService struct{ c *Client }

func (s *ThreadsService) Create(channelID, title string) (M, error) {
	return s.c.postObj("/channels/"+channelID+"/threads", M{"title": title})
}
func (s *ThreadsService) Get(id string) (M, error) { return s.c.getObj("/threads/" + id) }
func (s *ThreadsService) Context(id string, query url.Values) (M, error) {
	return s.c.getObj("/threads/" + id + "/context" + qs(query))
}
func (s *ThreadsService) Transition(id string, body any) (M, error) {
	return s.c.postObj("/threads/"+id, body)
}
func (s *ThreadsService) SetResult(id string, result any) (M, error) {
	raw, err := s.c.do(http.MethodPut, "/threads/"+id+"/result", M{"result": result})
	if err != nil {
		return nil, err
	}
	return decodeObj(raw)
}
func (s *ThreadsService) GetResult(id string) (M, error) {
	return s.c.getObj("/threads/" + id + "/result")
}

// ClaimNextThread is the hero: readiness/skill/lease-aware claim. Returns nil, nil
// when nothing is claimable.
func (c *Client) ClaimNextThread(channelID string, body M) (M, error) {
	if body == nil {
		body = M{}
	}
	return c.postObj("/channels/"+channelID+"/threads/claim-next", body)
}

// RenewClaim is the holder-only lease heartbeat.
func (c *Client) RenewClaim(threadID string) (M, error) {
	return c.postObj("/threads/"+threadID+"/claim/renew", M{})
}

// --- Messages ---

type MessagesService struct{ c *Client }

func (s *MessagesService) List(threadID string, query url.Values) ([]M, error) {
	raw, err := s.c.do(http.MethodGet, "/threads/"+threadID+"/messages"+qs(query), nil)
	if err != nil {
		return nil, err
	}
	return decodeArr(raw)
}
func (s *MessagesService) Post(threadID, authorID, body string) (M, error) {
	return s.c.postObj("/threads/"+threadID+"/messages", M{"author_id": authorID, "body": body})
}

// --- Artifacts ---

type ArtifactsService struct{ c *Client }

func (s *ArtifactsService) Upload(data []byte, kind string) (M, error) {
	_, raw, err := s.c.doRaw(http.MethodPost, "/artifacts?kind="+url.QueryEscape(kind), data)
	if err != nil {
		return nil, err
	}
	return decodeObj(raw)
}
func (s *ArtifactsService) Get(sha string) ([]byte, error) {
	b, _, err := s.c.doRaw(http.MethodGet, "/artifacts/"+sha, nil)
	return b, err
}
func (s *ArtifactsService) Meta(sha string) (M, error) {
	return s.c.getObj("/artifacts/" + sha + "/meta")
}

func qs(query url.Values) string {
	if len(query) == 0 {
		return ""
	}
	return "?" + query.Encode()
}
