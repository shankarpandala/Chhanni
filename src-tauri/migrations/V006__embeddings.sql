-- Per-message embedding. Stored as a BLOB of little-endian f32s. We pin the
-- dimension at write time and refuse to mix dimensions in `embeddings_meta`.
--
-- We deliberately use a plain BLOB rather than the sqlite-vec extension at
-- this stage:
--   * At 5K-20K messages × 768 floats × 4 bytes the index fits in RAM.
--   * Brute-force cosine in Rust takes microseconds.
--   * Adding sqlite-vec means a custom rusqlite build + load_extension dance.
-- sqlite-vec is in BACKLOG as a Phase 8+ optimization.

CREATE TABLE embeddings (
    account_id      TEXT NOT NULL,
    provider_msg_id TEXT NOT NULL,
    model_version   TEXT NOT NULL,
    dim             INTEGER NOT NULL,
    embedding       BLOB NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (account_id, provider_msg_id),
    FOREIGN KEY (account_id, provider_msg_id)
        REFERENCES messages(account_id, provider_msg_id) ON DELETE CASCADE
);

-- Cluster assignments are materialised per-message; we store the cluster's
-- representative key (the lower-cased sender address for the canonical case,
-- or a UUID-like synthetic id for fallback groups) alongside a cosine score
-- to the centroid for transparency.
CREATE TABLE message_clusters (
    account_id      TEXT NOT NULL,
    provider_msg_id TEXT NOT NULL,
    cluster_key     TEXT NOT NULL,
    centroid_cosine REAL,
    assigned_at     TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (account_id, provider_msg_id),
    FOREIGN KEY (account_id, provider_msg_id)
        REFERENCES messages(account_id, provider_msg_id) ON DELETE CASCADE
);

CREATE INDEX idx_message_clusters_key
    ON message_clusters(account_id, cluster_key);
