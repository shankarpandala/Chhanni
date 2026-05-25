-- Outcome of every executed action. One row per (staged_action, message)
-- so a cluster-level archive that fanned out to 1,000 messages produces
-- 1,000 rows, each independently re-tryable.
CREATE TABLE actions_log (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id        TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    staged_action_id  INTEGER REFERENCES staged_actions(id) ON DELETE SET NULL,
    cluster_key       TEXT NOT NULL,
    provider_msg_id   TEXT NOT NULL,
    action_type       TEXT NOT NULL,
    outcome           TEXT NOT NULL CHECK (outcome IN ('success', 'failure', 'cancelled', 'skipped')),
    error_message     TEXT,
    executed_at       TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_actions_log_account ON actions_log(account_id, executed_at DESC);
CREATE INDEX idx_actions_log_outcome ON actions_log(account_id, outcome);
