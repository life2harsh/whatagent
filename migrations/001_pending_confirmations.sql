CREATE TABLE IF NOT EXISTS pending_confirmations (
    request_id TEXT PRIMARY KEY NOT NULL,
    chat_id TEXT NOT NULL,
    original_message_id TEXT NOT NULL,
    candidate_json TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pending_confirmations_chat_id
    ON pending_confirmations (chat_id);

CREATE TABLE IF NOT EXISTS processed_messages (
    message_id TEXT PRIMARY KEY NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('received', 'sent_to_n8n', 'completed', 'failed')),
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
