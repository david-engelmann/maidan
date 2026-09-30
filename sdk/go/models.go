package maidan

import (
	"encoding/json"
	"time"
)

// Response models, from the server's OpenAPI schemas. encoding/json ignores
// members a struct does not declare, so a field added to the server does not
// break a client. A string enum is a named string type: the constants are the
// values the server sends today, and any other value still decodes.

// ThreadState is a thread's FSM state.
type ThreadState string

const (
	ThreadOpen     ThreadState = "open"
	ThreadInReview ThreadState = "in_review"
	ThreadClosed   ThreadState = "closed"
	ThreadArchived ThreadState = "archived"
)

// MemberKind says whether a member is a person or an agent.
type MemberKind string

const (
	MemberHuman MemberKind = "human"
	MemberAgent MemberKind = "agent"
)

// ArtifactKind classifies an uploaded artifact.
type ArtifactKind string

const (
	ArtifactScreenshot      ArtifactKind = "screenshot"
	ArtifactRecording       ArtifactKind = "recording"
	ArtifactTranscript      ArtifactKind = "transcript"
	ArtifactCodeDump        ArtifactKind = "code_dump"
	ArtifactAttachment      ArtifactKind = "attachment"
	ArtifactContextSnapshot ArtifactKind = "context_snapshot"
)

// ImportMode is how a workspace import treats ids: "new" remaps them,
// "restore" keeps them.
type ImportMode string

const (
	ImportNew     ImportMode = "new"
	ImportRestore ImportMode = "restore"
)

// RefSide is one end of a Reference.
type RefSide string

const (
	RefThread  RefSide = "thread"
	RefMessage RefSide = "message"
)

// ReviewDecision is a reviewer's verdict.
type ReviewDecision string

const (
	ReviewApprove        ReviewDecision = "approve"
	ReviewRequestChanges ReviewDecision = "request_changes"
)

type Workspace struct {
	ID           string     `json:"id"`
	Name         string     `json:"name"`
	CreatedAt    time.Time  `json:"created_at"`
	UpdatedAt    time.Time  `json:"updated_at"`
	TombstonedAt *time.Time `json:"tombstoned_at,omitempty"`
}

type ImportResult struct {
	WorkspaceID string     `json:"workspace_id"`
	Mode        ImportMode `json:"mode"`
}

type Member struct {
	ID           string     `json:"id"`
	WorkspaceID  string     `json:"workspace_id"`
	Handle       string     `json:"handle"`
	Kind         MemberKind `json:"kind"`
	DisplayName  *string    `json:"display_name,omitempty"`
	CreatedAt    time.Time  `json:"created_at"`
	UpdatedAt    time.Time  `json:"updated_at"`
	TombstonedAt *time.Time `json:"tombstoned_at,omitempty"`
}

type TokenQuota struct {
	Capability   string `json:"capability"`
	MaxPerWindow int    `json:"max_per_window"`
	WindowSecs   int64  `json:"window_secs"`
}

// MintedToken is a mint's answer. Secret is returned here once and never again.
type MintedToken struct {
	ID           string       `json:"id"`
	Secret       string       `json:"secret"`
	WorkspaceID  string       `json:"workspace_id"`
	MemberID     string       `json:"member_id"`
	Capabilities []string     `json:"capabilities"`
	ExpiresAt    *time.Time   `json:"expires_at,omitempty"`
	Quotas       []TokenQuota `json:"quotas"`
}

// TokenSummary is token metadata; it never carries the secret.
type TokenSummary struct {
	ID           string     `json:"id"`
	WorkspaceID  string     `json:"workspace_id"`
	MemberID     string     `json:"member_id"`
	Label        *string    `json:"label,omitempty"`
	Capabilities []string   `json:"capabilities"`
	CreatedAt    time.Time  `json:"created_at"`
	ExpiresAt    *time.Time `json:"expires_at,omitempty"`
	RevokedAt    *time.Time `json:"revoked_at,omitempty"`
}

type Channel struct {
	ID           string     `json:"id"`
	WorkspaceID  string     `json:"workspace_id"`
	Name         string     `json:"name"`
	Private      bool       `json:"private"`
	Topic        *string    `json:"topic,omitempty"`
	CreatedAt    time.Time  `json:"created_at"`
	UpdatedAt    time.Time  `json:"updated_at"`
	TombstonedAt *time.Time `json:"tombstoned_at,omitempty"`
}

