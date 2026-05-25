use std::sync::Arc;

use parking_lot::Mutex;
use tauri::{Emitter, State};

use crate::actions::executor::{
    execute_account, CancellationToken, ExecuteConfig, ExecuteProgress, ExecuteSink,
};
use crate::actions::{ActionLogEntry, ActionsLogRepo, OutcomeCounts};
use crate::auth::token::Provider;
use crate::commands::gmail::AppState;
use crate::providers::gmail::{GmailMutations, GmailMutationsClient};
use crate::providers::graph::GraphMutationsClient;

/// One in-flight executor per (account_id, current run). Stored in `AppState`
/// so a cancel button on the frontend can flip the bool.
#[derive(Clone, Default)]
pub struct ExecutorRegistry {
    inner: Arc<Mutex<std::collections::HashMap<String, CancellationToken>>>,
}

impl ExecutorRegistry {
    pub fn register(&self, account_id: &str) -> CancellationToken {
        let token = CancellationToken::new();
        self.inner.lock().insert(account_id.to_owned(), token.clone());
        token
    }
    pub fn cancel(&self, account_id: &str) -> bool {
        if let Some(t) = self.inner.lock().get(account_id) {
            t.cancel();
            true
        } else {
            false
        }
    }
    pub fn clear(&self, account_id: &str) {
        self.inner.lock().remove(account_id);
    }
}

fn stringify<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn run_executor(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, ExecutorRegistry>,
    account_id: String,
) -> Result<(), String> {
    let accounts = state.token_store.list_accounts().map_err(stringify)?;
    let account = accounts
        .into_iter()
        .find(|a| a.account_id == account_id)
        .ok_or_else(|| format!("account not found: {account_id}"))?;

    let mutator: Arc<dyn GmailMutations> = match account.provider {
        Provider::Gmail => {
            let creds = state
                .oauth_config
                .resolve(crate::auth::config::OAuthProvider::Gmail)
                .map_err(stringify)?;
            let token = crate::auth::gmail::ensure_fresh_token(&creds, &state.token_store, &account_id)
                .await
                .map_err(stringify)?;
            Arc::new(GmailMutationsClient::new(state.http.clone(), token.access_token))
        }
        Provider::Graph => {
            let creds = state
                .oauth_config
                .resolve(crate::auth::config::OAuthProvider::Graph)
                .map_err(stringify)?;
            let token = crate::auth::graph::ensure_fresh_token(&creds, &state.token_store, &account_id)
                .await
                .map_err(stringify)?;
            Arc::new(GraphMutationsClient::new(state.http.clone(), token.access_token))
        }
    };

    let cancel = registry.register(&account_id);

    let app_for_emit = app.clone();
    let sink: ExecuteSink = Arc::new(move |p: &ExecuteProgress| {
        let _ = app_for_emit.emit("execute:progress", p.clone());
    });

    let result = execute_account(
        mutator,
        state.db.clone(),
        &account_id,
        ExecuteConfig::default(),
        cancel,
        sink,
    )
    .await;

    registry.clear(&account_id);
    result.map_err(stringify)
}

#[tauri::command]
pub fn cancel_executor(
    registry: State<'_, ExecutorRegistry>,
    account_id: String,
) -> bool {
    registry.cancel(&account_id)
}

#[tauri::command]
pub fn actions_log_counts(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<OutcomeCounts, String> {
    ActionsLogRepo::new(&state.db)
        .counts(&account_id)
        .map_err(stringify)
}

#[tauri::command]
pub fn actions_log_recent(
    state: State<'_, AppState>,
    account_id: String,
    limit: u32,
) -> Result<Vec<ActionLogEntry>, String> {
    ActionsLogRepo::new(&state.db)
        .list_recent(&account_id, limit)
        .map_err(stringify)
}
