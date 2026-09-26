CREATE TABLE IF NOT EXISTS google_keep_outbox (
    task_id INTEGER PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    chat_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending', 'sending', 'synced', 'failed')),
    attempts INTEGER NOT NULL DEFAULT 0,
    claimed_at TEXT,
    keep_note_name TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_google_keep_outbox_status
    ON google_keep_outbox (status, task_id);
