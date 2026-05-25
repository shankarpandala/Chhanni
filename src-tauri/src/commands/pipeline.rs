use std::sync::Arc;

use tauri::{Emitter, State};

use crate::commands::gmail::AppState;
use crate::db::{ClusterSummary, ClustersRepo, EmbeddingsRepo};
use crate::pipeline::{cluster_account, embed_account, ClusterConfig, EmbedConfig, EmbedProgress};
use crate::sidecar::{Bootstrapper, BootstrapProgress, HttpSidecarClient};

fn stringify<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
pub async fn embed_bootstrap(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let data_dir = data_dir_for_app().map_err(stringify)?;
    let boot = Bootstrapper::new(state.http.clone(), data_dir);
    let app_for_emit = app.clone();
    let sink = crate::sidecar::bootstrap::map_sink(Arc::new(move |p: BootstrapProgress| {
        let _ = app_for_emit.emit("embed:bootstrap", p);
    }));
    let path = boot.ensure_model(sink).await.map_err(stringify)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn embed_run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    sidecar_port: u16,
) -> Result<u64, String> {
    let client = HttpSidecarClient::new(state.http.clone(), sidecar_port);
    let app_for_emit = app.clone();
    let sink: crate::pipeline::embed::EmbedSink = Arc::new(move |p: &EmbedProgress| {
        let _ = app_for_emit.emit("embed:progress", p.clone());
    });
    let n = embed_account(
        Arc::new(client),
        state.db.clone(),
        &account_id,
        EmbedConfig::default(),
        sink,
    )
    .await
    .map_err(stringify)?;
    Ok(n)
}

#[tauri::command]
pub fn cluster_run(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<u64, String> {
    cluster_account(&state.db, &account_id, ClusterConfig::default()).map_err(stringify)
}

#[tauri::command]
pub fn list_clusters(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<ClusterSummary>, String> {
    ClustersRepo::new(&state.db)
        .list_summaries(&account_id)
        .map_err(stringify)
}

#[tauri::command]
pub fn embedding_status(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<EmbeddingStatus, String> {
    let total = state
        .db
        .with_connection(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id = ?1",
                [&account_id],
                |r| r.get(0),
            )
            .map_err(crate::error::DbError::from)
        })
        .map_err(stringify)?;
    let embedded = EmbeddingsRepo::new(&state.db)
        .count_for_account(&account_id)
        .map_err(stringify)?;
    let clusters = ClustersRepo::new(&state.db)
        .count_clusters(&account_id)
        .map_err(stringify)?;
    Ok(EmbeddingStatus {
        total,
        embedded,
        clusters,
    })
}

#[derive(serde::Serialize)]
pub struct EmbeddingStatus {
    pub total: i64,
    pub embedded: i64,
    pub clusters: i64,
}

fn data_dir_for_app() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(d) = std::env::var("CHHANNI_DATA_DIR") {
        return Ok(std::path::PathBuf::from(d));
    }
    let proj = directories::ProjectDirs::from("com", "chhanni", "chhanni")
        .ok_or_else(|| anyhow::anyhow!("could not determine app data dir"))?;
    Ok(proj.data_dir().to_path_buf())
}
