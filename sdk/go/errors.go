package maidan

import (
	"encoding/json"
	"errors"
	"fmt"
	"net/http"
	"strconv"
)

// ProblemBase is the URI prefix of every problem type the server emits (RFC 9457).
const ProblemBase = "https://maidan.dev/problems/"

// APIError is a failed request. Type, Title and Detail come from the server's
// problem body; Problem is that body as sent, unknown members included (nil
// when the body was not a JSON object, in which case Detail holds its text).
//
// The client returns one of the per-type errors below, each wrapping an
// *APIError, so either form works with errors.As:
//
//	var nf *maidan.NotFoundError   // this problem type only
//	var api *maidan.APIError       // any failed request
type APIError struct {
	Status  int
	Type    string
	Title   string
	Detail  string
	Problem map[string]any
	// RetryAfter is the Retry-After header in seconds (sent on 429 and 503).
	RetryAfter float64
}

func (e *APIError) Error() string {
	if e.Detail != "" {
		return fmt.Sprintf("maidan: request failed: HTTP %d: %s", e.Status, e.Detail)
	}
	return fmt.Sprintf("maidan: request failed: HTTP %d", e.Status)
}

// IsConflict reports a 409.
func (e *APIError) IsConflict() bool { return e.Status == 409 }

// IsCursorTooOld reports a 409 must_refetch / cursor-too-old — fail loud, never clamp.
func (e *APIError) IsCursorTooOld() bool {
	if e.Status != 409 {
		return false
	}
	if v, ok := e.Problem["must_refetch"].(bool); ok && v {
		return true
	}
	return e.Type == ProblemBase+"cursor-too-old" || e.Type == "cursor_too_old"
}

// IsForbidden reports a 403 (missing capability / channel access — not retryable).
func (e *APIError) IsForbidden() bool { return e.Status == 403 }

// IsRateLimited reports a 429 (server rate limit).
func (e *APIError) IsRateLimited() bool { return e.Status == 429 }

// NotFoundError is a 404 not-found.
type NotFoundError struct{ *APIError }

// MethodNotAllowedError is a 405 method-not-allowed.
type MethodNotAllowedError struct{ *APIError }

// ConflictError is a 409 conflict: the resource's state refuses the change.
type ConflictError struct{ *APIError }

// BadRequestError is a 400 bad-request.
type BadRequestError struct{ *APIError }

// UnauthorizedError is a 401 unauthorized: missing or invalid bearer token.
type UnauthorizedError struct{ *APIError }

// InvalidSignatureError is a 401 invalid-signature (webhook ingress).
type InvalidSignatureError struct{ *APIError }

// ForbiddenError is a 403 forbidden: a missing capability or channel access.
type ForbiddenError struct{ *APIError }

// PayloadTooLargeError is a 413 payload-too-large.
type PayloadTooLargeError struct{ *APIError }

// UnsupportedMediaTypeError is a 415 unsupported-media-type.
type UnsupportedMediaTypeError struct{ *APIError }

// RateLimitedError is a 429 rate-limited; see RetryAfter.
type RateLimitedError struct{ *APIError }

// BadGatewayError is a 502 bad-gateway.
type BadGatewayError struct{ *APIError }

// InternalError is a 500 internal.
type InternalError struct{ *APIError }

// OverloadedError is a 503 overloaded: refused without running; retry after RetryAfter.
type OverloadedError struct{ *APIError }

// IdempotencyKeyReusedError is a 422 idempotency-key-reused: the key was used
// for a different request.
type IdempotencyKeyReusedError struct{ *APIError }

// IdempotencyKeyInFlightError is a 409 idempotency-key-in-flight: the first
// request with the key still runs.
type IdempotencyKeyInFlightError struct{ *APIError }

// CursorTooOldError is a 409 cursor-too-old: refetch from Snapshot, never clamp.
type CursorTooOldError struct{ *APIError }

// Snapshot is the path of the snapshot covering the pruned prefix, if sent.
func (e *CursorTooOldError) Snapshot() string {
	s, _ := e.Problem["snapshot"].(string)
	return s
}

// EventLogBrokenError is a 409 event-log-broken: the hash chain failed verification.
type EventLogBrokenError struct{ *APIError }

// UnknownProblemError is a problem type this client does not know, or a body
// that is not a problem.
type UnknownProblemError struct{ *APIError }