type Thread struct {
	ID                  string      `json:"id"`
	ChannelID           string      `json:"channel_id"`
	ParentThreadID      *string     `json:"parent_thread_id,omitempty"`
	Title               *string     `json:"title,omitempty"`
	State               ThreadState `json:"state"`
	AssigneeID          *string     `json:"assignee_id,omitempty"`
	OwnerID             *string     `json:"owner_id,omitempty"`
	AssignmentExpiresAt *time.Time  `json:"assignment_expires_at,omitempty"`
	// ClaimLeaseID is the fencing token RenewClaim takes.
	ClaimLeaseID  *string    `json:"claim_lease_id,omitempty"`
	WorkStartedAt *time.Time `json:"work_started_at,omitempty"`
	CreatedAt     time.Time  `json:"created_at"`
	UpdatedAt     time.Time  `json:"updated_at"`
	TombstonedAt  *time.Time `json:"tombstoned_at,omitempty"`
}

// StrongRef is a content-addressed pin (maidan:event/{id} plus its hash).
type StrongRef struct {
	URI         string `json:"uri"`
	ContentHash string `json:"content_hash"`
}

// ClaimedThread is a claim: the thread's fields at the top level, plus the pin.
type ClaimedThread struct {
	Thread
	Pin StrongRef `json:"pin"`
}

type ThreadResult struct {
	ThreadID string `json:"thread_id"`
	// Result is the producer's JSON as it was set; unmarshal it into your type.
	Result     json.RawMessage `json:"result"`
	ProducedBy string          `json:"produced_by"`
	ProducedAt time.Time       `json:"produced_at"`
}

// ContentBlock is a structured message block, discriminated by Type ("text",
// "code", "tool_use", "tool_result", "resource_link"); each type sets its own
// fields.
type ContentBlock struct {
	Type      string          `json:"type"`
	Text      string          `json:"text,omitempty"`
	Code      string          `json:"code,omitempty"`
	Language  *string         `json:"language,omitempty"`
	ID        string          `json:"id,omitempty"`
	Name      string          `json:"name,omitempty"`
	Input     json.RawMessage `json:"input,omitempty"`
	ToolUseID string          `json:"tool_use_id,omitempty"`
	Content   string          `json:"content,omitempty"`
	IsError   bool            `json:"is_error,omitempty"`
	URI       string          `json:"uri,omitempty"`
	MimeType  *string         `json:"mime_type,omitempty"`
	Title     *string         `json:"title,omitempty"`
}

type Message struct {
	ID           string         `json:"id"`
	ThreadID     string         `json:"thread_id"`
	AuthorID     string         `json:"author_id"`
	Body         string         `json:"body"`
	Content      []ContentBlock `json:"content,omitempty"`
	Metadata     map[string]any `json:"metadata,omitempty"`
	PostedAt     time.Time      `json:"posted_at"`
	EditedAt     *time.Time     `json:"edited_at,omitempty"`
	TombstonedAt *time.Time     `json:"tombstoned_at,omitempty"`
}

type Artifact struct {
	ID           string       `json:"id"`
	SHA256       string       `json:"sha256"`
	SizeBytes    int64        `json:"size_bytes"`
	Kind         ArtifactKind `json:"kind"`
	MimeType     *string      `json:"mime_type,omitempty"`
	UploadedBy   *string      `json:"uploaded_by,omitempty"`
	CreatedAt    time.Time    `json:"created_at"`
	TombstonedAt *time.Time   `json:"tombstoned_at,omitempty"`
}

type MessageEditView struct {
	ID        int64     `json:"id"`
	MessageID string    `json:"message_id"`
	EditorID  string    `json:"editor_id"`
	EditedAt  time.Time `json:"edited_at"`
	// BodyBefore and BodyAfter are sent only with include_edits=true.
	BodyBefore *string `json:"body_before,omitempty"`
	BodyAfter  *string `json:"body_after,omitempty"`
}

type Reference struct {
	ID        string    `json:"id"`
	SrcKind   RefSide   `json:"src_kind"`
	SrcID     string    `json:"src_id"`
	DstKind   RefSide   `json:"dst_kind"`
	DstID     string    `json:"dst_id"`
	Relation  string    `json:"relation"`
	CreatedAt time.Time `json:"created_at"`
}

