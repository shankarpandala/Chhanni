use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{Emitter, State};
use tauri_plugin_opener::OpenerExt;

use crate::auth::config::{OAuthConfigRepo, OAuthProvider};
use crate::auth::gmail::{connect_account, ensure_fresh_token};
use crate::auth::keychain::Keychain;
use crate::auth::token::{AccountRecord, TokenStore};
use crate::db::{open_with_path, Db, MessagesRepo, SyncStateRepo};
use crate::providers::gmail::GmailClient;
use crate::providers::graph::{run_graph_sync, GraphClient};
use crate::sync::{run_gmail_sync, GmailSyncConfig, SyncProgress};

#[derive(Serialize)]
pub struct ConnectAccountResult {
    pub account_id: String,
    pub email: String,
}

/// Application state shared across commands.
#[derive(Clone)]
pub struct AppState {
    pub token_store: TokenStore<Keychain>,
    pub oauth_config: OAuthConfigRepo<Keychain>,
    pub http: reqwest::Client,
    pub db: Db,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent(concat!("chhanni/", env!("CARGO_PKG_VERSION")))
            .build()?;
        let path = crate::db::connection::default_db_path()?;
        let db = open_with_path(&path)?;
        let keychain = Arc::new(Keychain::new());
        Ok(Self {
            token_store: TokenStore::new(Arc::clone(&keychain)),
            oauth_config: OAuthConfigRepo::new(keychain),
            http,
            db,
        })
    }
}

