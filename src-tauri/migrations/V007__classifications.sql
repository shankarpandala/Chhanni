-- One row per cluster_key per (model_version, prompt_version). Re-running
-- classification is a no-op when the cluster signature + model_version +
-- prompt_version are unchanged (enforced in code, not SQL).
CREATE TABLE cluster_classifications (
    account_id      TEXT NOT NULL,
    cluster_key     TEXT NOT NULL,
    category        TEXT NOT NULL,
    confidence      REAL NOT NULL,
    reason          TEXT,
    model_version   TEXT NOT NULL,
    prompt_version  TEXT NOT NULL,
    cluster_signature TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (account_id, cluster_key)
);

CREATE INDEX idx_cluster_classifications_account
    ON cluster_classifications(account_id, category);

-- Materialised per-message category, mirrored from cluster_classifications
-- via a trigger on insert/update. Done as a column on `messages` rather than
-- a join because the review queue filters and sorts on category constantly.
ALTER TABLE messages ADD COLUMN category TEXT;
ALTER TABLE messages ADD COLUMN classified_confidence REAL;
