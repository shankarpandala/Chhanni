//! First-run acquisition of the sidecar binary + the embedding model.
//!
//! This module is *not* wired up in `lib.rs` initialisation — startup
//! deliberately does NOT block on a 2.5 GB model download. The frontend
//! triggers it on-demand via the `embed_bootstrap` Tauri command, and we
//! report progress through the event bus.

use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;

use crate::error::SidecarResult;
use crate::sidecar::download::{download_resumable, DownloadSink, DownloadSpec};
use crate::sidecar::release::{
    asset_for, binary_filename, classifier_model, embedding_model, install_root_in, HostPlatform,
};

#[derive(Clone, Debug, Serialize)]
pub struct BootstrapPaths {
    pub binary: PathBuf,
    pub model: PathBuf,
}

pub struct Bootstrapper {
    pub http: reqwest::Client,
    pub data_dir: PathBuf,
}

impl Bootstrapper {
    pub fn new(http: reqwest::Client, data_dir: PathBuf) -> Self {
        Self { http, data_dir }
    }

    pub fn paths(&self) -> Option<BootstrapPaths> {
        let platform = HostPlatform::detect();
        let asset = asset_for(platform)?;
        let install_root = install_root_in(&self.data_dir);
        let binary = install_root.join(binary_filename(platform));
        // The archive layout puts the binary under a versioned subfolder; we
        // also keep the archive-relative path so we can extract there.
        let _ = asset;
        let model = self.data_dir.join("models").join(embedding_model().local_filename);
        Some(BootstrapPaths { binary, model })
    }

    /// Download the embedding model. Idempotent (returns immediately if the
    /// destination already matches the expected checksum).
    pub async fn ensure_embedding_model(&self, sink: DownloadSink) -> SidecarResult<PathBuf> {
        let asset = embedding_model();
        self.ensure_model_asset(asset, sink).await
    }

    /// Download the classifier model. Same semantics.
    pub async fn ensure_classifier_model(&self, sink: DownloadSink) -> SidecarResult<PathBuf> {
        let asset = classifier_model();
        self.ensure_model_asset(asset, sink).await
    }

    async fn ensure_model_asset(
        &self,
        asset: crate::sidecar::release::ModelAsset,
        sink: DownloadSink,
    ) -> SidecarResult<PathBuf> {
        let dest = self.data_dir.join("models").join(&asset.local_filename);
        let spec = DownloadSpec {
            url: asset.url,
            destination: dest.clone(),
            expected_sha256: asset.sha256,
        };
        download_resumable(&self.http, &spec, sink).await?;
        Ok(dest)
    }
}

/// Type-erased progress payload combining model + binary downloads. Frontend
/// renders a single bar by mapping URL → label.
#[derive(Clone, Debug, Serialize)]
pub struct BootstrapProgress {
    pub kind: &'static str,
    pub bytes_downloaded: u64,
    pub bytes_total: Option<u64>,
}

pub fn map_sink(emit: Arc<dyn Fn(BootstrapProgress) + Send + Sync>) -> DownloadSink {
    Arc::new(move |p| {
        let kind = if p.url.contains("llama") { "binary" } else { "model" };
        emit(BootstrapProgress {
            kind,
            bytes_downloaded: p.bytes_downloaded,
            bytes_total: p.bytes_total,
        });
    })
}
