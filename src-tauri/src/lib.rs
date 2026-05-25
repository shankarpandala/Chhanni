#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo
    )
)]

pub mod auth;
pub mod commands;
pub mod error;

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

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::gmail::gmail_connect_account,
            commands::gmail::gmail_list_accounts,
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
