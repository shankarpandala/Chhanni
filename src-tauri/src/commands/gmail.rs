use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

use crate::auth::gmail::{connect_account, load_credentials};
use crate::auth::keychain::Keychain;
use crate::auth::token::{AccountRecord, TokenStore};

#[derive(Serialize)]
pub struct ConnectAccountResult {
    pub account_id: String,
    pub email: String,
}

/// Application state shared across commands.
#[derive(Clone)]
pub struct AppState {
    pub token_store: TokenStore<Keychain>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("chhanni/", env!("CARGO_PKG_VERSION")))
            .build()?;
        Ok(Self {
            token_store: TokenStore::new(Arc::new(Keychain::new())),
            http,
        })
    }
}

#[tauri::command]
pub async fn gmail_connect_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectAccountResult, String> {
    let creds = load_credentials().map_err(stringify)?;
    let token_store = state.token_store.clone();
    let http = state.http.clone();
    let app_for_open = app.clone();

    let outcome = connect_account(creds, token_store, http, move |url| {
        app_for_open
            .opener()
            .open_url(url.as_str(), None::<&str>)
            .map_err(|e| crate::error::AuthError::OAuth {
                reason: format!("failed to open browser: {e}"),
            })
    })
    .await
    .map_err(stringify)?;

    Ok(ConnectAccountResult {
        account_id: outcome.account_id,
        email: outcome.email,
    })
}

#[tauri::command]
pub fn gmail_list_accounts(
    state: State<'_, AppState>,
) -> Result<Vec<AccountRecord>, String> {
    state.token_store.list_accounts().map_err(stringify)
}

fn stringify<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}
