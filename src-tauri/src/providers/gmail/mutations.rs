use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::Serialize;
use tracing::warn;

use crate::error::{SyncError, SyncResult};

const API_BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me";
const BATCH_RETRY_MAX: u32 = 5;
const SYSTEM_LABEL_INBOX: &str = "INBOX";
const SYSTEM_LABEL_TRASH: &str = "TRASH";

#[derive(Serialize)]
struct BatchModifyBody<'a> {
    ids: &'a [String],
    #[serde(skip_serializing_if = "Option::is_none", rename = "addLabelIds")]
    add_label_ids: Option<&'a [&'a str]>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "removeLabelIds")]
    remove_label_ids: Option<&'a [&'a str]>,
}

#[derive(Serialize)]
struct BatchDeleteBody<'a> {
    ids: &'a [String],
}

/// Mutation surface for Gmail. Defined as a trait so the action executor can
/// be unit-tested with a `FakeMutator`.
#[async_trait]
pub trait GmailMutations: Send + Sync {
    async fn batch_archive(&self, ids: &[String]) -> SyncResult<()>;
    async fn batch_trash(&self, ids: &[String]) -> SyncResult<()>;
    async fn batch_add_label(&self, ids: &[String], label: &str) -> SyncResult<()>;
    async fn batch_remove_label(&self, ids: &[String], label: &str) -> SyncResult<()>;
    async fn batch_mark_read(&self, ids: &[String]) -> SyncResult<()>;
    /// Permanently delete. Reserved for the future; not used by the executor.
    async fn batch_delete(&self, ids: &[String]) -> SyncResult<()>;
}

#[derive(Clone)]
pub struct GmailMutationsClient {
    http: reqwest::Client,
    access_token: String,
    base: String,
}

impl GmailMutationsClient {
    pub fn new(http: reqwest::Client, access_token: String) -> Self {
        Self {
            http,
            access_token,
            base: API_BASE.to_owned(),
        }
    }

    pub fn with_base(http: reqwest::Client, access_token: String, base: String) -> Self {
        Self { http, access_token, base }
    }

    async fn post_with_backoff(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> SyncResult<()> {
        let url = format!("{}{}", self.base, path);
        let mut delay = Duration::from_secs(1);
        for attempt in 1..=BATCH_RETRY_MAX {
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.access_token)
                .json(body)
                .send()
                .await;
            match resp {
                Ok(r) => {
                    let status = r.status();
                    if status.is_success() {
                        return Ok(());
                    }
                    let body_text = r.text().await.unwrap_or_default();
                    if is_retryable(status) && attempt < BATCH_RETRY_MAX {
                        warn!(attempt, status = status.as_u16(), "gmail mutation retryable");
                        tokio::time::sleep(delay).await;
                        delay = (delay * 2).min(Duration::from_secs(32));
                        continue;
                    }
                    return Err(SyncError::GmailStatus {
                        status: status.as_u16(),
                        body: body_text,
                    });
                }
                Err(e) if attempt < BATCH_RETRY_MAX => {
                    warn!(attempt, error = %e, "gmail mutation network error");
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

#[async_trait]
impl GmailMutations for GmailMutationsClient {
    async fn batch_archive(&self, ids: &[String]) -> SyncResult<()> {
        // Gmail "archive" = remove INBOX label.
        let body = serde_json::to_value(&BatchModifyBody {
            ids,
            add_label_ids: None,
            remove_label_ids: Some(&[SYSTEM_LABEL_INBOX]),
        })
        .map_err(SyncError::GmailMalformed)?;
        self.post_with_backoff("/messages/batchModify", &body).await
    }

    async fn batch_trash(&self, ids: &[String]) -> SyncResult<()> {
        let body = serde_json::to_value(&BatchModifyBody {
            ids,
            add_label_ids: Some(&[SYSTEM_LABEL_TRASH]),
            remove_label_ids: Some(&[SYSTEM_LABEL_INBOX]),
        })
        .map_err(SyncError::GmailMalformed)?;
        self.post_with_backoff("/messages/batchModify", &body).await
    }

    async fn batch_add_label(&self, ids: &[String], label: &str) -> SyncResult<()> {
        let body = serde_json::to_value(&BatchModifyBody {
            ids,
            add_label_ids: Some(&[label]),
            remove_label_ids: None,
        })
        .map_err(SyncError::GmailMalformed)?;
        self.post_with_backoff("/messages/batchModify", &body).await
    }

    async fn batch_remove_label(&self, ids: &[String], label: &str) -> SyncResult<()> {
        let body = serde_json::to_value(&BatchModifyBody {
            ids,
            add_label_ids: None,
            remove_label_ids: Some(&[label]),
        })
        .map_err(SyncError::GmailMalformed)?;
        self.post_with_backoff("/messages/batchModify", &body).await
    }

    async fn batch_mark_read(&self, ids: &[String]) -> SyncResult<()> {
        self.batch_remove_label(ids, "UNREAD").await
    }

    async fn batch_delete(&self, ids: &[String]) -> SyncResult<()> {
        let body = serde_json::to_value(BatchDeleteBody { ids })
            .map_err(SyncError::GmailMalformed)?;
        self.post_with_backoff("/messages/batchDelete", &body).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retryable_matches_429_and_5xx() {
        assert!(is_retryable(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(!is_retryable(StatusCode::OK));
        assert!(!is_retryable(StatusCode::BAD_REQUEST));
    }
}
