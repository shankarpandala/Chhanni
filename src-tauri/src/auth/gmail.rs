use std::time::Duration;

use serde::Deserialize;
use tracing::{info, warn};

use crate::auth::keychain::SecretStore;
use crate::auth::loopback::LoopbackServer;
use crate::auth::oauth::{
    begin_authorization, build_client, exchange_code, refresh_token, AppCredentials,
    ProviderConfig,
};
use crate::auth::token::{new_account_id, AccountRecord, Provider, StoredToken, TokenStore};
use crate::error::{AuthError, AuthResult};

pub const GMAIL_SCOPE_READONLY: &str = "https://www.googleapis.com/auth/gmail.readonly";

const AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const USERINFO_URL: &str = "https://openidconnect.googleapis.com/v1/userinfo";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

pub fn config() -> ProviderConfig {
    ProviderConfig {
        auth_url: AUTH_URL.to_owned(),
        token_url: TOKEN_URL.to_owned(),
        scopes: vec![
            GMAIL_SCOPE_READONLY.to_owned(),
            // Needed to fetch the connected account's email address (userinfo).
            "openid".to_owned(),
            "email".to_owned(),
        ],
        // `access_type=offline` ensures a refresh_token; `prompt=consent`
        // forces Google to re-issue one even if the user has already granted
        // access in the past.
        extra_auth_params: vec![("access_type", "offline"), ("prompt", "consent")],
    }
}

pub fn load_credentials() -> AuthResult<AppCredentials> {
    load_credentials_from(|k| std::env::var(k).ok())
}

pub fn load_credentials_from<F>(get: F) -> AuthResult<AppCredentials>
where
    F: Fn(&str) -> Option<String>,
{
    let client_id = get("GMAIL_CLIENT_ID").ok_or(AuthError::MissingEnv("GMAIL_CLIENT_ID"))?;
    let client_secret = get("GMAIL_CLIENT_SECRET");
    Ok(AppCredentials {
        client_id,
        client_secret,
    })
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    email: String,
}

/// Outcome of `connect_account`. Email is the only PII; safe to surface to the
/// frontend but never log it directly — use the account_id.
#[derive(Debug, Clone)]
pub struct ConnectOutcome {
    pub account_id: String,
    pub email: String,
}

/// Run the full OAuth flow end-to-end. Caller is responsible for opening
/// `on_authorize_url` in the user's browser (typically via
/// `tauri_plugin_opener`).
pub async fn connect_account<S, F>(
    creds: AppCredentials,
    token_store: TokenStore<S>,
    http: reqwest::Client,
    on_authorize_url: F,
) -> AuthResult<ConnectOutcome>
where
    S: SecretStore,
    F: FnOnce(&url::Url) -> AuthResult<()>,
{
    let cfg = config();
    let server = LoopbackServer::bind().await?;
    let redirect_uri = server.redirect_uri();
    let client = build_client(&creds, &cfg, &redirect_uri)?;

    let pending = begin_authorization(&client, &cfg);
    on_authorize_url(&pending.authorize_url)?;

    info!(redirect_port = server.port(), "awaiting oauth callback");
    let callback = server.wait_for_callback(CALLBACK_TIMEOUT).await?;

    let token = exchange_code(
        &client,
        pending.pkce_verifier,
        callback.code,
        &pending.csrf,
        &callback.state,
        &cfg.scopes,
    )
    .await?;

    let email = fetch_user_email(&http, &token.access_token).await?;
    let account_id = new_account_id();

    token_store.save_token(&account_id, &token)?;
    token_store.upsert_account(AccountRecord {
        account_id: account_id.clone(),
        provider: Provider::Gmail,
        email: email.clone(),
    })?;

    info!(account_id = %account_id, "connected gmail account");

    Ok(ConnectOutcome { account_id, email })
}

async fn fetch_user_email(http: &reqwest::Client, access_token: &str) -> AuthResult<String> {
    let resp = http
        .get(USERINFO_URL)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(AuthError::Refresh)?;
    if !resp.status().is_success() {
        return Err(AuthError::OAuth {
            reason: format!("userinfo http {}", resp.status()),
        });
    }
    let info: UserInfo = resp.json().await.map_err(AuthError::Refresh)?;
    Ok(info.email)
}

/// Return a non-expired access token, refreshing via the store-of-record if
/// needed. Persists the refreshed bundle on success.
pub async fn ensure_fresh_token<S: SecretStore>(
    creds: &AppCredentials,
    token_store: &TokenStore<S>,
    account_id: &str,
) -> AuthResult<StoredToken> {
    let token = token_store.get_token(account_id)?;
    if !token.is_expired(time::OffsetDateTime::now_utc()) {
        return Ok(token);
    }

    let cfg = config();
    // The redirect_uri isn't actually used for refresh, but the client
    // builder requires one. Use a fixed loopback placeholder.
    let client = build_client(creds, &cfg, "http://127.0.0.1:0/callback")?;
    let refreshed = retry_refresh(&client, &token, &cfg.scopes).await?;
    token_store.save_token(account_id, &refreshed)?;
    info!(account_id, "refreshed gmail access token");
    Ok(refreshed)
}

async fn retry_refresh(
    client: &oauth2::basic::BasicClient,
    existing: &StoredToken,
    scopes: &[String],
) -> AuthResult<StoredToken> {
    const MAX_ATTEMPTS: u32 = 3;
    let mut delay = Duration::from_millis(250);

    for attempt in 1..=MAX_ATTEMPTS {
        match refresh_token(client, existing, scopes).await {
            Ok(t) => return Ok(t),
            Err(e) if attempt == MAX_ATTEMPTS => return Err(e),
            Err(e) => {
                warn!(attempt, error = %e, "token refresh failed; will retry");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!("retry loop exhausted without returning")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requests_offline_access_and_consent() {
        let cfg = config();
        assert!(cfg
            .extra_auth_params
            .iter()
            .any(|(k, v)| *k == "access_type" && *v == "offline"));
        assert!(cfg
            .extra_auth_params
            .iter()
            .any(|(k, v)| *k == "prompt" && *v == "consent"));
        assert!(cfg.scopes.contains(&GMAIL_SCOPE_READONLY.to_owned()));
    }

    #[test]
    fn load_credentials_errors_when_id_missing() {
        let err = load_credentials_from(|_| None).unwrap_err();
        assert!(matches!(err, AuthError::MissingEnv("GMAIL_CLIENT_ID")));
    }

    #[test]
    fn load_credentials_succeeds_with_id_only() {
        let creds = load_credentials_from(|k| {
            (k == "GMAIL_CLIENT_ID").then(|| "id-value".to_owned())
        })
        .unwrap();
        assert_eq!(creds.client_id, "id-value");
        assert!(creds.client_secret.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ensure_fresh_returns_unexpired_without_network() {
        use crate::auth::keychain::MemoryStore;
        let store = TokenStore::new(std::sync::Arc::new(MemoryStore::new()));
        let id = new_account_id();
        let tok = StoredToken {
            access_token: "at".to_owned(),
            refresh_token: "rt".to_owned(),
            expires_at: time::OffsetDateTime::now_utc() + time::Duration::hours(1),
            scope: GMAIL_SCOPE_READONLY.to_owned(),
        };
        store.save_token(&id, &tok).unwrap();
        let creds = AppCredentials {
            client_id: "x".to_owned(),
            client_secret: None,
        };
        let out = ensure_fresh_token(&creds, &store, &id).await.unwrap();
        assert_eq!(out.access_token, "at");
    }
}
