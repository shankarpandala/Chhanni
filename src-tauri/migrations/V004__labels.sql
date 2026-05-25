-- Provider labels (Gmail) / folders (Graph). Cached locally for the UI.
CREATE TABLE labels (
    account_id           TEXT NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    provider_label_id    TEXT NOT NULL,
    name                 TEXT NOT NULL,
    label_type           TEXT NOT NULL,    -- 'system' | 'user'
    PRIMARY KEY (account_id, provider_label_id)
);
