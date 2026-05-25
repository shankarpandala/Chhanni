-- Staged (= queued, not yet executed) actions. A row per (account_id,
-- cluster_key, message_id, action_type). message_id is nullable so a single
-- row can represent a cluster-wide bulk action; concrete per-message rows
-- materialise on execution.
--
-- payload is a JSON blob with action-specific args (e.g. labels to add/remove
-- for label changes). Kept as TEXT JSON rather than per-action columns
-- because the action set is going to grow.

CREATE TABLE staged_actions (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    account_id       TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    cluster_key      TEXT NOT NULL,
    -- Null = "applies to every member of the cluster as of stage_time".
    provider_msg_id  TEXT,
    action_type      TEXT NOT NULL CHECK (action_type IN (
        'archive', 'trash', 'add_label', 'remove_label', 'mark_read', 'unsubscribe'
    )),
    payload          TEXT NOT NULL DEFAULT '{}',
    proposed_reason  TEXT,            -- rule-engine rationale for the UI
    staged_at        TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Two partial unique indexes because SQLite treats NULL as distinct in plain
-- UNIQUE constraints, so a 4-col UNIQUE would never dedupe cluster-level rows.
CREATE UNIQUE INDEX idx_staged_actions_uniq_cluster
    ON staged_actions(account_id, cluster_key, action_type)
    WHERE provider_msg_id IS NULL;

CREATE UNIQUE INDEX idx_staged_actions_uniq_message
    ON staged_actions(account_id, cluster_key, action_type, provider_msg_id)
    WHERE provider_msg_id IS NOT NULL;

CREATE INDEX idx_staged_actions_account
    ON staged_actions(account_id, action_type);
