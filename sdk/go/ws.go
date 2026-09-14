package maidan

import (
	"bufio"
	"crypto/rand"
	"crypto/tls"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"fmt"
	"io"
	"net"
	"net/url"
	"strconv"
	"strings"
	"sync"
	"time"
)

// Subscription is a live WebSocket subscription handle. Call Close to stop it.
type Subscription struct {
	conn    net.Conn
	closed  chan struct{}
	once    sync.Once
	writeMu sync.Mutex
}

// Close stops the subscription and closes the socket. Safe to call more than once.
func (s *Subscription) Close() error {
	s.once.Do(func() {
		close(s.closed)
		s.writeFrame(0x8, nil) // best-effort close frame
		_ = s.conn.Close()
	})
	return nil
}

// Subscribe opens a WebSocket to /ws/subscribe. filter follows
// contracts/ws-subscribe-filter.schema.json (set workspace_id for replay). Control
// frames (subscribe_ack, schema_version, replay_*) are skipped; each domain event
// is passed to onEvent. Unknown kinds are still delivered (forward-compat).
func (c *Client) Subscribe(filter M, onEvent func(Event), onError func(error)) (*Subscription, error) {
	return c.SubscribeFrom(filter, 0, "", onEvent, onError)
}

// SubscribeFrom is Subscribe with an explicit after_id / durable consumer_id on
// the subscribe frame (siblings of filter, not inside it).
func (c *Client) SubscribeFrom(filter M, afterID int64, consumerID string, onEvent func(Event), onError func(error)) (*Subscription, error) {
	conn, br, err := wsDial(c.BaseURL, "/ws/subscribe")
	if err != nil {
		return nil, err
	}
	sub := &Subscription{conn: conn, closed: make(chan struct{})}
	if filter == nil {
		filter = M{}
	}
	frame := M{"filter": filter, "token": c.Token}
	if afterID > 0 {
		frame["after_id"] = afterID
	}
	if consumerID != "" {
		frame["consumer_id"] = consumerID
	}
	payload, err := json.Marshal(frame)
	if err != nil {
		_ = conn.Close()
		return nil, err
	}
	if err := sub.writeFrame(0x1, payload); err != nil {
		_ = conn.Close()
		return nil, err
	}
	go sub.readLoop(br, onEvent, onError)
	return sub, nil
}

func (c *Client) waitForKind(filter M, kind string, timeout time.Duration) (Event, error) {
	f := M{}
	for k, v := range filter {
		f[k] = v
	}
	f["kinds"] = []string{kind}
	ch := make(chan Event, 1)
	sub, err := c.Subscribe(f, func(e Event) {
		select {
		case ch <- e:
		default:
		}
	}, nil)
	if err != nil {
		return nil, err
	}
	defer sub.Close()
	select {
	case e := <-ch:
		return e, nil
	case <-time.After(timeout):
		return nil, nil
	}
}

// WaitForResult blocks until the thread's result is set, or timeout (nil, nil).
func (c *Client) WaitForResult(threadID, workspaceID string, timeout time.Duration) (Event, error) {
	return c.waitForKind(M{"workspace_id": workspaceID, "thread_id": threadID}, "thread_result_set", timeout)
}

// WaitForMention blocks until the member is mentioned, or timeout (nil, nil).
func (c *Client) WaitForMention(memberID, workspaceID string, timeout time.Duration) (Event, error) {
	return c.waitForKind(M{"workspace_id": workspaceID, "member_id": memberID}, "mention_recorded", timeout)
}

// WaitForReady blocks until a task becomes claimable, or timeout (nil, nil).
// channelID "" scopes to the whole workspace.
func (c *Client) WaitForReady(workspaceID, channelID string, timeout time.Duration) (Event, error) {
	f := M{"workspace_id": workspaceID}
	if channelID != "" {
		f["channel_id"] = channelID
	}
	return c.waitForKind(f, "thread_ready", timeout)
}

// Follow is the projector shape for HTTP backfill then WS cutover.
type Follow struct {
	WorkspaceID string
	ChannelID   string
	ThreadID    string
	Types       []string
	ConsumerID  string
	AfterID     int64
	PageLimit   int64
}

