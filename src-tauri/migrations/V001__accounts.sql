-- Connected mail accounts. One row per (provider, account_id) where
-- account_id is the opaque UUID minted by auth::token::new_account_id.
-- The actual tokens live in the OS keychain, NOT here.
CREATE TABLE accounts (
    account_id   TEXT PRIMARY KEY NOT NULL,
    provider     TEXT NOT NULL CHECK (provider IN ('gmail', 'graph')),
    email        TEXT NOT NULL,
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);
