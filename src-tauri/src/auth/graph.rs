//! Microsoft Graph OAuth + token management. Mirrors `auth::gmail` against
//! the Microsoft identity platform.

use std::sync::Arc;
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

/// Required scopes for the read-write surface.
pub const GRAPH_SCOPE_MAIL_RW: &str = "Mail.ReadWrite";
pub const GRAPH_SCOPE_OFFLINE: &str = "offline_access";

/// `common` allows both personal MSA and work/school AAD accounts to sign in.
const AUTH_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const TOKEN_URL: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const USERINFO_URL: &str = "https://graph.microsoft.com/v1.0/me";
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(300);

pub fn config() -> ProviderConfig {
    ProviderConfig {
        auth_url: AUTH_URL.to_owned(),
        token_url: TOKEN_URL.to_owned(),
        scopes: vec![
            GRAPH_SCOPE_MAIL_RW.to_owned(),
            GRAPH_SCOPE_OFFLINE.to_owned(),
            "User.Read".to_owned(),
            "openid".to_owned(),
            "email".to_owned(),
            "profile".to_owned(),
        ],
        extra_auth_params: vec![("prompt", "select_account")],
    }
}

pub fn load_credentials() -> AuthResult<AppCredentials> {
    load_credentials_from(|k| std::env::var(k).ok())
}

pub fn load_credentials_from<F>(get: F) -> AuthResult<AppCredentials>
where
    F: Fn(&str) -> Option<String>,
{
    let client_id = get("GRAPH_CLIENT_ID").ok_or(AuthError::MissingEnv("GRAPH_CLIENT_ID"))?;
    // Microsoft's Desktop app registrations are public clients — no secret.
    let client_secret = get("GRAPH_CLIENT_SECRET");
    Ok(AppCredentials {
        client_id,
        client_secret,
    })
}

#[derive(Debug, Deserialize)]
struct UserInfo {
    #[serde(rename = "userPrincipalName")]
    user_principal_name: Option<String>,
    mail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ConnectOutcome {
    pub account_id: String,
    pub email: String,
}

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

    info!(redirect_port = server.port(), "awaiting graph oauth callback");
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
        provider: Provider::Graph,
        email: email.clone(),
    })?;
    info!(account_id = %account_id, "connected microsoft graph account");

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
            reason: format!("graph /me http {}", resp.status()),
        });
    }
    let info: UserInfo = resp.json().await.map_err(AuthError::Refresh)?;
    // Microsoft work accounts often return `mail = null` and the address
    // lives in `userPrincipalName`. Prefer `mail` when present.
    info.mail
        .or(info.user_principal_name)
        .ok_or_else(|| AuthError::OAuth {
            reason: "graph /me returned no email".to_owned(),
        })
}

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
    let client = build_client(creds, &cfg, "http://127.0.0.1:0/callback")?;
    let refreshed = retry_refresh(&client, &token, &cfg.scopes).await?;
    token_store.save_token(account_id, &refreshed)?;
    info!(account_id, "refreshed graph access token");
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
                warn!(attempt, error = %e, "graph token refresh failed; will retry");
                tokio::time::sleep(delay).await;
                delay *= 2;
            }
        }
    }
    unreachable!("retry loop exhausted without returning")
}

// Silence unused warnings if/when sub-features get wired in.
#[allow(dead_code)]
fn _force_arc_import() {
    let _: Arc<u8> = Arc::new(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requests_offline_access_and_mail_rw() {
        let cfg = config();
        assert!(cfg.scopes.contains(&GRAPH_SCOPE_MAIL_RW.to_owned()));
        assert!(cfg.scopes.contains(&GRAPH_SCOPE_OFFLINE.to_owned()));
    }

    #[test]
    fn load_credentials_errors_when_id_missing() {
        let err = load_credentials_from(|_| None).unwrap_err();
        assert!(matches!(err, AuthError::MissingEnv("GRAPH_CLIENT_ID")));
    }
}