#[tauri::command]
pub async fn gmail_connect_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectAccountResult, String> {
    let creds = state
        .oauth_config
        .resolve(OAuthProvider::Gmail)
        .map_err(stringify)?;
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

/// Remove every SQLite row belonging to `account_id`. FK CASCADE covers
/// messages/threads/labels/sync_state/embeddings/staged_actions/actions_log;
/// `cluster_classifications` has no FK to `accounts` so we delete it
/// explicitly. Idempotent: a no-op for unknown ids.
fn delete_account_rows(db: &Db, account_id: &str) -> crate::error::DbResult<()> {
    db.with_connection(|conn| {
        let tx = conn.transaction().map_err(crate::error::DbError::Sqlite)?;
        tx.execute(
            "DELETE FROM cluster_classifications WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .map_err(crate::error::DbError::Sqlite)?;
        tx.execute(
            "DELETE FROM accounts WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .map_err(crate::error::DbError::Sqlite)?;
        tx.commit().map_err(crate::error::DbError::Sqlite)?;
        Ok(())
    })
}

/// Remove an account everywhere: keychain index + token plus every SQLite row.
#[tauri::command]
pub fn delete_account(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    delete_account_rows(&state.db, &account_id).map_err(stringify)?;
    state
        .token_store
        .delete_account(&account_id)
        .map_err(stringify)
}

#[tauri::command]
pub async fn graph_connect_account(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<ConnectAccountResult, String> {
    let creds = state
        .oauth_config
        .resolve(OAuthProvider::Graph)
        .map_err(stringify)?;
    let token_store = state.token_store.clone();
    let http = state.http.clone();
    let app_for_open = app.clone();
    let outcome = crate::auth::graph::connect_account(creds, token_store, http, move |url| {
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
pub async fn graph_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    let creds = state
        .oauth_config
        .resolve(OAuthProvider::Graph)
        .map_err(stringify)?;
    let token_store = state.token_store.clone();
    let http = state.http.clone();
    let db = state.db.clone();

    let accounts = token_store.list_accounts().map_err(stringify)?;
    let account = accounts
        .into_iter()
        .find(|a| a.account_id == account_id)
        .ok_or_else(|| format!("account not found: {account_id}"))?;
    mirror_account(&db, &account)?;

    let token = crate::auth::graph::ensure_fresh_token(&creds, &token_store, &account_id)
        .await
        .map_err(stringify)?;
    let client = GraphClient::new(http, token.access_token);

    let app_for_emit = app.clone();
    let sink: crate::providers::graph::sync::ProgressSink =
        Arc::new(move |p: &SyncProgress| {
            let _ = app_for_emit.emit("sync:progress", p.clone());
        });
    run_graph_sync(Arc::new(client), db, &account_id, sink)
        .await
        .map_err(stringify)
}

#[derive(Serialize)]
pub struct AccountSummary {
    pub account_id: String,
    pub email: String,
    pub provider: String,
    pub message_count: i64,
    pub phase: Option<String>,
    pub last_sync_at: Option<String>,
}

#[tauri::command]
pub fn gmail_account_summaries(
    state: State<'_, AppState>,
) -> Result<Vec<AccountSummary>, String> {
    let accounts = state.token_store.list_accounts().map_err(stringify)?;
    let mut out = Vec::with_capacity(accounts.len());
    for a in accounts {
        let count = MessagesRepo::new(&state.db)
            .count_for_account(&a.account_id)
            .map_err(stringify)?;
        let sync = SyncStateRepo::new(&state.db)
            .get(&a.account_id)
            .map_err(stringify)?;
        out.push(AccountSummary {
            account_id: a.account_id.clone(),
            email: a.email,
            provider: match a.provider {
                crate::auth::token::Provider::Gmail => "gmail".to_owned(),
                crate::auth::token::Provider::Graph => "graph".to_owned(),
            },
            message_count: count,
            phase: sync.as_ref().map(|s| match s.phase {
                crate::db::SyncPhase::Initial => "initial".to_owned(),
                crate::db::SyncPhase::Incremental => "incremental".to_owned(),
            }),
            last_sync_at: sync.and_then(|s| s.last_sync_at),
        });
    }
    Ok(out)
}

/// Ensure accounts known to the token store have an `accounts` row in SQLite.
/// Idempotent.
fn mirror_account(db: &Db, account: &AccountRecord) -> Result<(), String> {
    db.with_connection(|c| {
        c.execute(
            "INSERT INTO accounts (account_id, provider, email)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id) DO UPDATE SET email = excluded.email",
            rusqlite::params![
                account.account_id,
                match account.provider {
                    crate::auth::token::Provider::Gmail => "gmail",
                    crate::auth::token::Provider::Graph => "graph",
                },
                account.email,
            ],
        )
        .map(|_| ())
        .map_err(crate::error::DbError::Sqlite)
    })
    .map_err(stringify)
}

#[tauri::command]
pub async fn gmail_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
) -> Result<(), String> {
    let creds = state
        .oauth_config
        .resolve(OAuthProvider::Gmail)
        .map_err(stringify)?;
    let token_store = state.token_store.clone();
    let http = state.http.clone();
    let db = state.db.clone();

    // Mirror the keychain-known account into the accounts table so FKs hold.
    let accounts = token_store.list_accounts().map_err(stringify)?;
    let account = accounts
        .into_iter()
        .find(|a| a.account_id == account_id)
        .ok_or_else(|| format!("account not found: {account_id}"))?;
    mirror_account(&db, &account)?;

    let token = ensure_fresh_token(&creds, &token_store, &account_id)
        .await
        .map_err(stringify)?;
    let client = GmailClient::new(http, token.access_token);

    let app_for_emit = app.clone();
    let sink: crate::sync::gmail::ProgressSink = Arc::new(move |p: &SyncProgress| {
        let _ = app_for_emit.emit("sync:progress", p.clone());
    });

    run_gmail_sync(
        Arc::new(client),
        db,
        &account_id,
        GmailSyncConfig::default(),
        sink,
    )
    .await
    .map_err(stringify)?;

    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_in_memory;

    #[test]
    fn delete_account_rows_cascades_messages_and_classifications() {
        let db = open_in_memory().unwrap();
        db.with_connection(|c| {
            c.execute(
                "INSERT INTO accounts (account_id, provider, email) VALUES ('a1', 'gmail', 'a@b')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO accounts (account_id, provider, email) VALUES ('a2', 'gmail', 'b@b')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO messages (account_id, provider_msg_id, thread_id, sender, sender_email, subject, snippet, internal_date, label_ids, history_id)
                 VALUES ('a1', 'm1', 't1', 'X', 'x@y', 's', 'sn', 0, '[]', 0),
                        ('a2', 'm2', 't2', 'X', 'x@y', 's', 'sn', 0, '[]', 0)",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO cluster_classifications (account_id, cluster_key, category, confidence, model_version, prompt_version, cluster_signature)
                 VALUES ('a1', 'c1', 'newsletter', 0.9, 'm', 'p', 'sig'),
                        ('a2', 'c2', 'newsletter', 0.9, 'm', 'p', 'sig')",
                [],
            )
            .unwrap();
            Ok(())
        })
        .unwrap();

        delete_account_rows(&db, "a1").unwrap();

        db.with_connection(|c| {
            let accounts: i64 = c
                .query_row("SELECT count(*) FROM accounts WHERE account_id = 'a1'", [], |r| r.get(0))
                .unwrap();
            let msgs: i64 = c
                .query_row("SELECT count(*) FROM messages WHERE account_id = 'a1'", [], |r| r.get(0))
                .unwrap();
            let cls: i64 = c
                .query_row(
                    "SELECT count(*) FROM cluster_classifications WHERE account_id = 'a1'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(accounts, 0, "accounts row removed");
            assert_eq!(msgs, 0, "messages cascaded");
            assert_eq!(cls, 0, "cluster_classifications removed");

            // Untouched account survives.
            let other: i64 = c
                .query_row("SELECT count(*) FROM accounts WHERE account_id = 'a2'", [], |r| r.get(0))
                .unwrap();
            assert_eq!(other, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn delete_account_rows_is_idempotent_for_unknown_id() {
        let db = open_in_memory().unwrap();
        delete_account_rows(&db, "ghost").unwrap();
        delete_account_rows(&db, "ghost").unwrap();
    }
}
