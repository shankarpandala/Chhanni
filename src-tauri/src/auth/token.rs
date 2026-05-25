use std::sync::Arc;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::keychain::SecretStore;
use crate::error::{AuthError, AuthResult};

/// Stored OAuth token bundle for a single account. Persisted as JSON in the
/// OS keychain. Treat every field as a secret.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoredToken {
    pub access_token: String,
    pub refresh_token: String,
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    pub scope: String,
}

impl StoredToken {
    /// True when the access token is past its expiry or within the safety
    /// margin (60s).
    pub fn is_expired(&self, now: OffsetDateTime) -> bool {
        self.expires_at <= now + time::Duration::seconds(60)
    }
}

/// Public-facing account metadata. Email is the only PII; it is intentionally
/// excluded from `Debug` to keep it out of accidental log lines.
#[derive(Clone, Serialize, Deserialize)]
pub struct AccountRecord {
    pub account_id: String,
    pub provider: Provider,
    pub email: String,
}

impl std::fmt::Debug for AccountRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRecord")
            .field("account_id", &self.account_id)
            .field("provider", &self.provider)
            .field("email", &"<redacted>")
            .finish()
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Gmail,
    Graph,
}

const INDEX_KEY: &str = "accounts_index";

fn token_key(account_id: &str) -> String {
    format!("account::{account_id}::token")
}

/// Persisted across the keychain:
///   - `accounts_index`         → `Vec<AccountRecord>`
///   - `account::<id>::token`   → `StoredToken`
pub struct TokenStore<S: SecretStore> {
    store: Arc<S>,
}

impl<S: SecretStore> Clone for TokenStore<S> {
    fn clone(&self) -> Self {
        Self {
            store: Arc::clone(&self.store),
        }
    }
}

impl<S: SecretStore> TokenStore<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    pub fn list_accounts(&self) -> AuthResult<Vec<AccountRecord>> {
        let Some(raw) = self.store.get(INDEX_KEY)? else {
            return Ok(Vec::new());
        };
        serde_json::from_str(&raw).map_err(AuthError::MalformedToken)
    }

    pub fn get_token(&self, account_id: &str) -> AuthResult<StoredToken> {
        let raw = self
            .store
            .get(&token_key(account_id))?
            .ok_or_else(|| AuthError::AccountNotFound(account_id.to_owned()))?;
        serde_json::from_str(&raw).map_err(AuthError::MalformedToken)
    }

    pub fn save_token(&self, account_id: &str, token: &StoredToken) -> AuthResult<()> {
        let json = serde_json::to_string(token).map_err(AuthError::MalformedToken)?;
        self.store.set(&token_key(account_id), &json)
    }

    /// Insert or update `account`, returning the resulting full index.
    pub fn upsert_account(&self, account: AccountRecord) -> AuthResult<Vec<AccountRecord>> {
        let mut accounts = self.list_accounts()?;
        if let Some(existing) = accounts.iter_mut().find(|a| a.account_id == account.account_id) {
            *existing = account;
        } else {
            accounts.push(account);
        }
        let json = serde_json::to_string(&accounts).map_err(AuthError::MalformedToken)?;
        self.store.set(INDEX_KEY, &json)?;
        Ok(accounts)
    }

    pub fn delete_account(&self, account_id: &str) -> AuthResult<()> {
        let mut accounts = self.list_accounts()?;
        accounts.retain(|a| a.account_id != account_id);
        let json = serde_json::to_string(&accounts).map_err(AuthError::MalformedToken)?;
        self.store.set(INDEX_KEY, &json)?;
        self.store.delete(&token_key(account_id))?;
        Ok(())
    }
}

/// Generate a fresh opaque account identifier. Not derived from email, so the
/// same address can be re-connected without colliding with stale state.
pub fn new_account_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::keychain::MemoryStore;

    fn store() -> TokenStore<MemoryStore> {
        TokenStore::new(Arc::new(MemoryStore::new()))
    }

    fn sample_token(suffix: &str) -> StoredToken {
        StoredToken {
            access_token: format!("at_{suffix}"),
            refresh_token: format!("rt_{suffix}"),
            expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
            scope: "https://www.googleapis.com/auth/gmail.readonly".to_owned(),
        }
    }

    fn sample_account(id: &str, email: &str) -> AccountRecord {
        AccountRecord {
            account_id: id.to_owned(),
            provider: Provider::Gmail,
            email: email.to_owned(),
        }
    }

    #[test]
    fn empty_store_lists_no_accounts() {
        assert!(store().list_accounts().unwrap().is_empty());
    }

    #[test]
    fn upsert_then_list() {
        let s = store();
        let id = new_account_id();
        s.upsert_account(sample_account(&id, "a@example.com")).unwrap();
        let accounts = s.list_accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].account_id, id);
    }

    #[test]
    fn upsert_is_idempotent_on_same_id() {
        let s = store();
        let id = new_account_id();
        s.upsert_account(sample_account(&id, "a@example.com")).unwrap();
        s.upsert_account(sample_account(&id, "a@example.com")).unwrap();
        assert_eq!(s.list_accounts().unwrap().len(), 1);
    }

    #[test]
    fn save_and_get_token() {
        let s = store();
        let id = new_account_id();
        let token = sample_token("1");
        s.save_token(&id, &token).unwrap();
        let loaded = s.get_token(&id).unwrap();
        assert_eq!(loaded.access_token, token.access_token);
        assert_eq!(loaded.refresh_token, token.refresh_token);
    }

    #[test]
    fn get_token_missing_returns_not_found() {
        let s = store();
        match s.get_token("nope") {
            Err(AuthError::AccountNotFound(id)) => assert_eq!(id, "nope"),
            other => panic!("expected AccountNotFound, got {other:?}"),
        }
    }

    #[test]
    fn delete_account_removes_index_and_token() {
        let s = store();
        let id = new_account_id();
        s.upsert_account(sample_account(&id, "a@example.com")).unwrap();
        s.save_token(&id, &sample_token("1")).unwrap();
        s.delete_account(&id).unwrap();
        assert!(s.list_accounts().unwrap().is_empty());
        assert!(matches!(s.get_token(&id), Err(AuthError::AccountNotFound(_))));
    }

    #[test]
    fn token_expiry_includes_safety_margin() {
        let mut tok = sample_token("e");
        let now = OffsetDateTime::now_utc();
        tok.expires_at = now + time::Duration::seconds(30);
        assert!(tok.is_expired(now));
        tok.expires_at = now + time::Duration::seconds(120);
        assert!(!tok.is_expired(now));
    }

    #[test]
    fn account_debug_redacts_email() {
        let account = sample_account("id-1", "secret@example.com");
        let debug = format!("{account:?}");
        assert!(!debug.contains("secret@example.com"));
        assert!(debug.contains("redacted"));
    }
}
