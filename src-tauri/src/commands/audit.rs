use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use crate::actions::log::{ActionLogEntry, ActionsLogRepo};
use crate::actions::undo::{undo_one, UndoMutations};
use crate::auth::token::Provider;
use crate::commands::gmail::AppState;
use crate::providers::gmail::{GmailMutations, GmailMutationsClient};
use crate::providers::graph::GraphMutationsClient;

fn stringify<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub fn list_reversible(
    state: State<'_, AppState>,
    account_id: String,
    limit: u32,
) -> Result<Vec<ActionLogEntry>, String> {
    ActionsLogRepo::new(&state.db)
        .list_reversible(&account_id, limit)
        .map_err(stringify)
}

#[tauri::command]
pub async fn undo_action(
    state: State<'_, AppState>,
    log_id: i64,
    account_id: String,
) -> Result<(), String> {
    let accounts = state.token_store.list_accounts().map_err(stringify)?;
    let account = accounts
        .into_iter()
        .find(|a| a.account_id == account_id)
        .ok_or_else(|| format!("account not found: {account_id}"))?;

    match account.provider {
        Provider::Gmail => {
            let creds = state
                .oauth_config
                .resolve(crate::auth::config::OAuthProvider::Gmail)
                .map_err(stringify)?;
            let token = crate::auth::gmail::ensure_fresh_token(&creds, &state.token_store, &account_id)
                .await
                .map_err(stringify)?;
            let client = Arc::new(GmailMutationsClient::new(state.http.clone(), token.access_token));
            let m: Arc<dyn GmailMutations> = client.clone();
            let u: Arc<dyn UndoMutations> = client;
            undo_one(&*m, &*u, &state.db, log_id).await
        }
        Provider::Graph => {
            let creds = state
                .oauth_config
                .resolve(crate::auth::config::OAuthProvider::Graph)
                .map_err(stringify)?;
            let token = crate::auth::graph::ensure_fresh_token(&creds, &state.token_store, &account_id)
                .await
                .map_err(stringify)?;
            let client = Arc::new(GraphMutationsClient::new(state.http.clone(), token.access_token));
            let m: Arc<dyn GmailMutations> = client.clone();
            let u: Arc<dyn UndoMutations> = client;
            undo_one(&*m, &*u, &state.db, log_id).await
        }
    }
}

#[derive(Serialize)]
pub struct AuditExport {
    pub format: &'static str,
    pub content: String,
    pub rows: usize,
}

#[tauri::command]
pub fn export_audit(
    state: State<'_, AppState>,
    account_id: String,
    format: String,
) -> Result<AuditExport, String> {
    let rows = ActionsLogRepo::new(&state.db)
        .list_recent(&account_id, 100_000)
        .map_err(stringify)?;
    match format.as_str() {
        "json" => {
            let json = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
            Ok(AuditExport {
                format: "json",
                content: json,
                rows: rows.len(),
            })
        }
        "csv" => {
            let mut s = String::new();
            s.push_str("id,executed_at,outcome,action_type,cluster_key,provider_msg_id,reversed_at,error_message\n");
            for r in &rows {
                s.push_str(&format!(
                    "{},{},{},{},{},{},{},{}\n",
                    r.id,
                    csv_escape(&r.executed_at),
                    csv_escape(&r.outcome),
                    csv_escape(&r.action_type),
                    csv_escape(&r.cluster_key),
                    csv_escape(&r.provider_msg_id),
                    csv_escape(r.reversed_at.as_deref().unwrap_or("")),
                    csv_escape(r.error_message.as_deref().unwrap_or("")),
                ));
            }
            Ok(AuditExport {
                format: "csv",
                content: s,
                rows: rows.len(),
            })
        }
        other => Err(format!("unknown format: {other}")),
    }
}

fn csv_escape(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') {
        let escaped = s.replace('"', "\"\"");
        format!("\"{escaped}\"")
    } else {
        s.to_owned()
    }
}
