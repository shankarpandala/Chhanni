//! OAuth client credentials stored in the OS keychain. Lets users configure
//! Gmail / Microsoft Graph credentials through the Settings UI rather than
//! editing a `.env` file. Resolution order at runtime:
//!
//!   1. Keychain (set via the Settings UI; per-provider)
//!   2. Compile-time defaults (none today — the developer can wire these in
//!      a future `build.rs` once they register their own production OAuth
//!      clients).
//!   3. Environment variables (developer convenience).

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::keychain::SecretStore;
use crate::auth::oauth::AppCredentials;
use crate::error::{AuthError, AuthResult};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OAuthProvider {
    Gmail,
    Graph,
}

impl OAuthProvider {
    fn keychain_key(self) -> &'static str {
        match self {
            OAuthProvider::Gmail => "oauth::gmail",
            OAuthProvider::Graph => "oauth::graph",
        }
    }

    pub fn id_env_var(self) -> &'static str {
        match self {
            OAuthProvider::Gmail => "GMAIL_CLIENT_ID",
            OAuthProvider::Graph => "GRAPH_CLIENT_ID",
        }
    }

    pub fn secret_env_var(self) -> &'static str {
        match self {
            OAuthProvider::Gmail => "GMAIL_CLIENT_SECRET",
            OAuthProvider::Graph => "GRAPH_CLIENT_SECRET",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredOAuth {
    client_id: String,
    client_secret: Option<String>,
}

/// Repo that reads/writes OAuth credentials in the keychain. Public only via
/// the typed methods — we never log the values.
pub struct OAuthConfigRepo<S: SecretStore> {
    store: Arc<S>,
}

impl<S: SecretStore> Clone for OAuthConfigRepo<S> {
    fn clone(&self) -> Self {
        Self { store: Arc::clone(&self.store) }
    }
}

impl<S: SecretStore> OAuthConfigRepo<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    /// Save credentials to the keychain. `client_secret` empty string is
    /// treated as `None` so the UI doesn't need a separate "unset" affordance.
    pub fn save(
        &self,
        provider: OAuthProvider,
        client_id: &str,
        client_secret: Option<&str>,
    ) -> AuthResult<()> {
        let client_id = client_id.trim();
        if client_id.is_empty() {
            return Err(AuthError::OAuth {
                reason: "client_id must not be empty".to_owned(),
            });
        }
        let payload = StoredOAuth {
            client_id: client_id.to_owned(),
            client_secret: client_secret
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
        };
        let json = serde_json::to_string(&payload).map_err(AuthError::MalformedToken)?;
        self.store.set(provider.keychain_key(), &json)
    }

    pub fn clear(&self, provider: OAuthProvider) -> AuthResult<()> {
        self.store.delete(provider.keychain_key())
    }

    /// `true` when a `client_id` is configured in the keychain. The UI calls
    /// this; the secret value is never sent out.
    pub fn is_configured(&self, provider: OAuthProvider) -> AuthResult<bool> {
        Ok(self.load_raw(provider)?.is_some())
    }

    /// Resolve credentials for a sign-in attempt. Falls back through the
    /// keychain → env vars → MissingEnv error chain.
    pub fn resolve(&self, provider: OAuthProvider) -> AuthResult<AppCredentials> {
        if let Some(stored) = self.load_raw(provider)? {
            return Ok(AppCredentials {
                client_id: stored.client_id,
                client_secret: stored.client_secret,
            });
        }
        if let Ok(id) = std::env::var(provider.id_env_var()) {
            return Ok(AppCredentials {
                client_id: id,
                client_secret: std::env::var(provider.secret_env_var()).ok(),
            });
        }
        Err(AuthError::MissingEnv(provider.id_env_var()))
    }

    fn load_raw(&self, provider: OAuthProvider) -> AuthResult<Option<StoredOAuth>> {
        let Some(raw) = self.store.get(provider.keychain_key())? else {
            return Ok(None);
        };
        serde_json::from_str::<StoredOAuth>(&raw)
            .map(Some)
            .map_err(AuthError::MalformedToken)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::keychain::MemoryStore;

    fn repo() -> OAuthConfigRepo<MemoryStore> {
        OAuthConfigRepo::new(Arc::new(MemoryStore::new()))
    }

    #[test]
    fn save_then_resolve_returns_keychain_value() {
        let r = repo();
        r.save(OAuthProvider::Gmail, "abc.apps.googleusercontent.com", Some("sek"))
            .unwrap();
        let creds = r.resolve(OAuthProvider::Gmail).unwrap();
        assert_eq!(creds.client_id, "abc.apps.googleusercontent.com");
        assert_eq!(creds.client_secret.as_deref(), Some("sek"));
        assert!(r.is_configured(OAuthProvider::Gmail).unwrap());
    }

    #[test]
    fn empty_secret_is_stored_as_none() {
        let r = repo();
        r.save(OAuthProvider::Graph, "abc", Some("  ")).unwrap();
        let creds = r.resolve(OAuthProvider::Graph).unwrap();
        assert!(creds.client_secret.is_none());
    }

    #[test]
    fn empty_client_id_is_rejected() {
        let r = repo();
        let err = r.save(OAuthProvider::Gmail, "  ", None).unwrap_err();
        assert!(matches!(err, AuthError::OAuth { .. }));
    }

    #[test]
    fn clear_makes_resolve_fall_through() {
        let r = repo();
        r.save(OAuthProvider::Gmail, "x", None).unwrap();
        r.clear(OAuthProvider::Gmail).unwrap();
        assert!(!r.is_configured(OAuthProvider::Gmail).unwrap());
        // With no env var either, resolve errors with the typed variant.
        let err = r.resolve(OAuthProvider::Gmail).unwrap_err();
        assert!(matches!(err, AuthError::MissingEnv("GMAIL_CLIENT_ID")));
    }
}
