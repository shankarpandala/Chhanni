#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo
    )
)]

pub mod actions;
pub mod auth;
pub mod commands;
pub mod db;
pub mod error;
pub mod pipeline;
pub mod providers;
pub mod sidecar;
pub mod sync;

use tracing::info;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("chhanni=info,warn"));
    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(fmt::layer().with_target(true).with_level(true))
        .try_init();
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();
    info!(app = "chhanni", version = env!("CARGO_PKG_VERSION"), "starting");

    let state = match commands::gmail::AppState::new() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = %e, "failed to initialise app state");
            return;
        }
    };

    let registry = commands::execute::ExecutorRegistry::default();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .manage(registry)
        .invoke_handler(tauri::generate_handler![
            commands::gmail::gmail_connect_account,
            commands::gmail::gmail_list_accounts,
            commands::gmail::gmail_account_summaries,
            commands::gmail::gmail_sync,
            commands::gmail::graph_connect_account,
            commands::gmail::graph_sync,
            commands::pipeline::embed_bootstrap,
            commands::pipeline::classifier_bootstrap,
            commands::pipeline::embed_run,
            commands::pipeline::cluster_run,
            commands::pipeline::classify_run,
            commands::pipeline::list_clusters,
            commands::pipeline::embedding_status,
            commands::pipeline::classification_summary,
            commands::pipeline::classification_count,
            commands::review::list_review_queue,
            commands::review::stage_action,
            commands::review::unstage_action,
            commands::review::list_staged_actions,
            commands::review::expand_cluster,
            commands::execute::run_executor,
            commands::execute::cancel_executor,
            commands::execute::actions_log_counts,
            commands::execute::actions_log_recent,
            commands::audit::list_reversible,
            commands::audit::undo_action,
            commands::audit::export_audit,
            commands::settings::oauth_status,
            commands::settings::set_oauth_credentials,
            commands::settings::clear_oauth_credentials,
        ]);

    if let Err(err) = builder.run(tauri::generate_context!()) {
        tracing::error!(error = %err, "tauri runtime error");
    }
}

#[cfg(test)]
mod tests {
    use super::init_tracing;

    #[test]
    fn init_tracing_is_idempotent() {
        init_tracing();
        init_tracing();
    }
}
