use std::sync::Arc;

use tauri::{Emitter, State};

use crate::commands::gmail::AppState;
use crate::db::{ClassificationsRepo, ClusterSummary, ClustersRepo, EmbeddingsRepo};
use crate::pipeline::{
    classify_account, cluster_account, embed_account, ClassifyConfig, ClassifyProgress,
    ClusterConfig, EmbedConfig, EmbedProgress,
};
use crate::sidecar::{Bootstrapper, BootstrapProgress, HttpSidecarClient};

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

#[tauri::command]
pub async fn embed_bootstrap(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let data_dir = data_dir_for_app().map_err(|e| format!("{e:#}"))?;
    let boot = Bootstrapper::new(state.http.clone(), data_dir);
    let app_for_emit = app.clone();
    let sink = crate::sidecar::bootstrap::map_sink(Arc::new(move |p: BootstrapProgress| {
        let _ = app_for_emit.emit("embed:bootstrap", p);
    }));
    let path = boot.ensure_embedding_model(sink).await.map_err(stringify)?;
    Ok(path.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn classifier_bootstrap(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let data_dir = data_dir_for_app().map_err(|e| format!("{e:#}"))?;
    let boot = Bootstrapper::new(state.http.clone(), data_dir);
    let app_for_emit = app.clone();
    let sink = crate::sidecar::bootstrap::map_sink(Arc::new(move |p: BootstrapProgress| {
        let _ = app_for_emit.emit("classifier:bootstrap", p);
    }));
    let path = boot.ensure_classifier_model(sink).await.map_err(stringify)?;
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
pub async fn classify_run(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    classifier_port: u16,
) -> Result<u64, String> {
    let client = HttpSidecarClient::new(state.http.clone(), classifier_port);
    let app_for_emit = app.clone();
    let sink: crate::pipeline::classify::ClassifySink =
        Arc::new(move |p: &ClassifyProgress| {
            let _ = app_for_emit.emit("classify:progress", p.clone());
        });
    let n = classify_account(
        Arc::new(client),
        state.db.clone(),
        &account_id,
        ClassifyConfig::default(),
        sink,
    )
    .await
    .map_err(stringify)?;
    Ok(n)
}

#[derive(serde::Serialize)]
pub struct CategoryBucket {
    pub category: String,
    pub count: i64,
}

#[tauri::command]
pub fn classification_summary(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<CategoryBucket>, String> {
    state
        .db
        .with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT COALESCE(category, 'unclassified') AS c, COUNT(*) AS n
                     FROM messages WHERE account_id = ?1
                     GROUP BY c ORDER BY n DESC",
                )
                .map_err(crate::error::DbError::from)?;
            let rows = stmt
                .query_map([&account_id], |r| {
                    Ok(CategoryBucket {
                        category: r.get::<_, String>(0)?,
                        count: r.get::<_, i64>(1)?,
                    })
                })
                .map_err(crate::error::DbError::from)?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r.map_err(crate::error::DbError::from)?);
            }
            Ok(out)
        })
        .map_err(stringify)
}

#[tauri::command]
pub fn classification_count(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<i64, String> {
    ClassificationsRepo::new(&state.db)
        .count(&account_id)
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
