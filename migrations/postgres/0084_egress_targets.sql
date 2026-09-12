-- The egress trust boundary (Cluster 378.1): a per-workspace allowlist of the
-- external targets Maidan may deliver a result to.
--
-- Why this table exists: a result's `deliver_to` list is written by an *agent*
-- (anyone holding `thread:transition` on the thread), while Maidan's connector
-- credentials are operator-held and reach many repositories and channels.
-- Routing straight off the agent-supplied list would make Maidan a confused
-- deputy. So delivery is authorized twice — `deliver_to` *selects*, this
-- allowlist *authorizes* — and an unblessed target is skipped with a recorded
-- warning, exactly like an unknown surface.
--
-- **Default empty ⇒ deliver nowhere**, the same fail-safe as the secret-egress
-- broker (Cluster 371): with nothing configured, nothing is trusted.
--
-- The `(surface, selector)` pair is the same pair `maidan_egress_outbox` (0082)
-- persists, with one deliberate difference in grain: a GitHub *delivery* selector
-- is `owner/name#123`, but an operator blesses the **repository**
-- (`owner/name`) — per-issue blessing would mean an operator ticket per PR.
-- `EgressTarget::allowlist_selector()` is the projection between the two.
CREATE TABLE IF NOT EXISTS maidan_egress_targets (
    id UUID PRIMARY KEY,
    workspace_id UUID NOT NULL REFERENCES maidan_workspaces(id) ON DELETE CASCADE,
    surface TEXT NOT NULL, -- slack | github
    selector TEXT NOT NULL, -- a Slack channel id, or a GitHub `owner/name`
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- One blessing per destination per workspace. A re-bless is idempotent rather
-- than a second row, so the list stays the operator's mental model of "what may
-- we post to" with no duplicates to reconcile.
CREATE UNIQUE INDEX IF NOT EXISTS idx_egress_targets_unique
    ON maidan_egress_targets (workspace_id, surface, selector);
