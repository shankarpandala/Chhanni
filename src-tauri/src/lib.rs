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

    let builder = tauri::Builder::default().plugin(tauri_plugin_opener::init());

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