type ThreadTransition struct {
	ID         string      `json:"id"`
	ThreadID   string      `json:"thread_id"`
	FromState  ThreadState `json:"from_state"`
	ToState    ThreadState `json:"to_state"`
	ActorID    string      `json:"actor_id"`
	OccurredAt time.Time   `json:"occurred_at"`
}

type ThreadFsmContext struct {
	State       ThreadState        `json:"state"`
	Transitions []ThreadTransition `json:"transitions"`
}

type AcceptedDecision struct {
	ThreadID   string      `json:"thread_id"`
	State      ThreadState `json:"state"`
	Title      *string     `json:"title,omitempty"`
	ProducedBy string      `json:"produced_by"`
	ProducedAt time.Time   `json:"produced_at"`
	ResultKind *string     `json:"result_kind,omitempty"`
	Status     *string     `json:"status,omitempty"`
	Summary    *string     `json:"summary,omitempty"`
}

type ThreadReview struct {
	ThreadID    string         `json:"thread_id"`
	ReviewerID  string         `json:"reviewer_id"`
	ActorID     *string        `json:"actor_id,omitempty"`
	Decision    ReviewDecision `json:"decision"`
	Note        *string        `json:"note,omitempty"`
	DismissedAt *time.Time     `json:"dismissed_at,omitempty"`
	CreatedAt   time.Time      `json:"created_at"`
	UpdatedAt   time.Time      `json:"updated_at"`
}

type GlossaryTerm struct {
	ID          string    `json:"id"`
	WorkspaceID string    `json:"workspace_id"`
	Term        string    `json:"term"`
	Definition  string    `json:"definition"`
	Aliases     []string  `json:"aliases"`
	CreatedBy   string    `json:"created_by"`
	CreatedAt   time.Time `json:"created_at"`
	UpdatedAt   time.Time `json:"updated_at"`
}

type PackElision struct {
	ElidedMessageCount  int64  `json:"elided_message_count"`
	ElidedTokenEstimate int64  `json:"elided_token_estimate"`
	FirstElidedID       string `json:"first_elided_id"`
	LastElidedID        string `json:"last_elided_id"`
	Summary             string `json:"summary"`
}

type ParentGrounding struct {
	ThreadID       string          `json:"thread_id"`
	State          ThreadState     `json:"state"`
	Title          *string         `json:"title,omitempty"`
	OpeningMessage *Message        `json:"opening_message,omitempty"`
	LatestResult   json.RawMessage `json:"latest_result,omitempty"`
}

// ThreadContext is GET /threads/{id}/context: the context pack a claimer reads.
type ThreadContext struct {
	WorkspaceID       string             `json:"workspace_id"`
	ChannelID         string             `json:"channel_id"`
	Thread            Thread             `json:"thread"`
	Messages          []Message          `json:"messages"`
	MessageEdits      []MessageEditView  `json:"message_edits"`
	References        []Reference        `json:"references"`
	Artifacts         []Artifact         `json:"artifacts"`
	Fsm               ThreadFsmContext   `json:"fsm"`
	AcceptedDecisions []AcceptedDecision `json:"accepted_decisions,omitempty"`
	ChangeRequests    []ThreadReview     `json:"change_requests,omitempty"`
	Glossary          []GlossaryTerm     `json:"glossary,omitempty"`
	Elision           *PackElision       `json:"elision,omitempty"`
	ParentGrounding   *ParentGrounding   `json:"parent_grounding,omitempty"`
	NextMessageCursor *string            `json:"next_message_cursor,omitempty"`
}

// StoredEvent is a row of GET /workspaces/{id}/events. Payload is the event
// itself, shaped by Kind.
type StoredEvent struct {
	Type        string          `json:"$type"`
	ID          int64           `json:"id"`
	LSN         int64           `json:"lsn"`
	Kind        string          `json:"kind"`
	WorkspaceID *string         `json:"workspace_id,omitempty"`
	ChannelID   *string         `json:"channel_id,omitempty"`
	ThreadID    *string         `json:"thread_id,omitempty"`
	Payload     json.RawMessage `json:"payload"`
	OccurredAt  time.Time       `json:"occurred_at"`
	PrevHash    string          `json:"prev_hash"`
	ContentHash string          `json:"content_hash"`
	ContentKey  *string         `json:"content_key,omitempty"`
	Traceparent *string         `json:"traceparent,omitempty"`
	Tracestate  *string         `json:"tracestate,omitempty"`
}