// FollowLog pages GET /workspaces/{id}/events then cuts over to SubscribeFrom.
// A 409 must_refetch is returned as-is (*APIError.IsCursorTooOld) — never clamped.
func (c *Client) FollowLog(spec Follow, onEvent func(Event), onError func(error)) (*Subscription, error) {
	limit := spec.PageLimit
	if limit <= 0 {
		limit = 100
	}
	after := spec.AfterID
	for {
		q := url.Values{}
		q.Set("after_id", strconv.FormatInt(after, 10))
		q.Set("limit", strconv.FormatInt(limit, 10))
		if spec.ChannelID != "" {
			q.Set("channel_id", spec.ChannelID)
		}
		if spec.ThreadID != "" {
			q.Set("thread_id", spec.ThreadID)
		}
		if len(spec.Types) > 0 {
			q.Set("types", strings.Join(spec.Types, ","))
		}
		if spec.ConsumerID != "" {
			q.Set("consumer_id", spec.ConsumerID)
		}
		page, err := c.Workspaces.ListEvents(spec.WorkspaceID, q)
		if err != nil {
			return nil, err
		}
		if len(page) == 0 {
			break
		}
		for _, row := range page {
			if id, ok := storedID(row); ok && id > after {
				after = id
			}
			onEvent(normalizeStored(row))
		}
		if int64(len(page)) < limit {
			break
		}
	}
	f := M{"workspace_id": spec.WorkspaceID}
	if spec.ChannelID != "" {
		f["channel_id"] = spec.ChannelID
	}
	if spec.ThreadID != "" {
		f["thread_id"] = spec.ThreadID
	}
	if len(spec.Types) > 0 {
		f["kinds"] = spec.Types
	}
	return c.SubscribeFrom(f, after, spec.ConsumerID, onEvent, onError)
}

func storedID(row M) (int64, bool) {
	for _, key := range []string{"id", "log_id"} {
		switch v := row[key].(type) {
		case float64:
			return int64(v), true
		case int64:
			return v, true
		case json.Number:
			n, err := v.Int64()
			return n, err == nil
		}
	}
	return 0, false
}

func normalizeStored(row M) Event {
	out := M{}
	if payload, ok := row["payload"].(map[string]any); ok {
		for k, v := range payload {
			out[k] = v
		}
	} else {
		for k, v := range row {
			out[k] = v
		}
	}
	if id, ok := storedID(row); ok {
		out["log_id"] = id
	}
	for _, key := range []string{"kind", "workspace_id", "channel_id", "thread_id"} {
		if _, exists := out[key]; !exists {
			if v, ok := row[key]; ok {
				out[key] = v
			}
		}
	}
	return out
}

func (s *Subscription) readLoop(br *bufio.Reader, onEvent func(Event), onError func(error)) {
	var msg []byte
	var msgOp byte
	for {
		select {
		case <-s.closed:
			return
		default:
		}
		fin, opcode, payload, err := wsReadFrame(br)
		if err != nil {
			select {
			case <-s.closed:
			default:
				if onError != nil {
					onError(err)
				}
			}
			return
		}
		switch opcode {
		case 0x8: // close
			return
		case 0x9: // ping -> pong
			_ = s.writeFrame(0xA, payload)
			continue
		case 0xA: // pong
			continue
		case 0x1, 0x2: // text / binary
			msg = append([]byte(nil), payload...)
			msgOp = opcode
		case 0x0: // continuation
			msg = append(msg, payload...)
		}
		if fin {
			if msgOp == 0x1 {
				var frame map[string]any
				if json.Unmarshal(msg, &frame) == nil {
					if t, _ := frame["type"].(string); t == "cursor_too_old" {
						onEvent(frame)
						return
					} else if t == "" {
						if k, ok := frame["kind"].(string); ok && k != "" {
							onEvent(frame)
						}
					}
				}
			}
			msg = nil
			msgOp = 0
		}
	}
}

