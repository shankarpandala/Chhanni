use serde::{Deserialize, Serialize};
use tauri::State;

use crate::auth::config::OAuthProvider;
use crate::commands::gmail::AppState;

fn stringify<E: std::error::Error>(e: E) -> String {
    let mut out = e.to_string();
    let mut src = e.source();
    while let Some(err) = src {
        out.push_str(": ");
        out.push_str(&err.to_string());
        src = err.source();
    }
    out
}

#[derive(Serialize)]
pub struct OAuthStatus {
    pub gmail_configured: bool,
    pub graph_configured: bool,
}

#[tauri::command]
pub fn oauth_status(state: State<'_, AppState>) -> Result<OAuthStatus, String> {
    Ok(OAuthStatus {
        gmail_configured: state
            .oauth_config
            .is_configured(OAuthProvider::Gmail)
            .map_err(stringify)?,
        graph_configured: state
            .oauth_config
            .is_configured(OAuthProvider::Graph)
            .map_err(stringify)?,
    })
}

#[derive(Deserialize)]
pub struct SetOAuthInput {
    pub provider: String, // "gmail" | "graph"
    pub client_id: String,
    pub client_secret: Option<String>,
}

#[tauri::command]
pub fn set_oauth_credentials(
    state: State<'_, AppState>,
    input: SetOAuthInput,
) -> Result<(), String> {
    let provider = parse_provider(&input.provider)?;
    state
        .oauth_config
        .save(provider, &input.client_id, input.client_secret.as_deref())
        .map_err(stringify)
}

#[tauri::command]
pub fn clear_oauth_credentials(
    state: State<'_, AppState>,
    provider: String,
) -> Result<(), String> {
    let p = parse_provider(&provider)?;
    state.oauth_config.clear(p).map_err(stringify)
}

fn parse_provider(s: &str) -> Result<OAuthProvider, String> {
    match s {
        "gmail" => Ok(OAuthProvider::Gmail),
        "graph" => Ok(OAuthProvider::Graph),
        other => Err(format!("unknown provider: {other}")),
    }
}
