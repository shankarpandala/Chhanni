-- Add reversal payload to actions_log so we can undo an action without
-- another API round-trip. prior_label_ids is the message's label set BEFORE
-- the mutation (captured by the executor); reversal_kind names the inverse
-- operation we'd run on undo.
ALTER TABLE actions_log ADD COLUMN prior_label_ids TEXT;       -- JSON array
ALTER TABLE actions_log ADD COLUMN reversal_kind TEXT;         -- 'restore_labels' | 'untrash' | 'none'
ALTER TABLE actions_log ADD COLUMN reversed_at TEXT;           -- ISO8601 when undone

CREATE INDEX idx_actions_log_reversal
    ON actions_log(account_id, reversal_kind)
    WHERE reversal_kind IS NOT NULL AND reversed_at IS NULL;