// wsDial performs the RFC-6455 client handshake and returns the connection plus a
// buffered reader positioned at the first frame.
func wsDial(baseURL, path string) (net.Conn, *bufio.Reader, error) {
	u, err := url.Parse(baseURL)
	if err != nil {
		return nil, nil, err
	}
	secure := u.Scheme == "https" || u.Scheme == "wss"
	host := u.Hostname()
	port := u.Port()
	if port == "" {
		if secure {
			port = "443"
		} else {
			port = "80"
		}
	}
	addr := net.JoinHostPort(host, port)

	var conn net.Conn
	if secure {
		conn, err = tls.Dial("tcp", addr, &tls.Config{ServerName: host})
	} else {
		conn, err = net.Dial("tcp", addr)
	}
	if err != nil {
		return nil, nil, err
	}

	key := make([]byte, 16)
	if _, err := rand.Read(key); err != nil {
		_ = conn.Close()
		return nil, nil, err
	}
	req := fmt.Sprintf(
		"GET %s HTTP/1.1\r\nHost: %s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"+
			"Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n",
		path, addr, base64.StdEncoding.EncodeToString(key),
	)
	if _, err := conn.Write([]byte(req)); err != nil {
		_ = conn.Close()
		return nil, nil, err
	}

	br := bufio.NewReader(conn)
	status, err := br.ReadString('\n')
	if err != nil {
		_ = conn.Close()
		return nil, nil, err
	}
	if !strings.Contains(status, "101") {
		_ = conn.Close()
		return nil, nil, &APIError{Status: 0, Body: strings.TrimSpace(status)}
	}
	for { // drain headers to the blank line
		line, err := br.ReadString('\n')
		if err != nil {
			_ = conn.Close()
			return nil, nil, err
		}
		if line == "\r\n" || line == "\n" {
			break
		}
	}
	return conn, br, nil
}

func wsReadFrame(br *bufio.Reader) (fin bool, opcode byte, payload []byte, err error) {
	h := make([]byte, 2)
	if _, err = io.ReadFull(br, h); err != nil {
		return
	}
	fin = h[0]&0x80 != 0
	opcode = h[0] & 0x0F
	masked := h[1]&0x80 != 0
	length := uint64(h[1] & 0x7F)
	switch length {
	case 126:
		ext := make([]byte, 2)
		if _, err = io.ReadFull(br, ext); err != nil {
			return
		}
		length = uint64(binary.BigEndian.Uint16(ext))
	case 127:
		ext := make([]byte, 8)
		if _, err = io.ReadFull(br, ext); err != nil {
			return
		}
		length = binary.BigEndian.Uint64(ext)
	}
	var mask []byte
	if masked {
		mask = make([]byte, 4)
		if _, err = io.ReadFull(br, mask); err != nil {
			return
		}
	}
	payload = make([]byte, length)
	if length > 0 {
		if _, err = io.ReadFull(br, payload); err != nil {
			return
		}
	}
	if masked {
		for i := range payload {
			payload[i] ^= mask[i%4]
		}
	}
	return
}

func (s *Subscription) writeFrame(opcode byte, payload []byte) error {
	header := []byte{0x80 | opcode}
	length := len(payload)
	switch {
	case length < 126:
		header = append(header, 0x80|byte(length))
	case length < 1<<16:
		header = append(header, 0x80|126)
		ext := make([]byte, 2)
		binary.BigEndian.PutUint16(ext, uint16(length))
		header = append(header, ext...)
	default:
		header = append(header, 0x80|127)
		ext := make([]byte, 8)
		binary.BigEndian.PutUint64(ext, uint64(length))
		header = append(header, ext...)
	}
	mask := make([]byte, 4)
	if _, err := rand.Read(mask); err != nil {
		return err
	}
	header = append(header, mask...)
	masked := make([]byte, length)
	for i := range payload {
		masked[i] = payload[i] ^ mask[i%4]
	}
	s.writeMu.Lock()
	defer s.writeMu.Unlock()
	_, err := s.conn.Write(append(header, masked...))
	return err
}
