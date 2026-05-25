-- One row per account, holding the cursor used to resume incremental sync.
--
--   phase = 'initial' → still walking messages.list with cursor_token (Gmail
--                       pageToken). Switch to 'incremental' once the initial
--                       walk finishes and we capture a historyId baseline.
--   phase = 'incremental' → use history.list(startHistoryId=last_history_id).
CREATE TABLE sync_state (
    account_id        TEXT PRIMARY KEY NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    phase             TEXT NOT NULL CHECK (phase IN ('initial', 'incremental')),
    cursor_token      TEXT,
    last_history_id   INTEGER,
    last_sync_at      TEXT,
    initial_started_at TEXT,
    initial_completed_at TEXT
);
