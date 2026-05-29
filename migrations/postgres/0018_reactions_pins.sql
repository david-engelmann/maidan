-- Emoji reactions and thread pins (Cluster 41.0).

CREATE TABLE maidan_reactions (
    message_id UUID NOT NULL REFERENCES maidan_messages (id) ON DELETE CASCADE,
    member_id  UUID NOT NULL REFERENCES maidan_members (id) ON DELETE CASCADE,
    emoji      TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (message_id, member_id, emoji)
);

CREATE INDEX idx_reactions_message ON maidan_reactions (message_id);

CREATE TABLE maidan_pins (
    thread_id  UUID NOT NULL REFERENCES maidan_threads (id) ON DELETE CASCADE,
    message_id UUID NOT NULL REFERENCES maidan_messages (id) ON DELETE CASCADE,
    member_id  UUID NOT NULL REFERENCES maidan_members (id) ON DELETE RESTRICT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (thread_id, message_id)
);

CREATE INDEX idx_pins_thread ON maidan_pins (thread_id);
