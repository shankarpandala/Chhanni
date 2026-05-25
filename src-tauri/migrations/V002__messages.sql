-- Per-message metadata. NEVER stores the email body. Subject and snippet are
-- stored as plain text in the local SQLite file (filesystem-permission only;
-- SQLCipher in BACKLOG). They are required for clustering + classification
-- and never leave the device.
CREATE TABLE messages (
    -- Composite key: (account_id, provider_msg_id). Provider IDs are unique
    -- per mailbox, not globally, so we always key on the pair.
    account_id        TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    provider_msg_id   TEXT NOT NULL,
    thread_id         TEXT NOT NULL,
    sender            TEXT,                 -- "Name <addr>" raw value
    sender_email      TEXT,                 -- parsed address only, used for clustering
    subject           TEXT,
    snippet           TEXT,
    internal_date     INTEGER NOT NULL,     -- unix epoch ms (provider-reported)
    label_ids         TEXT NOT NULL DEFAULT '[]',  -- JSON array
    history_id        INTEGER,              -- gmail history cursor for this row
    embedded_at       TEXT,                 -- ISO8601 once pipeline runs
    classified_at     TEXT,
    PRIMARY KEY (account_id, provider_msg_id)
);

CREATE INDEX idx_messages_account_thread ON messages(account_id, thread_id);
CREATE INDEX idx_messages_account_sender ON messages(account_id, sender_email);
CREATE INDEX idx_messages_account_date   ON messages(account_id, internal_date DESC);
