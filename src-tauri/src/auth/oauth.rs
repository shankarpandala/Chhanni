use std::time::Duration;

use oauth2::basic::{BasicClient, BasicTokenType};
use oauth2::reqwest::async_http_client;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, RefreshToken, Scope, StandardTokenResponse, TokenResponse,
    TokenUrl,
};
use time::OffsetDateTime;
use url::Url;

use crate::auth::token::StoredToken;
use crate::error::{AuthError, AuthResult};

/// Static configuration for an OAuth provider.
#[derive(Clone)]
pub struct ProviderConfig {
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    /// Additional auth-URL query params (e.g. `access_type=offline`,
    /// `prompt=consent` for Google).
    pub extra_auth_params: Vec<(&'static str, &'static str)>,
}

#[derive(Clone)]
pub struct AppCredentials {
    pub client_id: String,
    pub client_secret: Option<String>,
}

impl std::fmt::Debug for AppCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppCredentials")
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// Output of `begin_authorization`. Hold `pkce_verifier` and `csrf` for the
/// duration of the flow; pass them back into `exchange_code`.
pub struct PendingAuthorization {
    pub authorize_url: Url,
    pub csrf: CsrfToken,
    pub pkce_verifier: PkceCodeVerifier,
}

pub fn build_client(
    creds: &AppCredentials,
    config: &ProviderConfig,
    redirect_uri: &str,
) -> AuthResult<BasicClient> {
    let auth_url = AuthUrl::new(config.auth_url.clone())
        .map_err(|e| AuthError::OAuth { reason: format!("auth url: {e}") })?;
    let token_url = TokenUrl::new(config.token_url.clone())
        .map_err(|e| AuthError::OAuth { reason: format!("token url: {e}") })?;
    let redirect = RedirectUrl::new(redirect_uri.to_owned())
        .map_err(|e| AuthError::OAuth { reason: format!("redirect url: {e}") })?;

    let mut client = BasicClient::new(
        ClientId::new(creds.client_id.clone()),
        creds.client_secret.clone().map(ClientSecret::new),
        auth_url,
        Some(token_url),
    )
    .set_redirect_uri(redirect);

    // Force token endpoint to receive client_id as a body param too; some
    // providers (Google) accept either header or body.
    client = client.set_auth_type(oauth2::AuthType::RequestBody);
    Ok(client)
}

pub fn begin_authorization(client: &BasicClient, config: &ProviderConfig) -> PendingAuthorization {
    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();

    let mut req = client.authorize_url(CsrfToken::new_random);
    for scope in &config.scopes {
        req = req.add_scope(Scope::new(scope.clone()));
    }
    for (k, v) in &config.extra_auth_params {
        req = req.add_extra_param(*k, *v);
    }
    let (authorize_url, csrf) = req.set_pkce_challenge(pkce_challenge).url();

    PendingAuthorization {
        authorize_url,
        csrf,
        pkce_verifier,
    }
}

/// Trade an authorization code for tokens. The returned `StoredToken` always
/// contains a refresh token; if the provider did not issue one we surface an
/// error so the caller can prompt re-consent.
pub async fn exchange_code(
    client: &BasicClient,
    pkce_verifier: PkceCodeVerifier,
    code: String,
    expected_state: &CsrfToken,
    received_state: &str,
    scopes: &[String],
) -> AuthResult<StoredToken> {
    if expected_state.secret() != received_state {
        return Err(AuthError::StateMismatch);
    }

    let token = client
        .exchange_code(AuthorizationCode::new(code))
        .set_pkce_verifier(pkce_verifier)
        .request_async(async_http_client)
        .await
        .map_err(|e| AuthError::OAuth { reason: format!("token exchange: {e}") })?;

    into_stored(token, scopes, None)
}

/// Refresh an existing token. The new bundle keeps the old refresh token if
/// the provider doesn't rotate it.
pub async fn refresh_token(
    client: &BasicClient,
    existing: &StoredToken,
    scopes: &[String],
) -> AuthResult<StoredToken> {
    let response = client
        .exchange_refresh_token(&RefreshToken::new(existing.refresh_token.clone()))
        .request_async(async_http_client)
        .await
        .map_err(|e| AuthError::OAuth { reason: format!("refresh: {e}") })?;

    into_stored(response, scopes, Some(existing.refresh_token.clone()))
}

fn into_stored(
    response: StandardTokenResponse<oauth2::EmptyExtraTokenFields, BasicTokenType>,
    scopes: &[String],
    fallback_refresh: Option<String>,
) -> AuthResult<StoredToken> {
    let access_token = response.access_token().secret().clone();
    let refresh_token = response
        .refresh_token()
        .map(|r| r.secret().clone())
        .or(fallback_refresh)
        .ok_or_else(|| AuthError::OAuth {
            reason: "provider did not issue a refresh token".to_owned(),
        })?;
    let expires_in = response
        .expires_in()
        .unwrap_or_else(|| Duration::from_secs(3600));
    let expires_at = OffsetDateTime::now_utc() + time::Duration::seconds(expires_in.as_secs() as i64);
    let scope = response
        .scopes()
        .map(|s| s.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(" "))
        .unwrap_or_else(|| scopes.join(" "));

    Ok(StoredToken {
        access_token,
        refresh_token,
        expires_at,
        scope,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> AppCredentials {
        AppCredentials {
            client_id: "client".to_owned(),
            client_secret: Some("secret".to_owned()),
        }
    }

    fn config() -> ProviderConfig {
        ProviderConfig {
            auth_url: "https://accounts.example.com/auth".to_owned(),
            token_url: "https://accounts.example.com/token".to_owned(),
            scopes: vec!["scope.a".to_owned(), "scope.b".to_owned()],
            extra_auth_params: vec![("access_type", "offline"), ("prompt", "consent")],
        }
    }

    #[test]
    fn begin_authorization_builds_a_url_with_pkce_and_scopes() {
        let client = build_client(&creds(), &config(), "http://127.0.0.1:1234/callback").unwrap();
        let pending = begin_authorization(&client, &config());
        let url = pending.authorize_url.as_str();
        assert!(url.contains("code_challenge="));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("scope=scope.a+scope.b"));
        assert!(url.contains("access_type=offline"));
        assert!(url.contains("prompt=consent"));
        assert!(url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A1234%2Fcallback"));
    }

    #[test]
    fn build_client_rejects_garbage_urls() {
        let bad = ProviderConfig {
            auth_url: "not-a-url".to_owned(),
            ..config()
        };
        let err = build_client(&creds(), &bad, "http://127.0.0.1:1/callback").unwrap_err();
        assert!(matches!(err, AuthError::OAuth { .. }));
    }
}
