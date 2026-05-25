//! Microsoft Graph mutations. Trash = move to `deleteditems`. Archive on
//! Outlook is a destination folder named "Archive" — we resolve its id once
//! per client construction (or use the well-known `archive` mail folder
//! identifier if present). Mark-read flips `isRead` via PATCH.

use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde_json::json;
use tracing::warn;

use crate::actions::undo::UndoMutations;
use crate::error::{SyncError, SyncResult};
use crate::providers::gmail::GmailMutations;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";
const RETRY_MAX: u32 = 5;

#[async_trait]
pub trait GraphMutations: Send + Sync {
    async fn batch_move(&self, ids: &[String], dest_folder: &str) -> SyncResult<()>;
    async fn batch_set_read(&self, ids: &[String], read: bool) -> SyncResult<()>;
}

#[derive(Clone)]
pub struct GraphMutationsClient {
    http: reqwest::Client,
    access_token: String,
    base: String,
}

impl GraphMutationsClient {
    pub fn new(http: reqwest::Client, access_token: String) -> Self {
        Self {
            http,
            access_token,
            base: GRAPH_BASE.to_owned(),
        }
    }

    pub fn with_base(http: reqwest::Client, access_token: String, base: String) -> Self {
        Self { http, access_token, base }
    }

    async fn send_with_retry(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> SyncResult<()> {
        let url = format!("{}{}", self.base, path);
        let mut delay = Duration::from_secs(1);
        for attempt in 1..=RETRY_MAX {
            let mut req = self.http.request(method.clone(), &url).bearer_auth(&self.access_token);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        return Ok(());
                    }
                    let body_text = r.text().await.unwrap_or_default();
                    if is_retryable(status) && attempt < RETRY_MAX {
                        warn!(attempt, status = status.as_u16(), "graph mutation retryable");
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(32));
                        continue;
                    }
                    return Err(SyncError::GmailStatus { status: status.as_u16(), body: body_text });
                }
                Err(e) if attempt < RETRY_MAX => {
                    warn!(attempt, error = %e, "graph mutation network");
                    tokio::time::sleep(delay).await;
                    delay = (delay * 2).min(Duration::from_secs(32));
                }
                Err(e) => return Err(SyncError::Gmail(e)),
            }
        }
        Err(SyncError::GmailStatus { status: 0, body: "retry exhausted".into() })
    }
}

fn is_retryable(status: StatusCode) -> bool {
    status.as_u16() == 429 || status.is_server_error()
}

/// Graph's `$batch` endpoint caps at 20 sub-requests. We fan a list of N ids
/// into ceil(N/20) sequential $batch posts to stay within the limit and keep
/// retry semantics per-batch.
const BATCH_CAP: usize = 20;

#[async_trait]
impl GraphMutations for GraphMutationsClient {
    async fn batch_move(&self, ids: &[String], dest_folder: &str) -> SyncResult<()> {
        for chunk in ids.chunks(BATCH_CAP) {
            let requests: Vec<serde_json::Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    json!({
                        "id": (i + 1).to_string(),
                        "method": "POST",
                        "url": format!("/me/messages/{id}/move"),
                        "headers": { "Content-Type": "application/json" },
                        "body": { "destinationId": dest_folder }
                    })
                })
                .collect();
            let body = json!({ "requests": requests });
            self.send_with_retry(reqwest::Method::POST, "/$batch", Some(&body))
                .await?;
        }
        Ok(())
    }

    async fn batch_set_read(&self, ids: &[String], read: bool) -> SyncResult<()> {
        for chunk in ids.chunks(BATCH_CAP) {
            let requests: Vec<serde_json::Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    json!({
                        "id": (i + 1).to_string(),
                        "method": "PATCH",
                        "url": format!("/me/messages/{id}"),
                        "headers": { "Content-Type": "application/json" },
                        "body": { "isRead": read }
                    })
                })
                .collect();
            let body = json!({ "requests": requests });
            self.send_with_retry(reqwest::Method::POST, "/$batch", Some(&body))
                .await?;
        }
        Ok(())
    }
}

/// `GraphMutationsClient` also implements the shared `GmailMutations` trait
/// so the existing executor can dispatch against it without knowing which
/// provider it's hitting. Folder names follow Outlook well-known ids.
#[async_trait]
impl GmailMutations for GraphMutationsClient {
    async fn batch_archive(&self, ids: &[String]) -> SyncResult<()> {
        self.batch_move(ids, "archive").await
    }

    async fn batch_trash(&self, ids: &[String]) -> SyncResult<()> {
        self.batch_move(ids, "deleteditems").await
    }

    async fn batch_add_label(&self, _ids: &[String], _label: &str) -> SyncResult<()> {
        Err(SyncError::GmailStatus {
            status: 0,
            body: "add_label not supported on graph (use categories patch)".into(),
        })
    }
    async fn batch_remove_label(&self, _ids: &[String], _label: &str) -> SyncResult<()> {
        Err(SyncError::GmailStatus {
            status: 0,
            body: "remove_label not supported on graph (use categories patch)".into(),
        })
    }
    async fn batch_mark_read(&self, ids: &[String]) -> SyncResult<()> {
        self.batch_set_read(ids, true).await
    }
    async fn batch_delete(&self, ids: &[String]) -> SyncResult<()> {
        for chunk in ids.chunks(BATCH_CAP) {
            let requests: Vec<serde_json::Value> = chunk
                .iter()
                .enumerate()
                .map(|(i, id)| {
                    json!({
                        "id": (i + 1).to_string(),
                        "method": "DELETE",
                        "url": format!("/me/messages/{id}")
                    })
                })
                .collect();
            let body = json!({ "requests": requests });
            self.send_with_retry(reqwest::Method::POST, "/$batch", Some(&body))
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl UndoMutations for GraphMutationsClient {
    async fn untrash(&self, id: &str) -> SyncResult<()> {
        // Move from `deleteditems` back to `inbox`. Outlook does not record
        // the source folder we trashed from, so `inbox` is the safe default.
        let path = format!("/me/messages/{id}/move");
        let body = json!({ "destinationId": "inbox" });
        self.send_with_retry(reqwest::Method::POST, &path, Some(&body))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_matches_429_and_5xx() {
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable(StatusCode::OK));
        assert!(!is_retryable(StatusCode::NOT_FOUND));
    }
}
