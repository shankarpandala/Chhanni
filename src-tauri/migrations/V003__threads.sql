-- One row per (account_id, provider_thread_id). Sparse for now; the sync
-- engine fills last_history_id and messages_count on each pass.
CREATE TABLE threads (
    account_id           TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    provider_thread_id   TEXT NOT NULL,
    last_history_id      INTEGER,
    messages_count       INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, provider_thread_id)
);