func (e *NotFoundError) Unwrap() error               { return e.APIError }
func (e *MethodNotAllowedError) Unwrap() error       { return e.APIError }
func (e *ConflictError) Unwrap() error               { return e.APIError }
func (e *BadRequestError) Unwrap() error             { return e.APIError }
func (e *UnauthorizedError) Unwrap() error           { return e.APIError }
func (e *InvalidSignatureError) Unwrap() error       { return e.APIError }
func (e *ForbiddenError) Unwrap() error              { return e.APIError }
func (e *PayloadTooLargeError) Unwrap() error        { return e.APIError }
func (e *UnsupportedMediaTypeError) Unwrap() error   { return e.APIError }
func (e *RateLimitedError) Unwrap() error            { return e.APIError }
func (e *BadGatewayError) Unwrap() error             { return e.APIError }
func (e *InternalError) Unwrap() error               { return e.APIError }
func (e *OverloadedError) Unwrap() error             { return e.APIError }
func (e *IdempotencyKeyReusedError) Unwrap() error   { return e.APIError }
func (e *IdempotencyKeyInFlightError) Unwrap() error { return e.APIError }
func (e *CursorTooOldError) Unwrap() error           { return e.APIError }
func (e *EventLogBrokenError) Unwrap() error         { return e.APIError }
func (e *UnknownProblemError) Unwrap() error         { return e.APIError }

// ProblemTypes maps each problem type the server documents to the error the
// client returns for it.
var ProblemTypes = map[string]func(*APIError) error{
	ProblemBase + "not-found":                 func(e *APIError) error { return &NotFoundError{e} },
	ProblemBase + "method-not-allowed":        func(e *APIError) error { return &MethodNotAllowedError{e} },
	ProblemBase + "conflict":                  func(e *APIError) error { return &ConflictError{e} },
	ProblemBase + "bad-request":               func(e *APIError) error { return &BadRequestError{e} },
	ProblemBase + "unauthorized":              func(e *APIError) error { return &UnauthorizedError{e} },
	ProblemBase + "invalid-signature":         func(e *APIError) error { return &InvalidSignatureError{e} },
	ProblemBase + "forbidden":                 func(e *APIError) error { return &ForbiddenError{e} },
	ProblemBase + "payload-too-large":         func(e *APIError) error { return &PayloadTooLargeError{e} },
	ProblemBase + "unsupported-media-type":    func(e *APIError) error { return &UnsupportedMediaTypeError{e} },
	ProblemBase + "rate-limited":              func(e *APIError) error { return &RateLimitedError{e} },
	ProblemBase + "bad-gateway":               func(e *APIError) error { return &BadGatewayError{e} },
	ProblemBase + "internal":                  func(e *APIError) error { return &InternalError{e} },
	ProblemBase + "overloaded":                func(e *APIError) error { return &OverloadedError{e} },
	ProblemBase + "idempotency-key-reused":    func(e *APIError) error { return &IdempotencyKeyReusedError{e} },
	ProblemBase + "idempotency-key-in-flight": func(e *APIError) error { return &IdempotencyKeyInFlightError{e} },
	ProblemBase + "cursor-too-old":            func(e *APIError) error { return &CursorTooOldError{e} },
	ProblemBase + "event-log-broken":          func(e *APIError) error { return &EventLogBrokenError{e} },
}

// NewProblemError builds the error for a failed response from its status and
// body: the type its problem names, or *UnknownProblemError.
func NewProblemError(status int, raw []byte) error {
	e := &APIError{Status: status}
	var problem map[string]any
	if json.Unmarshal(raw, &problem) == nil && problem != nil {
		e.Problem = problem
		e.Type, _ = problem["type"].(string)
		e.Title, _ = problem["title"].(string)
		e.Detail, _ = problem["detail"].(string)
	} else {
		e.Detail = string(raw)
	}
	if wrap, ok := ProblemTypes[e.Type]; ok {
		return wrap(e)
	}
	return &UnknownProblemError{e}
}

func apiError(resp *http.Response, raw []byte) error {
	err := NewProblemError(resp.StatusCode, raw)
	if ra := resp.Header.Get("Retry-After"); ra != "" {
		var base *APIError
		if f, perr := strconv.ParseFloat(ra, 64); perr == nil && errors.As(err, &base) {
			base.RetryAfter = f
		}
	}
	return err
}
